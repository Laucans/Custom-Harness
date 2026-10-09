//! The event store: one SQLite database per checkout, at
//! [`Workspace::events`](crate::domain::workspace::Workspace::events).
//!
//! SQLite rather than a file of lines because several processes write at
//! once: each insert is a transaction, so no line is ever half written or
//! interleaved, and readers query by id or by time instead of re-reading a
//! file from the start. Rather than a message broker because nothing has to
//! run for it: the database is a file, and an event written while no reader
//! listens is still there.
//!
//! WAL mode lets readers read while a writer writes; the busy timeout makes a
//! writer wait for another rather than fail.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, params};

use crate::domain::{Halt, Outcome};
use crate::ports::store::events::{EventLog, Stored};
use crate::traces::Event;

/// How long a writer waits for another one before giving up.
const BUSY: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
    id     INTEGER PRIMARY KEY AUTOINCREMENT,
    at     TEXT NOT NULL,
    source TEXT NOT NULL,
    kind   TEXT NOT NULL,
    body   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS events_at ON events (at);
";

/// The store, open.
///
/// A connection behind a lock: `rusqlite`'s connection may move between
/// threads but not be shared, and the view's handlers share it.
pub struct SqliteEvents {
    conn: Mutex<Connection>,
}

fn failed(what: &str, e: impl std::fmt::Display) -> Halt {
    Halt::Failed(format!("the event store {what}: {e}"))
}

impl SqliteEvents {
    /// Opens the store at `path`, creating it and its directory when absent.
    ///
    /// # Errors
    ///
    /// The directory or the database cannot be created or opened.
    pub fn open(path: &Path) -> Outcome<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| failed("has no directory", e))?;
        }
        let conn = Connection::open(path).map_err(|e| failed("cannot be opened", e))?;
        conn.busy_timeout(BUSY)
            .map_err(|e| failed("cannot wait", e))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| failed("cannot use WAL", e))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| failed("cannot be created", e))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // The prepared statement borrows the connection: the guard cannot go
    // before the rows are collected, which is where the block already ends it.
    #[allow(clippy::significant_drop_tightening)]
    fn query(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Outcome<Vec<Stored>> {
        let rows = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| failed("is poisoned", "a reader panicked"))?;
            let mut statement = conn.prepare(sql).map_err(|e| failed("cannot be read", e))?;
            let mapped = statement
                .query_map(args, |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|e| failed("cannot be read", e))?;
            mapped
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| failed("cannot be read", e))?
        };
        // An event this build does not know — written by a newer one — is
        // skipped, never a failure of the whole read.
        Ok(rows
            .into_iter()
            .filter_map(|(id, at, source, body)| {
                serde_json::from_str::<Event>(&body)
                    .ok()
                    .map(|event| Stored {
                        id: u64::try_from(id).unwrap_or(0),
                        at,
                        source,
                        event,
                    })
            })
            .collect())
    }
}

impl EventLog for SqliteEvents {
    fn append(&self, at: &str, source: &str, event: &Event) -> Outcome<()> {
        let body = serde_json::to_string(event).map_err(|e| failed("cannot encode", e))?;
        let kind = serde_json::to_value(event)
            .ok()
            .and_then(|v| v.get("event").and_then(|k| k.as_str()).map(str::to_string))
            .unwrap_or_default();
        self.conn
            .lock()
            .map_err(|_| failed("is poisoned", "a writer panicked"))?
            .execute(
                "INSERT INTO events (at, source, kind, body) VALUES (?1, ?2, ?3, ?4)",
                params![at, source, kind, body],
            )
            .map_err(|e| failed("cannot be written", e))?;
        Ok(())
    }

    fn after(&self, id: u64, limit: usize) -> Outcome<Vec<Stored>> {
        let id = i64::try_from(id).unwrap_or(i64::MAX);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.query(
            "SELECT id, at, source, body FROM events WHERE id > ?1 ORDER BY id LIMIT ?2",
            &[&id, &limit],
        )
    }

    fn between(&self, from: Option<&str>, to: Option<&str>, limit: usize) -> Outcome<Vec<Stored>> {
        let from = from.unwrap_or("");
        let to = to.unwrap_or("9999");
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut newest = self.query(
            "SELECT id, at, source, body FROM events WHERE at >= ?1 AND at <= ?2 \
             ORDER BY id DESC LIMIT ?3",
            &[&from, &to, &limit],
        )?;
        newest.reverse();
        Ok(newest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("harness-events-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn what_is_appended_reads_back_in_order_by_id_and_by_time() {
        let dir = Dir::new("order");
        let store = SqliteEvents::open(&dir.0.join("harness.db")).expect("opened");
        store
            .append("2026-10-09T01:00:00Z", "watch", &Event::WatchStopped)
            .expect("written");
        store
            .append(
                "2026-10-09T02:00:00Z",
                "watch",
                &Event::TaskParked { task: 15 },
            )
            .expect("written");
        let all = store.after(0, 10).expect("read");
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].event, Event::TaskParked { task: 15 });
        assert!(all[0].id < all[1].id);
        assert_eq!(store.after(all[0].id, 10).expect("read").len(), 1);
        let late = store
            .between(Some("2026-10-09T01:30:00Z"), None, 10)
            .expect("read");
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].at, "2026-10-09T02:00:00Z");
    }

    #[test]
    fn two_writers_on_one_store_lose_nothing() {
        let dir = Dir::new("writers");
        let path = dir.0.join("harness.db");
        let a = SqliteEvents::open(&path).expect("a");
        let b = SqliteEvents::open(&path).expect("b");
        let writers: Vec<_> = [(a, "lane-0"), (b, "lane-1")]
            .into_iter()
            .map(|(store, source)| {
                std::thread::spawn(move || {
                    for task in 0..50 {
                        store
                            .append("2026-10-09T01:00:00Z", source, &Event::TaskParked { task })
                            .expect("written");
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("joined");
        }
        let reader = SqliteEvents::open(&path).expect("reader");
        assert_eq!(reader.after(0, 1000).expect("read").len(), 100);
    }

    #[test]
    fn an_event_this_build_does_not_know_is_skipped() {
        let dir = Dir::new("unknown");
        let path = dir.0.join("harness.db");
        let store = SqliteEvents::open(&path).expect("opened");
        store
            .conn
            .lock()
            .expect("lock")
            .execute(
                "INSERT INTO events (at, source, kind, body) VALUES ('t', 's', 'later', '{\"event\":\"later\"}')",
                [],
            )
            .expect("raw insert");
        store
            .append("t", "s", &Event::WatchStopped)
            .expect("written");
        assert_eq!(store.after(0, 10).expect("read").len(), 1);
    }
}
