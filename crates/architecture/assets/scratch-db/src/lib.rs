//! A test's scratch database, dropped even when the test panics —
//! installed by `harness init-repo`, and the harness's: never edit it here,
//! contest it.
//!
//! A test that creates its database by hand drops it at its end, which a
//! panicking test, a timed-out one or a mutant `cargo mutants` kills never
//! reaches: thousands of databases left on the factory's server. This guard
//! drops its database in `Drop`, which runs while a panic unwinds:
//!
//! ```ignore
//! #[tokio::test]
//! async fn a_moved_token_stays_where_it_was_left() {
//!     let db = platform_scratch_db::ScratchDb::create("move_token_moved")
//!         .await
//!         .expect("DATABASE_URL names the harness's PostgreSQL");
//!     let client = db.connect().await.expect("connect");
//!     // migrate, seed, test — `db.url()` for whatever opens its own pool
//! }
//! ```
//!
//! Each database is named `scratch_<epoch>_<pid>_<seq>_<label>`: when it was
//! created, by which process, the how-many-th, for which test. The
//! harness's watch reads that name and drops a database a killed process
//! left, once it is idle and half an hour old (`scratch_sweep`).
//!
//! A dev-dependency only: `platform-scratch-db = { path =
//! "<to the root>/crates/platform/scratch-db" }`. It reaches PostgreSQL
//! through the engine's own client, and never the product's data.

#![deny(unsafe_code)]
#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use harness_dataguard::tokio_postgres::{self, Client, Config, NoTls};

/// The longest name PostgreSQL keeps (`NAMEDATALEN - 1`).
const MAX_NAME: usize = 63;

/// How long the drop may wait for the server to accept a connection.
const CONNECT_WITHIN: Duration = Duration::from_secs(5);

/// How long the whole drop may take: a server that is up but hung must not
/// hang the test binary, which `cargo mutants` would then kill.
const DROP_WITHIN: Duration = Duration::from_secs(30);

/// The scratch databases this process created so far.
static CREATED: AtomicU32 = AtomicU32::new(0);

/// Why a scratch database could not be created or reached.
#[derive(Debug)]
pub enum Error {
    /// `DATABASE_URL` is not set.
    NoDatabaseUrl,
    /// `DATABASE_URL` is no PostgreSQL URL.
    NotAUrl,
    /// The server refused.
    Postgres(tokio_postgres::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDatabaseUrl => write!(f, "DATABASE_URL is not set"),
            Self::NotAUrl => write!(f, "DATABASE_URL is no postgres:// URL"),
            Self::Postgres(e) => write!(f, "PostgreSQL refused: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Postgres(e) => Some(e),
            Self::NoDatabaseUrl | Self::NotAUrl => None,
        }
    }
}

impl From<tokio_postgres::Error> for Error {
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Postgres(e)
    }
}

/// A database of its own for one test, dropped when the guard is.
pub struct ScratchDb {
    /// Its name, `scratch_<epoch>_<pid>_<seq>_<label>`.
    name: String,
    /// `DATABASE_URL`, naming this database instead.
    url: String,
    /// `DATABASE_URL` as set: where the database is created and dropped.
    server: String,
}

impl ScratchDb {
    /// Creates an empty database on the server `DATABASE_URL` names, as its
    /// role, for the test `label` describes (reduced to `[a-z0-9_]`).
    ///
    /// # Errors
    ///
    /// `DATABASE_URL` is not set or is no URL, or the server refused.
    pub async fn create(label: &str) -> Result<Self, Error> {
        let server = std::env::var("DATABASE_URL").map_err(|_| Error::NoDatabaseUrl)?;
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let seq = CREATED.fetch_add(1, Ordering::Relaxed);
        let name = name_for(epoch, std::process::id(), seq, label);
        let url = naming(&server, &name).ok_or(Error::NotAUrl)?;
        let (client, connection) = tokio_postgres::connect(&server, NoTls).await?;
        let driver = tokio::spawn(connection);
        let created = client
            .batch_execute(&format!("CREATE DATABASE {}", quoted(&name)))
            .await;
        drop(client);
        driver.abort();
        created?;
        Ok(Self { name, url, server })
    }

    /// The database's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `DATABASE_URL`, naming this database: for whatever opens its own
    /// connection or pool.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// A connection to this database, driven on the test's runtime.
    ///
    /// # Errors
    ///
    /// The server refused.
    pub async fn connect(&self) -> Result<Client, Error> {
        let (client, connection) = tokio_postgres::connect(self.url(), NoTls).await?;
        tokio::spawn(connection);
        Ok(client)
    }
}

/// Its name only: both URLs carry `DATABASE_URL`'s password, which a
/// `{db:?}` in an assertion would write into the test's output and the CI's
/// logs.
impl fmt::Debug for ScratchDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScratchDb")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Drop for ScratchDb {
    /// Drops the database, forcing out whoever is still connected. On a
    /// thread and a runtime of its own: a test's runtime may be the one
    /// unwinding, and a runtime cannot be blocked on from inside one.
    /// Never panics — a database it could not drop is the watch's sweep's.
    fn drop(&mut self) {
        let server = self.server.clone();
        let name = self.name.clone();
        let dropper = std::thread::spawn(move || drop_database(&server, &name));
        // A dropper that panicked left the database to the sweep.
        let _ = dropper.join();
    }
}

/// `DROP DATABASE … WITH (FORCE)` on `server`, within [`DROP_WITHIN`];
/// every failure ignored.
fn drop_database(server: &str, name: &str) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let statement = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", quoted(name));
    runtime.block_on(async {
        let _ = tokio::time::timeout(DROP_WITHIN, async {
            let mut config: Config = server.parse().ok()?;
            config.connect_timeout(CONNECT_WITHIN);
            let (client, connection) = config.connect(NoTls).await.ok()?;
            let driver = tokio::spawn(connection);
            let dropped = client.batch_execute(&statement).await;
            drop(client);
            driver.abort();
            dropped.ok()
        })
        .await;
    });
}

/// The name of the `seq`-th scratch database of process `pid`, created at
/// `epoch`, for `label`: `scratch_<epoch>_<pid>_<seq>_<label>`, the label
/// reduced to `[a-z0-9_]` and cut so the whole fits in 63 bytes.
fn name_for(epoch: u64, pid: u32, seq: u32, label: &str) -> String {
    let reduced: String = label
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '_'
            }
        })
        .collect();
    let room = MAX_NAME.saturating_sub(format!("scratch_{epoch}_{pid}_{seq}_").len());
    let mut label: String = reduced.chars().take(room).collect();
    if label.is_empty() {
        label.push_str("db");
    }
    format!("scratch_{epoch}_{pid}_{seq}_{label}")
}

/// `url` naming the database `name` instead of its own, its query kept;
/// `None` for what is no `postgres://` URL.
fn naming(url: &str, name: &str) -> Option<String> {
    let (address, query) = url
        .split_once('?')
        .map_or((url, None), |(a, q)| (a, Some(q)));
    let (scheme, rest) = address.split_once("://")?;
    if !scheme.starts_with("postgres") {
        return None;
    }
    let authority = rest.split('/').next().unwrap_or(rest);
    let query = query.map(|q| format!("?{q}")).unwrap_or_default();
    Some(format!("{scheme}://{authority}/{name}{query}"))
}

/// The name as an SQL identifier: double-quoted, an inner quote doubled.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_fits_63_bytes_whatever_its_label() {
        let long = "a".repeat(200);
        let name = name_for(9_999_999_999, u32::MAX, u32::MAX, &long);
        assert_eq!(name.len(), MAX_NAME, "{name}");
        assert!(name.starts_with("scratch_9999999999_4294967295_4294967295_a"));
    }

    #[test]
    fn a_label_is_reduced_to_the_grammar_the_sweep_reads() {
        assert_eq!(
            name_for(1_729_760_000, 42, 0, "Move-Token moved"),
            "scratch_1729760000_42_0_move_token_moved"
        );
        assert_eq!(
            name_for(1_729_760_000, 42, 3, ""),
            "scratch_1729760000_42_3_db"
        );
        assert_eq!(
            name_for(1_729_760_000, 42, 3, "é"),
            "scratch_1729760000_42_3__"
        );
    }

    #[test]
    fn the_url_names_the_scratch_database_and_keeps_its_query() {
        assert_eq!(
            naming(
                "postgres://app:pw@localhost:5432/forge?sslmode=disable",
                "s"
            )
            .as_deref(),
            Some("postgres://app:pw@localhost:5432/s?sslmode=disable")
        );
        assert_eq!(
            naming("postgresql://h", "s").as_deref(),
            Some("postgresql://h/s")
        );
        assert_eq!(naming("mysql://h/d", "s"), None);
        assert_eq!(naming("not a url", "s"), None);
    }

    #[test]
    fn a_name_is_quoted_and_its_inner_quotes_doubled() {
        assert_eq!(quoted("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn errors_say_what_is_missing_and_keep_the_server_s_own() {
        assert_eq!(Error::NoDatabaseUrl.to_string(), "DATABASE_URL is not set");
        assert!(std::error::Error::source(&Error::NotAUrl).is_none());
        let refused = Error::from("not a url".parse::<Config>().unwrap_err());
        assert!(refused.to_string().starts_with("PostgreSQL refused: "));
        assert!(std::error::Error::source(&refused).is_some());
    }

    /// A guard over a server nothing serves (port 1): its drop fails fast,
    /// and quietly.
    fn unserved() -> ScratchDb {
        ScratchDb {
            name: "scratch_1729760000_42_0_unserved".to_string(),
            url: "postgres://app:s3cret@127.0.0.1:1/scratch_1729760000_42_0_unserved".to_string(),
            server: "postgres://app:s3cret@127.0.0.1:1/forge".to_string(),
        }
    }

    #[test]
    fn a_guard_gives_its_name_and_url_and_drops_quietly_without_a_server() {
        let db = unserved();
        assert_eq!(db.name(), "scratch_1729760000_42_0_unserved");
        assert_eq!(
            db.url(),
            "postgres://app:s3cret@127.0.0.1:1/scratch_1729760000_42_0_unserved"
        );
        drop(db);
    }

    #[test]
    fn a_guard_debugs_its_name_never_the_password() {
        let shown = format!("{:?}", unserved());
        assert!(
            shown.contains("scratch_1729760000_42_0_unserved"),
            "{shown}"
        );
        assert!(!shown.contains("s3cret"), "{shown}");
    }

    /// Where `DATABASE_URL` is set — the product's CI, a factory's session.
    #[tokio::test]
    async fn two_scratches_differ_and_a_panicking_scope_leaves_no_database() {
        let Ok(server) = std::env::var("DATABASE_URL") else {
            return;
        };
        let first = ScratchDb::create("first").await.expect("created");
        let second = ScratchDb::create("first").await.expect("created");
        assert_ne!(first.name(), second.name());
        assert_eq!(
            Some(second.url()),
            naming(&server, second.name()).as_deref()
        );
        let client = second.connect().await.expect("connected");
        client
            .batch_execute("CREATE TABLE t (x int)")
            .await
            .expect("usable");
        let (sent, received) = std::sync::mpsc::channel();
        let panicked = tokio::spawn(async move {
            let db = ScratchDb::create("panics").await.expect("created");
            sent.send(db.name().to_string()).expect("sent");
            panic!("the test fails, its database must not stay");
        })
        .await;
        assert!(panicked.is_err());
        let name = received.recv().expect("a name");
        let second_name = second.name().to_string();
        // Still connected to it: the drop forces the session out.
        drop(second);
        drop(client);
        let (admin, connection) = tokio_postgres::connect(&server, NoTls)
            .await
            .expect("admin");
        tokio::spawn(connection);
        for gone in [name.as_str(), second_name.as_str()] {
            let left = admin
                .query("SELECT 1 FROM pg_database WHERE datname = $1", &[&gone])
                .await
                .expect("listed");
            assert!(left.is_empty(), "{gone} stayed");
        }
        let kept = admin
            .query(
                "SELECT 1 FROM pg_database WHERE datname = $1",
                &[&first.name()],
            )
            .await
            .expect("listed");
        assert_eq!(kept.len(), 1, "a live guard keeps its database");
    }
}
