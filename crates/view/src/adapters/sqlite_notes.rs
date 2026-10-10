//! The human's pins and their conversations, in `.llocal/harness.db` beside
//! the event store — tables of their own, nothing a run reads.
//!
//! The first open imports the pins the view used to keep in
//! `.llocal/annotations.json`, then renames that file `.imported`.

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

#[cfg(test)]
use crate::domain::annotations::Kind;
use crate::domain::annotations::{self, Note};
use crate::ports::{KeptNote, NoteStore};

/// How long a writer waits for another one.
const BUSY: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS notes (
    board        TEXT NOT NULL,
    id           TEXT NOT NULL,
    body         TEXT NOT NULL,
    deleted_at   TEXT,
    conversation TEXT,
    turns        INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (board, id)
);
";

/// The pins' tables, open.
pub struct SqliteNotes {
    conn: Mutex<Connection>,
}

impl SqliteNotes {
    /// Opens the store at `db` (the event store's file), creating the tables,
    /// and imports `legacy` once when it is there.
    ///
    /// # Errors
    ///
    /// The database cannot be opened or created.
    pub fn open(db: &Path, legacy: &Path) -> Result<Self, String> {
        if let Some(dir) = db.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let conn = Connection::open(db).map_err(|e| format!("{}: {e}", db.display()))?;
        conn.busy_timeout(BUSY).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.import(legacy)?;
        Ok(store)
    }

    fn import(&self, legacy: &Path) -> Result<(), String> {
        let Ok(text) = fs::read_to_string(legacy) else {
            return Ok(());
        };
        for (board, notes) in annotations::shelf(Some(&text)) {
            for note in notes {
                self.put(&board, &note)?;
            }
        }
        let done = legacy.with_extension("json.imported");
        fs::rename(legacy, &done).map_err(|e| format!("{}: {e}", legacy.display()))
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| "the pins' store is poisoned".to_string())?;
        f(&conn).map_err(|e| format!("the pins' store: {e}"))
    }
}

impl NoteStore for SqliteNotes {
    fn notes(&self, board: &str) -> Result<Vec<KeptNote>, String> {
        let rows: Vec<(String, Option<String>, i64)> = self.with(|conn| {
            let mut statement = conn.prepare(
                "SELECT body, deleted_at, turns FROM notes WHERE board = ?1 ORDER BY rowid",
            )?;
            let mapped = statement.query_map(params![board], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
            mapped.collect()
        })?;
        Ok(rows
            .into_iter()
            .filter_map(|(body, deleted_at, turns)| {
                let mut note: Note = serde_json::from_str(&body).ok()?;
                note.turns = usize::try_from(turns).unwrap_or(0);
                Some(KeptNote { note, deleted_at })
            })
            .collect())
    }

    fn put(&self, board: &str, note: &Note) -> Result<(), String> {
        let mut kept = note.clone();
        kept.turns = 0;
        let body = serde_json::to_string(&kept).map_err(|e| e.to_string())?;
        self.with(|conn| {
            conn.execute(
                "INSERT INTO notes (board, id, body) VALUES (?1, ?2, ?3)
                 ON CONFLICT (board, id) DO UPDATE SET body = excluded.body",
                params![board, note.id, body],
            )
        })
        .map(|_| ())
    }

    fn mark_deleted(&self, board: &str, id: &str, at: Option<&str>) -> Result<(), String> {
        self.with(|conn| {
            conn.execute(
                "UPDATE notes SET deleted_at = ?3 WHERE board = ?1 AND id = ?2",
                params![board, id, at],
            )
        })
        .map(|_| ())
    }

    fn destroy(&self, board: &str, id: &str) -> Result<(), String> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM notes WHERE board = ?1 AND id = ?2",
                params![board, id],
            )
        })
        .map(|_| ())
    }

    fn conversation(&self, board: &str, id: &str) -> Result<Option<String>, String> {
        self.with(|conn| {
            conn.query_row(
                "SELECT conversation FROM notes WHERE board = ?1 AND id = ?2",
                params![board, id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
        })
        .map(Option::flatten)
    }

    fn save_conversation(
        &self,
        board: &str,
        id: &str,
        json: &str,
        turns: usize,
    ) -> Result<(), String> {
        let turns = i64::try_from(turns).unwrap_or(i64::MAX);
        self.with(|conn| {
            conn.execute(
                "UPDATE notes SET conversation = ?3, turns = ?4 WHERE board = ?1 AND id = ?2",
                params![board, id, json, turns],
            )
        })
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, text: &str) -> Note {
        Note {
            id: id.to_string(),
            target: "table:campaign".to_string(),
            dx: 1.0,
            dy: 2.0,
            x: 3.0,
            y: 4.0,
            text: text.to_string(),
            at: String::new(),
            kind: Kind::Brainstorm,
            page: String::new(),
            component: String::new(),
            shape: None,
            turns: 0,
        }
    }

    #[test]
    fn pins_round_trip_soft_delete_and_keep_their_conversation() {
        let dir = std::env::temp_dir().join(format!("harness-notes-db-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let legacy = dir.join("annotations.json");
        fs::write(
            &legacy,
            r#"{"data-model":[{"id":"old","text":"from the file"}]}"#,
        )
        .expect("legacy");
        let store = SqliteNotes::open(&dir.join("harness.db"), &legacy).expect("opened");
        assert!(!legacy.exists(), "imported once, then set aside");
        assert!(dir.join("annotations.json.imported").exists());

        store.put("data-model", &note("a", "first")).expect("kept");
        store.put("data-model", &note("a", "second")).expect("kept");
        store
            .save_conversation("data-model", "a", "{\"turns\":[]}", 4)
            .expect("kept");
        store
            .put("data-model", &note("a", "third"))
            .expect("a new version keeps the conversation");
        let notes = store.notes("data-model").expect("read");
        assert_eq!(notes.len(), 2);
        let a = notes.iter().find(|k| k.note.id == "a").expect("a");
        assert_eq!(a.note.text, "third");
        assert_eq!(a.note.turns, 4);
        assert_eq!(a.note.kind, Kind::Brainstorm);
        assert_eq!(
            store
                .conversation("data-model", "a")
                .expect("read")
                .as_deref(),
            Some("{\"turns\":[]}")
        );

        store
            .mark_deleted("data-model", "a", Some("2026-10-09T12:00:00Z"))
            .expect("marked");
        assert!(
            store
                .notes("data-model")
                .expect("read")
                .iter()
                .any(|k| k.deleted_at.is_some())
        );
        store.mark_deleted("data-model", "a", None).expect("back");
        store.destroy("data-model", "a").expect("gone");
        assert_eq!(store.conversation("data-model", "a").expect("read"), None);
        assert!(store.notes("elsewhere").expect("read").is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
