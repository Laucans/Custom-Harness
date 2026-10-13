//! The register of factories: one SQLite database for the machine, outside
//! every checkout (`~/.harness/factories.db` by default).
//!
//! SQLite for the reason the event store is: several processes open it at
//! once (a view per switch, later the watches), and each write is a
//! transaction.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Row, params};

use harness_core::domain::{Halt, Outcome};
use harness_core::ports::store::factories::{Factories, Factory, id_from};

/// How long a writer waits for another one before giving up.
const BUSY: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS factories (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    slug               TEXT NOT NULL,
    root               TEXT NOT NULL UNIQUE,
    integration_branch TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS creations (
    id     TEXT PRIMARY KEY,
    record TEXT NOT NULL
);
";

const COLUMNS: &str = "id, name, slug, root, integration_branch";

/// The register, open.
pub struct SqliteFactories {
    conn: Mutex<Connection>,
}

fn failed(what: &str, e: impl std::fmt::Display) -> Halt {
    Halt::Failed(format!("the factory register {what}: {e}"))
}

fn factory_of(row: &Row<'_>) -> rusqlite::Result<Factory> {
    Ok(Factory {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        root: PathBuf::from(row.get::<_, String>(3)?),
        integration_branch: row.get(4)?,
    })
}

impl SqliteFactories {
    /// Opens the register at `path`, creating it and its directory when absent.
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
}

impl Factories for SqliteFactories {
    // The prepared statement borrows the connection: the guard lives as long
    // as the rows are collected, which is where the function ends.
    #[allow(clippy::significant_drop_tightening)]
    fn list(&self) -> Outcome<Vec<Factory>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| failed("is poisoned", "a writer panicked"))?;
        let mut statement = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM factories ORDER BY name COLLATE NOCASE, id"
            ))
            .map_err(|e| failed("cannot be read", e))?;
        statement
            .query_map([], factory_of)
            .map_err(|e| failed("cannot be read", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| failed("cannot be read", e))
    }

    fn get(&self, id: &str) -> Outcome<Option<Factory>> {
        self.conn
            .lock()
            .map_err(|_| failed("is poisoned", "a writer panicked"))?
            .query_row(
                &format!("SELECT {COLUMNS} FROM factories WHERE id = ?1"),
                params![id],
                factory_of,
            )
            .optional()
            .map_err(|e| failed("cannot be read", e))
    }

    // The transaction borrows the connection until it commits, which is the
    // last thing the write does: the guard cannot go sooner.
    #[allow(clippy::significant_drop_tightening)]
    fn enroll(
        &self,
        root: &Path,
        name: &str,
        slug: &str,
        integration_branch: &str,
    ) -> Outcome<Factory> {
        let root_text = root.to_string_lossy().into_owned();
        let id = {
            let mut conn = self
                .conn
                .lock()
                .map_err(|_| failed("is poisoned", "a writer panicked"))?;
            // Immediate: the id is picked and written under one write lock, so
            // two views enrolling at once cannot pick the same one.
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| failed("cannot be written", e))?;
            let known: Option<String> = tx
                .query_row(
                    "SELECT id FROM factories WHERE root = ?1",
                    params![root_text],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| failed("cannot be read", e))?;
            let id = if let Some(id) = known {
                tx.execute(
                    "UPDATE factories SET name = ?2, slug = ?3, integration_branch = ?4 WHERE id = ?1",
                    params![id, name, slug, integration_branch],
                )
                .map_err(|e| failed("cannot be written", e))?;
                id
            } else {
                let base = id_from(name);
                let mut id = base.clone();
                let mut n = 1_u32;
                while tx
                    .query_row("SELECT 1 FROM factories WHERE id = ?1", params![id], |_| {
                        Ok(())
                    })
                    .optional()
                    .map_err(|e| failed("cannot be read", e))?
                    .is_some()
                {
                    n = n.saturating_add(1);
                    id = format!("{base}-{n}");
                }
                tx.execute(
                    "INSERT INTO factories (id, name, slug, root, integration_branch) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, name, slug, root_text, integration_branch],
                )
                .map_err(|e| failed("cannot be written", e))?;
                id
            };
            tx.commit().map_err(|e| failed("cannot be written", e))?;
            id
        };
        Ok(Factory {
            id,
            name: name.to_string(),
            slug: slug.to_string(),
            root: root.to_path_buf(),
            integration_branch: integration_branch.to_string(),
        })
    }

    fn creation(&self, id: &str) -> Outcome<Option<String>> {
        self.conn
            .lock()
            .map_err(|_| failed("is poisoned", "a writer panicked"))?
            .query_row(
                "SELECT record FROM creations WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| failed("cannot be read", e))
    }

    fn keep_creation(&self, id: &str, record: &str) -> Outcome<()> {
        self.conn
            .lock()
            .map_err(|_| failed("is poisoned", "a writer panicked"))?
            .execute(
                "INSERT INTO creations (id, record) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET record = excluded.record",
                params![id, record],
            )
            .map(|_| ())
            .map_err(|e| failed("cannot be written", e))
    }

    fn end_creation(&self, id: &str) -> Outcome<()> {
        self.conn
            .lock()
            .map_err(|_| failed("is poisoned", "a writer panicked"))?
            .execute("DELETE FROM creations WHERE id = ?1", params![id])
            .map(|_| ())
            .map_err(|e| failed("cannot be written", e))
    }

    // The transaction borrows the connection until it commits, which is the
    // last thing the write does: the guard cannot go sooner.
    #[allow(clippy::significant_drop_tightening)]
    fn forget(&self, id: &str) -> Outcome<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| failed("is poisoned", "a writer panicked"))?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| failed("cannot be written", e))?;
        for table in ["creations", "factories"] {
            tx.execute(&format!("DELETE FROM {table} WHERE id = ?1"), params![id])
                .map_err(|e| failed("cannot be written", e))?;
        }
        tx.commit().map_err(|e| failed("cannot be written", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("harness-factories-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn register(dir: &Dir) -> SqliteFactories {
        SqliteFactories::open(&dir.0.join("factories.db")).expect("opened")
    }

    #[test]
    fn an_enrolled_folder_reads_back_by_id_and_in_the_list() {
        let dir = Dir::new("read");
        let store = register(&dir);
        let made = store
            .enroll(Path::new("/work/a"), "Alpha", "me/alpha", "main_agent")
            .expect("enrolled");
        assert_eq!(made.id, "alpha");
        assert_eq!(store.get("alpha").expect("read"), Some(made.clone()));
        assert_eq!(store.get("beta").expect("read"), None);
        assert_eq!(store.list().expect("read"), vec![made]);
    }

    #[test]
    fn the_list_is_by_name_whatever_the_case() {
        let dir = Dir::new("order");
        let store = register(&dir);
        for (root, name) in [("/w/c", "charlie"), ("/w/a", "Alpha"), ("/w/b", "bravo")] {
            store
                .enroll(Path::new(root), name, "", "main_agent")
                .expect("enrolled");
        }
        let names: Vec<String> = store
            .list()
            .expect("read")
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, ["Alpha", "bravo", "charlie"]);
    }

    #[test]
    fn a_known_folder_keeps_its_id_and_takes_what_changed() {
        let dir = Dir::new("again");
        let store = register(&dir);
        let first = store
            .enroll(Path::new("/work/a"), "alpha", "", "main_agent")
            .expect("enrolled");
        let again = store
            .enroll(Path::new("/work/a"), "renamed", "me/alpha", "main")
            .expect("enrolled");
        assert_eq!(again.id, first.id);
        let kept = store.get(&first.id).expect("read").expect("known");
        assert_eq!(
            (
                kept.name.as_str(),
                kept.slug.as_str(),
                kept.integration_branch.as_str()
            ),
            ("renamed", "me/alpha", "main")
        );
        assert_eq!(store.list().expect("read").len(), 1);
    }

    #[test]
    fn two_folders_with_one_name_get_two_ids() {
        let dir = Dir::new("clash");
        let store = register(&dir);
        let a = store
            .enroll(Path::new("/one/app"), "app", "", "main_agent")
            .expect("enrolled");
        let b = store
            .enroll(Path::new("/two/app"), "app", "", "main_agent")
            .expect("enrolled");
        let c = store
            .enroll(Path::new("/three/app"), "app", "", "main_agent")
            .expect("enrolled");
        assert_eq!(
            (a.id.as_str(), b.id.as_str(), c.id.as_str()),
            ("app", "app-2", "app-3")
        );
    }

    #[test]
    fn a_creation_is_kept_replaced_and_forgotten_by_factory() {
        let dir = Dir::new("creation");
        let store = register(&dir);
        assert_eq!(store.creation("alpha").expect("read"), None);
        store.keep_creation("alpha", "one").expect("kept");
        store.keep_creation("beta", "other").expect("kept");
        store.keep_creation("alpha", "two").expect("kept");
        assert_eq!(
            store.creation("alpha").expect("read").as_deref(),
            Some("two")
        );
        store.end_creation("alpha").expect("ended");
        assert_eq!(store.creation("alpha").expect("read"), None);
        assert_eq!(
            register(&dir).creation("beta").expect("read").as_deref(),
            Some("other")
        );
    }

    #[test]
    fn a_forgotten_factory_leaves_the_register_with_its_creation_and_no_other() {
        let dir = Dir::new("forget");
        let store = register(&dir);
        let alpha = store
            .enroll(Path::new("/work/a"), "alpha", "me/alpha", "main_agent")
            .expect("enrolled");
        let beta = store
            .enroll(Path::new("/work/b"), "beta", "", "main_agent")
            .expect("enrolled");
        store.keep_creation(&alpha.id, "one").expect("kept");
        store.keep_creation(&beta.id, "other").expect("kept");

        store.forget(&alpha.id).expect("forgotten");

        assert_eq!(store.get(&alpha.id).expect("read"), None);
        assert_eq!(store.creation(&alpha.id).expect("read"), None);
        assert_eq!(store.get(&beta.id).expect("read"), Some(beta.clone()));
        assert_eq!(
            store.creation(&beta.id).expect("read").as_deref(),
            Some("other")
        );
        // Gone for good: a reopened register does not bring it back.
        assert_eq!(register(&dir).list().expect("read"), vec![beta]);
    }

    #[test]
    fn forgetting_a_factory_nobody_knows_is_not_an_error() {
        let dir = Dir::new("forget-none");
        register(&dir).forget("ghost").expect("nothing to forget");
    }

    #[test]
    fn a_forgotten_folder_can_be_enrolled_again_under_a_fresh_id() {
        let dir = Dir::new("forget-again");
        let store = register(&dir);
        let first = store
            .enroll(Path::new("/work/a"), "alpha", "", "main_agent")
            .expect("enrolled");
        store.forget(&first.id).expect("forgotten");
        let again = store
            .enroll(Path::new("/work/a"), "alpha", "", "main_agent")
            .expect("enrolled");
        assert_eq!(again.id, "alpha");
        assert_eq!(store.list().expect("read").len(), 1);
    }

    #[test]
    fn a_register_reopened_still_knows_its_factories() {
        let dir = Dir::new("reopen");
        let id = register(&dir)
            .enroll(Path::new("/work/a"), "alpha", "", "main_agent")
            .expect("enrolled")
            .id;
        assert!(register(&dir).get(&id).expect("read").is_some());
    }
}
