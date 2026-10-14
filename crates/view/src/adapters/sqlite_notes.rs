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
use crate::domain::drafts::Draft;
use crate::domain::mock_history::Entry;
use crate::domain::pin_journal::Event;
use crate::domain::steps::Step;
use crate::ports::{DraftStore, HistoryStore, KeptNote, NoteStore, PinJournal, StepStore};

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
CREATE TABLE IF NOT EXISTS drafts (
    board        TEXT NOT NULL,
    version      INTEGER NOT NULL,
    body         TEXT NOT NULL,
    PRIMARY KEY (board, version)
);
CREATE TABLE IF NOT EXISTS pin_journal (
    pin          TEXT NOT NULL,
    seq          INTEGER NOT NULL,
    body         TEXT NOT NULL,
    PRIMARY KEY (pin, seq)
);
CREATE TABLE IF NOT EXISTS history (
    branch       TEXT NOT NULL,
    id           INTEGER NOT NULL,
    body         TEXT NOT NULL,
    PRIMARY KEY (branch, id)
);
CREATE TABLE IF NOT EXISTS steps (
    branch       TEXT NOT NULL,
    pin          TEXT NOT NULL,
    seq          INTEGER NOT NULL,
    body         TEXT NOT NULL,
    PRIMARY KEY (branch, pin)
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
                NoteStore::put(self, &board, &note)?;
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

impl DraftStore for SqliteNotes {
    fn boards(&self) -> Result<Vec<String>, String> {
        self.with(|conn| {
            let mut statement = conn.prepare("SELECT DISTINCT board FROM drafts ORDER BY board")?;
            let mapped = statement.query_map([], |row| row.get(0))?;
            mapped.collect()
        })
    }

    fn versions(&self, board: &str) -> Result<Vec<Draft>, String> {
        let rows: Vec<String> = self.with(|conn| {
            let mut statement =
                conn.prepare("SELECT body FROM drafts WHERE board = ?1 ORDER BY version")?;
            let mapped = statement.query_map(params![board], |row| row.get(0))?;
            mapped.collect()
        })?;
        Ok(rows
            .iter()
            .filter_map(|body| serde_json::from_str(body).ok())
            .collect())
    }

    fn put(&self, board: &str, draft: &Draft) -> Result<(), String> {
        let body = serde_json::to_string(draft).map_err(|e| e.to_string())?;
        self.with(|conn| {
            conn.execute(
                "INSERT INTO drafts (board, version, body) VALUES (?1, ?2, ?3)
                 ON CONFLICT (board, version) DO UPDATE SET body = excluded.body",
                params![board, draft.version, body],
            )
        })
        .map(|_| ())
    }

    fn destroy(&self, board: &str, version: u32) -> Result<(), String> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM drafts WHERE board = ?1 AND version = ?2",
                params![board, version],
            )
        })
        .map(|_| ())
    }
}

impl PinJournal for SqliteNotes {
    fn events(&self, pin: &str) -> Result<Vec<Event>, String> {
        let rows: Vec<String> = self.with(|conn| {
            let mut statement =
                conn.prepare("SELECT body FROM pin_journal WHERE pin = ?1 ORDER BY seq")?;
            let mapped = statement.query_map(params![pin], |row| row.get(0))?;
            mapped.collect()
        })?;
        Ok(rows
            .iter()
            .filter_map(|body| serde_json::from_str(body).ok())
            .collect())
    }

    fn put(&self, pin: &str, event: &Event) -> Result<(), String> {
        let body = serde_json::to_string(event).map_err(|e| e.to_string())?;
        let seq = i64::try_from(event.seq).map_err(|e| e.to_string())?;
        self.with(|conn| {
            conn.execute(
                "INSERT INTO pin_journal (pin, seq, body) VALUES (?1, ?2, ?3)
                 ON CONFLICT (pin, seq) DO UPDATE SET body = excluded.body",
                params![pin, seq, body],
            )
        })
        .map(|_| ())
    }
}

impl HistoryStore for SqliteNotes {
    fn entries(&self, branch: &str) -> Result<Vec<Entry>, String> {
        let rows: Vec<String> = self.with(|conn| {
            let mut statement =
                conn.prepare("SELECT body FROM history WHERE branch = ?1 ORDER BY id")?;
            let mapped = statement.query_map(params![branch], |row| row.get(0))?;
            mapped.collect()
        })?;
        Ok(rows
            .iter()
            .filter_map(|body| serde_json::from_str(body).ok())
            .collect())
    }

    fn put(&self, branch: &str, entry: &Entry) -> Result<(), String> {
        let body = serde_json::to_string(entry).map_err(|e| e.to_string())?;
        let id = i64::try_from(entry.id).map_err(|e| e.to_string())?;
        self.with(|conn| {
            conn.execute(
                "INSERT INTO history (branch, id, body) VALUES (?1, ?2, ?3)
                 ON CONFLICT (branch, id) DO UPDATE SET body = excluded.body",
                params![branch, id, body],
            )
        })
        .map(|_| ())
    }

    fn remove(&self, branch: &str, id: u64) -> Result<(), String> {
        let id = i64::try_from(id).map_err(|e| e.to_string())?;
        self.with(|conn| {
            conn.execute(
                "DELETE FROM history WHERE branch = ?1 AND id = ?2",
                params![branch, id],
            )
        })
        .map(|_| ())
    }
}

impl StepStore for SqliteNotes {
    fn steps(&self, branch: &str) -> Result<Vec<Step>, String> {
        let rows: Vec<String> = self.with(|conn| {
            let mut statement =
                conn.prepare("SELECT body FROM steps WHERE branch = ?1 ORDER BY seq")?;
            let mapped = statement.query_map(params![branch], |row| row.get(0))?;
            mapped.collect()
        })?;
        Ok(rows
            .iter()
            .filter_map(|body| serde_json::from_str(body).ok())
            .collect())
    }

    fn put(&self, branch: &str, step: &Step) -> Result<(), String> {
        let body = serde_json::to_string(step).map_err(|e| e.to_string())?;
        let seq = i64::try_from(step.last()).map_err(|e| e.to_string())?;
        self.with(|conn| {
            conn.execute(
                "INSERT INTO steps (branch, pin, seq, body) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (branch, pin) DO UPDATE SET seq = excluded.seq, body = excluded.body",
                params![branch, step.pin, seq, body],
            )
        })
        .map(|_| ())
    }

    fn remove(&self, branch: &str, pin: &str) -> Result<(), String> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM steps WHERE branch = ?1 AND pin = ?2",
                params![branch, pin],
            )
        })
        .map(|_| ())
    }
}

impl NoteStore for SqliteNotes {
    fn boards(&self) -> Result<Vec<String>, String> {
        self.with(|conn| {
            let mut statement = conn.prepare(
                "SELECT DISTINCT board FROM notes WHERE deleted_at IS NULL ORDER BY board",
            )?;
            let mapped = statement.query_map([], |row| row.get(0))?;
            mapped.collect()
        })
    }

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
            part: String::new(),
            source: String::new(),
            state: crate::domain::annotations::RequestState::Pending,
            issue: String::new(),
            shape: None,
            turns: 0,
        }
    }

    #[test]
    fn a_pin_s_journal_round_trips_in_its_order() {
        let dir = std::env::temp_dir().join(format!("harness-journal-db-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let store =
            SqliteNotes::open(&dir.join("harness.db"), &dir.join("none.json")).expect("open");
        let event = |seq: u64, kind: &str| Event {
            seq,
            kind: kind.to_string(),
            ..Event::default()
        };
        PinJournal::put(&store, "p1", &event(2, "words")).expect("kept");
        PinJournal::put(&store, "p1", &event(1, "placed")).expect("kept");
        PinJournal::put(&store, "p2", &event(1, "placed")).expect("kept");
        PinJournal::put(&store, "p1", &event(2, "words again")).expect("a new state");
        let kept = store.events("p1").expect("read");
        assert_eq!(
            kept.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            ["placed", "words again"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_entries_round_trip_by_branch_in_their_order() {
        let dir = std::env::temp_dir().join(format!("harness-history-db-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let store =
            SqliteNotes::open(&dir.join("harness.db"), &dir.join("none.json")).expect("open");
        let entry = |id: u64| Entry {
            id,
            what: format!("entry {id}"),
            ..Entry::default()
        };
        HistoryStore::put(&store, "main", &entry(2)).expect("kept");
        HistoryStore::put(&store, "main", &entry(1)).expect("kept");
        HistoryStore::put(&store, "other", &entry(1)).expect("kept");
        let mut undone = entry(2);
        undone.undone = true;
        HistoryStore::put(&store, "main", &undone).expect("a new state");
        let kept = store.entries("main").expect("read");
        assert_eq!(kept.iter().map(|e| e.id).collect::<Vec<_>>(), [1, 2]);
        assert!(kept[1].undone);
        HistoryStore::remove(&store, "main", 1).expect("gone");
        assert_eq!(store.entries("main").expect("read").len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn steps_round_trip_by_branch_and_pin_in_their_order() {
        let dir = std::env::temp_dir().join(format!("harness-steps-db-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let store =
            SqliteNotes::open(&dir.join("harness.db"), &dir.join("none.json")).expect("open");
        let mut late = Step {
            pin: "b".into(),
            ..Step::default()
        };
        late.keep("components/X.draft.html", Some("a's".into()), 2);
        StepStore::put(&store, "main", &late).expect("kept");
        StepStore::put(
            &store,
            "main",
            &Step {
                pin: "a".into(),
                files: vec![crate::domain::steps::Before {
                    path: "components/X.draft.html".into(),
                    text: None,
                    seq: 1,
                }],
                ..Step::default()
            },
        )
        .expect("kept");
        StepStore::put(&store, "other", &Step::default()).expect("kept");
        let steps = store.steps("main").expect("read");
        assert_eq!(
            steps.iter().map(|s| s.pin.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(steps[1], late);
        late.keep("pages.json", None, 3);
        StepStore::put(&store, "main", &late).expect("a new state");
        assert_eq!(store.steps("main").expect("read")[1].files.len(), 2);
        StepStore::remove(&store, "main", "a").expect("gone");
        assert_eq!(store.steps("main").expect("read").len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn drafts_round_trip_by_board_and_version() {
        let dir = std::env::temp_dir().join(format!("harness-drafts-db-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let store =
            SqliteNotes::open(&dir.join("harness.db"), &dir.join("none.json")).expect("open");
        let mut draft = Draft {
            version: 1,
            name: "first".into(),
            ..Draft::default()
        };
        DraftStore::put(&store, "mockup--main--home", &draft).expect("kept");
        draft.version = 2;
        DraftStore::put(&store, "mockup--main--home", &draft).expect("kept");
        assert_eq!(
            DraftStore::boards(&store).expect("boards"),
            ["mockup--main--home"]
        );
        let versions = store.versions("mockup--main--home").expect("read");
        assert_eq!(
            versions.iter().map(|d| d.version).collect::<Vec<_>>(),
            [1, 2]
        );
        DraftStore::destroy(&store, "mockup--main--home", 1).expect("gone");
        assert_eq!(store.versions("mockup--main--home").expect("read").len(), 1);
        let _ = fs::remove_dir_all(&dir);
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

        NoteStore::put(&store, "data-model", &note("a", "first")).expect("kept");
        NoteStore::put(&store, "data-model", &note("a", "second")).expect("kept");
        store
            .save_conversation("data-model", "a", "{\"turns\":[]}", 4)
            .expect("kept");
        NoteStore::put(&store, "data-model", &note("a", "third"))
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
        NoteStore::destroy(&store, "data-model", "a").expect("gone");
        assert_eq!(store.conversation("data-model", "a").expect("read"), None);
        assert!(store.notes("elsewhere").expect("read").is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
