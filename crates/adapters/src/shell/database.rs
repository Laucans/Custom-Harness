//! The product's database server, from the watch.
//!
//! **Whether it answers**: one TCP connection to the host and port of its
//! `DATABASE_URL`, nothing logged in, nothing read. The watch asks it once a
//! tick: a code session or a repair started while it is down runs its tests
//! without a database — the ones that need it skip in silence locally, then
//! fail in CI (`ForgeDesMondes`, 2026-10-19/20: Docker stopped all night, 7
//! of 20 red `rust` checks).
//!
//! **What its role owns, and the drop of one** ([`PgDatabases`], core's
//! `Databases` port): the watch's sweep of the scratch databases a product's
//! tests left behind. It logs in with the factory's own role, lists only the
//! databases that role owns, and drops them with a plain `DROP DATABASE`,
//! which the server refuses while anyone is connected — the sweep never
//! cuts a test off. The drops of one sweep share one connection and one
//! deadline: a server that hangs holds the watch that long, no more. The
//! harness's only Postgres client; the URL, which carries a password, is
//! never written anywhere.

use std::time::Duration;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::database::{Databases, Dropped, Dropping, OwnedDatabase};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_postgres::{Client, NoTls};

/// How long the connection may take before the database counts as down.
const CONNECT_WITHIN: Duration = Duration::from_secs(2);

/// How long the listing may take.
const LIST_WITHIN: Duration = Duration::from_secs(20);

/// The databases the connected role owns, with the sessions on each.
const OWNED: &str = "\
SELECT d.datname, d.datistemplate, \
       (SELECT count(*) FROM pg_stat_activity a WHERE a.datid = d.oid)::int4 \
  FROM pg_database d \
 WHERE d.datdba = (SELECT oid FROM pg_roles WHERE rolname = current_user)";

/// The `SQLSTATE` of a database someone is connected to (`object_in_use`).
const OBJECT_IN_USE: &str = "55006";

/// The `host:port` a PostgreSQL URL names (`postgres://user:pass@host:5432/db`,
/// `postgresql://host/db` on the default port); `None` for what is no such URL.
#[must_use]
pub fn address_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))?;
    let authority = rest.split(['/', '?']).next()?;
    let host_port = authority.rsplit('@').next()?;
    if host_port.is_empty() {
        return None;
    }
    Some(
        if host_port
            .rsplit_once(':')
            .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
        {
            host_port.to_string()
        } else {
            format!("{host_port}:5432")
        },
    )
}

/// Whether the database `url` names accepts a connection: `None` when the
/// URL names no PostgreSQL address (nothing to check, nothing held).
pub async fn answers(url: &str) -> Option<bool> {
    let address = address_of(url)?;
    Some(matches!(
        tokio::time::timeout(CONNECT_WITHIN, tokio::net::TcpStream::connect(&address)).await,
        Ok(Ok(_))
    ))
}

/// The database a PostgreSQL URL names (`postgres://u:p@h:5432/db?x` →
/// `db`); `None` when it names none, or is no such URL.
#[must_use]
pub fn database_of(url: &str) -> Option<String> {
    address_of(url)?;
    let rest = url.split_once("://")?.1;
    let path = rest.split_once('/')?.1;
    let name = path.split('?').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// The name as an SQL identifier: double-quoted, an inner quote doubled.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// A drop the server refused: its `SQLSTATE`, when it answered one, and
/// its reason.
struct Refusal {
    /// `None` when no server answered: the connection is gone.
    sqlstate: Option<String>,
    /// The reason, in the server's words.
    reason: String,
}

/// What a refused drop means, and whether the pass goes on: someone
/// connected (`Busy`, on), another refusal of the server's (`Refused`, on),
/// or no server behind the connection any more (`Refused`, stop).
fn verdict(refusal: Refusal) -> (Dropped, bool) {
    match refusal.sqlstate.as_deref() {
        Some(OBJECT_IN_USE) => (Dropped::Busy, true),
        Some(_) => (Dropped::Refused(refusal.reason), true),
        None => (Dropped::Refused(refusal.reason), false),
    }
}

/// Drops `names` in order with `drop_one`, until `within` has passed or a
/// refusal ends the pass: one entry per name reached, none for the rest.
async fn drop_each(
    names: &[String],
    within: Duration,
    drop_one: impl AsyncFn(&str) -> Result<(), Refusal>,
) -> Vec<Dropping> {
    let deadline = Instant::now() + within;
    let mut reached = Vec::new();
    for name in names {
        if Instant::now() >= deadline {
            break;
        }
        let (dropped, goes_on) = match tokio::time::timeout_at(deadline, drop_one(name)).await {
            Ok(Ok(())) => (Dropped::Gone, true),
            Ok(Err(refusal)) => verdict(refusal),
            Err(_) => (
                Dropped::Refused("no answer before the sweep's deadline".to_string()),
                false,
            ),
        };
        reached.push(Dropping {
            name: name.clone(),
            dropped,
        });
        if !goes_on {
            break;
        }
    }
    reached
}

/// A refusal in the server's words — never the URL's.
fn refused(what: &str, error: &tokio_postgres::Error) -> Halt {
    Halt::Failed(format!("the database server refused {what}: {error}"))
}

/// The product's database server, logged in as the factory's role.
pub struct PgDatabases {
    url: String,
}

impl PgDatabases {
    /// The server `url` names (the factory's `DATABASE_URL`).
    #[must_use]
    pub const fn new(url: String) -> Self {
        Self { url }
    }

    /// A fresh connection and the task driving it, for `what`.
    async fn connect(
        &self,
        what: &str,
    ) -> Outcome<(Client, JoinHandle<Result<(), tokio_postgres::Error>>)> {
        let mut config: tokio_postgres::Config = self
            .url
            .parse()
            .map_err(|e: tokio_postgres::Error| refused(what, &e))?;
        config.connect_timeout(CONNECT_WITHIN);
        let (client, connection) = config.connect(NoTls).await.map_err(|e| refused(what, &e))?;
        Ok((client, tokio::spawn(connection)))
    }
}

#[async_trait(?Send)]
impl Databases for PgDatabases {
    async fn owned(&self) -> Outcome<Vec<OwnedDatabase>> {
        let (client, driver) = self.connect("the listing").await?;
        let listed = tokio::time::timeout(LIST_WITHIN, client.query(OWNED, &[])).await;
        drop(client);
        driver.abort();
        let rows = listed
            .map_err(|_| {
                Halt::Failed(format!(
                    "the database server did not answer the listing within {}s",
                    LIST_WITHIN.as_secs()
                ))
            })?
            .map_err(|e| refused("the listing", &e))?;
        rows.iter()
            .map(|row| {
                let connections: i32 = row.try_get(2).map_err(|e| refused("the listing", &e))?;
                Ok(OwnedDatabase {
                    name: row.try_get(0).map_err(|e| refused("the listing", &e))?,
                    template: row.try_get(1).map_err(|e| refused("the listing", &e))?,
                    // A count that does not fit reads as busy, never idle.
                    connections: u32::try_from(connections).unwrap_or(u32::MAX),
                })
            })
            .collect()
    }

    async fn drop_idle(&self, names: &[String], within: Duration) -> Outcome<Vec<Dropping>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let (client, driver) = self.connect("a drop").await?;
        let reached = drop_each(names, within, async |name| {
            client
                .batch_execute(&format!("DROP DATABASE IF EXISTS {}", quoted(name)))
                .await
                .map_err(|e| Refusal {
                    sqlstate: e.code().map(|state| state.code().to_string()),
                    reason: refused(&format!("dropping {name}"), &e)
                        .reason()
                        .to_string(),
                })
        })
        .await;
        drop(client);
        driver.abort();
        Ok(reached)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_is_read_off_the_url() {
        assert_eq!(
            address_of("postgres://app:secret@localhost:5433/forgedesmondes").as_deref(),
            Some("localhost:5433")
        );
        assert_eq!(
            address_of("postgresql://db.local/app").as_deref(),
            Some("db.local:5432")
        );
        assert_eq!(
            address_of("postgres://u@h:5432/d?sslmode=disable").as_deref(),
            Some("h:5432")
        );
        assert_eq!(address_of("mysql://h/d"), None);
        assert_eq!(address_of("postgres:///d"), None);
        assert_eq!(address_of(""), None);
    }

    #[test]
    fn the_database_is_read_off_the_url() {
        assert_eq!(
            database_of("postgres://app:secret@localhost:5433/forgedesmondes").as_deref(),
            Some("forgedesmondes")
        );
        assert_eq!(
            database_of("postgres://u@h:5432/d?sslmode=disable").as_deref(),
            Some("d")
        );
        assert_eq!(
            database_of("postgresql://db.local/app").as_deref(),
            Some("app")
        );
        assert_eq!(database_of("postgres://u@h:5432"), None);
        assert_eq!(database_of("postgres://u@h:5432/"), None);
        assert_eq!(database_of("postgres://u@h:5432/?sslmode=disable"), None);
        assert_eq!(database_of("mysql://h/d"), None);
        assert_eq!(database_of("not a url"), None);
    }

    #[test]
    fn a_name_is_quoted_and_its_inner_quotes_doubled() {
        assert_eq!(quoted("scratch_1_2_3_x"), "\"scratch_1_2_3_x\"");
        assert_eq!(quoted("a\"b"), "\"a\"\"b\"");
    }

    fn refusal(sqlstate: Option<&str>) -> Refusal {
        Refusal {
            sqlstate: sqlstate.map(ToString::to_string),
            reason: "why".to_string(),
        }
    }

    #[test]
    fn object_in_use_is_busy_another_refusal_goes_on_a_lost_server_stops() {
        assert_eq!(verdict(refusal(Some("55006"))), (Dropped::Busy, true));
        assert_eq!(
            verdict(refusal(Some("42501"))),
            (Dropped::Refused("why".to_string()), true),
            "a missing right is no busy database, and the next drop may pass"
        );
        assert_eq!(
            verdict(refusal(None)),
            (Dropped::Refused("why".to_string()), false)
        );
    }

    fn names(count: usize) -> Vec<String> {
        (0..count).map(|n| format!("scratch_1_2_{n}_x")).collect()
    }

    const LATER: Duration = Duration::from_secs(60);

    #[tokio::test]
    async fn every_name_is_dropped_in_order_before_the_deadline() {
        let reached = drop_each(&names(3), LATER, async |_| Ok(())).await;
        assert_eq!(
            reached,
            names(3)
                .into_iter()
                .map(|name| Dropping {
                    name,
                    dropped: Dropped::Gone
                })
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn a_deadline_already_past_reaches_no_name() {
        let reached = drop_each(&names(3), Duration::ZERO, async |_| Ok(())).await;
        assert_eq!(reached, []);
    }

    #[tokio::test]
    async fn busy_and_refused_go_on_and_a_lost_server_ends_the_pass() {
        let reached = drop_each(&names(4), LATER, async |name: &str| {
            match name.split('_').nth(3) {
                Some("0") => Err(refusal(Some("55006"))),
                Some("1") => Err(refusal(Some("42501"))),
                Some("2") => Err(refusal(None)),
                _ => Ok(()),
            }
        })
        .await;
        let said: Vec<Dropped> = reached.into_iter().map(|d| d.dropped).collect();
        assert_eq!(
            said,
            [
                Dropped::Busy,
                Dropped::Refused("why".to_string()),
                Dropped::Refused("why".to_string()),
            ],
            "the fourth is left for the next sweep"
        );
    }

    #[tokio::test]
    async fn a_drop_hung_past_the_deadline_ends_the_pass() {
        let reached = drop_each(&names(2), Duration::from_millis(50), async |_| {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(())
        })
        .await;
        assert_eq!(
            reached,
            [Dropping {
                name: names(1).remove(0),
                dropped: Dropped::Refused("no answer before the sweep's deadline".to_string()),
            }]
        );
    }

    #[tokio::test]
    async fn a_server_that_does_not_answer_is_a_failure_that_keeps_the_url_to_itself() {
        // Port 1 serves nothing on a developer machine or a CI runner.
        let databases = PgDatabases::new("postgres://app:secret@127.0.0.1:1/forge".to_string());
        let listed = databases.owned().await.expect_err("nothing listens");
        assert!(listed.reason().contains("the listing"), "{listed:?}");
        assert!(!listed.reason().contains("secret"), "{listed:?}");
        let dropped = databases
            .drop_idle(&names(1), Duration::from_secs(5))
            .await
            .expect_err("nothing listens");
        assert!(dropped.reason().contains("a drop"), "{dropped:?}");
        assert!(!dropped.reason().contains("secret"), "{dropped:?}");
        assert_eq!(
            databases
                .drop_idle(&[], Duration::from_secs(5))
                .await
                .expect("nothing to drop, nothing to connect to"),
            []
        );
        let unreadable = PgDatabases::new("not a url".to_string());
        assert!(unreadable.owned().await.is_err());
    }

    /// Against a real server, under a role that is no superuser:
    /// `PG_SWEEP_URL=postgres://app:…@localhost:5432/app cargo test -p
    /// harness-adapters -- --ignored owned_and_dropped`. It creates three
    /// databases of its own and drops them.
    #[tokio::test]
    #[ignore = "needs a PostgreSQL: PG_SWEEP_URL"]
    async fn owned_and_dropped_against_a_real_server() {
        let url = std::env::var("PG_SWEEP_URL").expect("PG_SWEEP_URL");
        let databases = PgDatabases::new(url.clone());
        let (idle, busy) = (
            "scratch_1000000000_1_0_sweep_idle",
            "scratch_1000000000_1_1_sweep_busy",
        );
        let (client, connection) = tokio_postgres::connect(&url, NoTls).await.expect("connect");
        let driver = tokio::spawn(connection);
        for name in [idle, busy] {
            // One statement a call: two would share a transaction block.
            for statement in ["DROP DATABASE IF EXISTS", "CREATE DATABASE"] {
                client
                    .batch_execute(&format!("{statement} {}", quoted(name)))
                    .await
                    .expect(statement);
            }
        }
        // PG_SWEEP_URL carries no query: its database is its last segment.
        let held = format!("{}/{busy}", url.rsplit_once('/').expect("a database").0);
        let (holder, held_connection) = tokio_postgres::connect(&held, NoTls).await.expect("hold");
        let holding = tokio::spawn(held_connection);
        let owned = databases.owned().await.expect("listed");
        let of = |name: &str| owned.iter().find(|d| d.name == name).cloned();
        assert_eq!(of(idle).map(|d| d.connections), Some(0), "{owned:?}");
        assert_eq!(of(busy).map(|d| d.connections), Some(1), "{owned:?}");
        assert!(
            of("template0").is_none(),
            "owned by the superuser: {owned:?}"
        );
        // One pass: the busy one is kept and the pass goes on; the idle one
        // goes, twice over (`IF EXISTS`).
        let pass = [busy.to_string(), idle.to_string(), idle.to_string()];
        let said: Vec<Dropped> = databases
            .drop_idle(&pass, Duration::from_secs(30))
            .await
            .expect("one connection")
            .into_iter()
            .map(|d| d.dropped)
            .collect();
        assert_eq!(said, [Dropped::Busy, Dropped::Gone, Dropped::Gone]);
        drop(holder);
        holding.abort();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let again = databases
            .drop_idle(&[busy.to_string()], Duration::from_secs(30))
            .await
            .expect("one connection");
        assert_eq!(
            again,
            [Dropping {
                name: busy.to_string(),
                dropped: Dropped::Gone
            }]
        );
        let left = databases.owned().await.expect("listed");
        assert!(
            left.iter().all(|d| d.name != idle && d.name != busy),
            "{left:?}"
        );
        drop(client);
        driver.abort();
    }

    #[tokio::test]
    async fn a_closed_port_does_not_answer_and_an_open_one_does() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();
        assert_eq!(
            answers(&format!("postgres://u:p@127.0.0.1:{port}/d")).await,
            Some(true)
        );
        // Not the port just released: a test beside this one may bind it
        // again first. Port 1 (tcpmux) is privileged and served by nothing
        // on a developer machine or a CI runner.
        drop(listener);
        assert_eq!(answers("postgres://u:p@127.0.0.1:1/d").await, Some(false));
        assert_eq!(answers("not a url").await, None);
    }
}
