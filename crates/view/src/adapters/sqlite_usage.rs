//! What the view's own `claude -p` calls cost, in `.llocal/harness.db`
//! beside the event store and the pins — a table of its own, nothing a run
//! reads.

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, Row, params};

use crate::ports::{Spend, Usage};

/// How long a writer waits for another one.
const BUSY: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS usage (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    at             TEXT NOT NULL,
    who            TEXT NOT NULL,
    model          TEXT NOT NULL,
    mode           TEXT NOT NULL,
    cost_usd       REAL,
    duration_ms    INTEGER,
    input_tokens   INTEGER,
    output_tokens  INTEGER
);
";

/// The table, open.
pub struct SqliteUsage {
    conn: Mutex<Connection>,
}

fn spend_of(row: &Row<'_>) -> rusqlite::Result<Spend> {
    Ok(Spend {
        at: row.get(0)?,
        who: row.get(1)?,
        model: row.get(2)?,
        mode: row.get(3)?,
        cost_usd: row.get(4)?,
        duration_ms: row
            .get::<_, Option<i64>>(5)?
            .map(|v| u64::try_from(v).unwrap_or(0)),
        input_tokens: row
            .get::<_, Option<i64>>(6)?
            .map(|v| u64::try_from(v).unwrap_or(0)),
        output_tokens: row
            .get::<_, Option<i64>>(7)?
            .map(|v| u64::try_from(v).unwrap_or(0)),
    })
}

impl SqliteUsage {
    /// Opens the table at `db` (the event store's own file), creating it
    /// when it is not there.
    ///
    /// # Errors
    ///
    /// The database cannot be opened or created.
    pub fn open(db: &Path) -> Result<Self, String> {
        if let Some(dir) = db.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let conn = Connection::open(db).map_err(|e| format!("{}: {e}", db.display()))?;
        conn.busy_timeout(BUSY).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl Usage for SqliteUsage {
    fn log(&self, spend: &Spend) -> Result<(), String> {
        self.conn
            .lock()
            .map_err(|_| "poisoned".to_string())?
            .execute(
                "INSERT INTO usage (at, who, model, mode, cost_usd, duration_ms, input_tokens, output_tokens)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    spend.at,
                    spend.who,
                    spend.model,
                    spend.mode,
                    spend.cost_usd,
                    spend.duration_ms.map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
                    spend.input_tokens.map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
                    spend.output_tokens.map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
                ],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    // The connection's guard outlives the statement that borrows it, which
    // is the whole of this function: there is nowhere sooner to drop it.
    #[allow(clippy::significant_drop_tightening)]
    fn all(&self) -> Result<Vec<Spend>, String> {
        let conn = self.conn.lock().map_err(|_| "poisoned".to_string())?;
        let mut statement = conn
            .prepare(
                "SELECT at, who, model, mode, cost_usd, duration_ms, input_tokens, output_tokens
                 FROM usage ORDER BY id DESC",
            )
            .map_err(|e| e.to_string())?;
        statement
            .query_map([], spend_of)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> SqliteUsage {
        let dir = std::env::temp_dir().join(format!("harness-usage-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        SqliteUsage::open(&dir.join("harness.db")).expect("opened")
    }

    fn spend(who: &str, cost: f64) -> Spend {
        Spend {
            at: "2026-10-12T00:00:00Z".to_string(),
            who: who.to_string(),
            model: "claude-opus-5-5".to_string(),
            mode: "write".to_string(),
            cost_usd: Some(cost),
            duration_ms: Some(1234),
            input_tokens: Some(100),
            output_tokens: Some(200),
        }
    }

    #[test]
    fn calls_are_kept_newest_first() {
        let store = store("newest-first");
        store.log(&spend("creation", 0.5)).expect("logged");
        store.log(&spend("kit", 0.25)).expect("logged");
        let kept = store.all().expect("read");
        assert_eq!(
            kept.iter().map(|s| s.who.as_str()).collect::<Vec<_>>(),
            ["kit", "creation"]
        );
        assert_eq!(kept[0].cost_usd, Some(0.25));
        assert_eq!(kept[0].duration_ms, Some(1234));
    }

    #[test]
    fn a_call_with_no_cost_is_kept_as_none_not_zero() {
        let store = store("no-cost");
        let mut s = spend("mf", 0.0);
        s.cost_usd = None;
        s.duration_ms = None;
        store.log(&s).expect("logged");
        assert_eq!(store.all().expect("read")[0].cost_usd, None);
    }

    #[test]
    fn a_table_reopened_still_has_what_it_kept() {
        let dir = std::env::temp_dir().join(format!("harness-usage-reopen-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let db = dir.join("harness.db");
        SqliteUsage::open(&db)
            .expect("opened")
            .log(&spend("notes", 0.1))
            .expect("logged");
        assert_eq!(
            SqliteUsage::open(&db)
                .expect("opened")
                .all()
                .expect("read")
                .len(),
            1
        );
    }
}
