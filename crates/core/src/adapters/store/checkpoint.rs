//! Where the harness is, on disk — the implementation of [`Checkpoints`].
//!
//! Two files, and it is deliberate:
//!
//! - a **pointer** of two lines (`task=`, `flow_id=`), readable with `cat` and
//!   editable by hand. It exists because an unreadable state is an
//!   undebuggable state;
//! - the **states**, one JSONL file per flow, one line per step.
//!
//! **What is not ported: sqlite.** The Python schema
//! (`flow_states(flow_uuid, method_name, timestamp, state_json)`) existed only
//! by inheritance from a `@persist` graph engine, and that engine died before
//! the migration. JSONL keeps the property that mattered — one line per step,
//! so a failed resume stays readable afterwards — without dragging a C
//! dependency for a file opened twice per round.
//!
//! What the port promises, and why an unreadable store is never "nothing ran
//! yet", is documented with it in
//! [`ports::store::checkpoint`](crate::ports::store::checkpoint).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::domain::{Halt, Outcome};
use crate::ports::store::checkpoint::{Checkpoints, Pointer};

/// The pointer, as it's written: two lines, readable with `cat`.
#[must_use]
fn render_pointer(task: &str, flow_id: &str) -> String {
    format!("task={task}\nflow_id={flow_id}\n")
}

/// The pointer, as it's re-read.
///
/// A line we don't understand is ignored: the file is meant to be hand-edited,
/// and a typo must not stop a run.
#[must_use]
fn parse_pointer(text: &str) -> Pointer {
    let mut found = Pointer::default();
    for line in text.lines() {
        if let Some(task) = line.strip_prefix("task=") {
            found.task = Some(task.trim().to_string()).filter(|t| !t.is_empty());
        } else if let Some(flow) = line.strip_prefix("flow_id=") {
            found.flow_id = Some(flow.trim().to_string()).filter(|f| !f.is_empty());
        }
    }
    found
}

/// The resume store, in a directory.
pub struct Checkpoint {
    dir: PathBuf,
}

impl Checkpoint {
    /// The store in this directory. Nothing is created until the first write.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    fn pointer_path(&self) -> PathBuf {
        self.dir.join("state")
    }

    /// One file per flow. The ID is sanitized: it comes from a workflow,
    /// and a `../` in it would write outside the store.
    fn flow_path(&self, flow_id: &str) -> PathBuf {
        let safe: String = flow_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("flow-{safe}.jsonl"))
    }
}

impl Checkpoints for Checkpoint {
    fn pointer(&self) -> Outcome<Pointer> {
        let path = self.pointer_path();
        if !path.exists() {
            return Ok(Pointer::default());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| unreadable(&path, &e.to_string()))?;
        Ok(parse_pointer(&text))
    }

    /// Overwritten, never appended: a resume point is a state, not a history.
    fn set_pointer(&self, task: &str, flow_id: &str) -> Outcome<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| wrote_nothing(&self.dir, &e.to_string()))?;
        let path = self.pointer_path();
        std::fs::write(&path, render_pointer(task, flow_id))
            .map_err(|e| wrote_nothing(&path, &e.to_string()))
    }

    /// An already-absent pointer is not a failure: the desired end state is
    /// reached.
    fn clear(&self) -> Outcome<()> {
        let path = self.pointer_path();
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(wrote_nothing(&path, &e.to_string())),
        }
    }

    /// One line per step rather than an update: [`Checkpoints::load`] reads the
    /// last, and keeping the previous ones makes a failed resume readable
    /// afterwards.
    fn save(&self, flow_id: &str, step: &str, state: &Value) -> Outcome<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| wrote_nothing(&self.dir, &e.to_string()))?;
        let path = self.flow_path(flow_id);
        let line = serde_json::json!({ "step": step, "state": state });
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| wrote_nothing(&path, &e.to_string()))?;
        writeln!(file, "{line}").map_err(|e| wrote_nothing(&path, &e.to_string()))
    }

    /// The last non-empty line wins; an empty file is "nothing ran yet", a line
    /// that will not decode is [`Halt::Unreadable`].
    fn load(&self, flow_id: &str) -> Outcome<Option<Value>> {
        if flow_id.is_empty() {
            return Ok(None);
        }
        let path = self.flow_path(flow_id);
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path).map_err(|e| unreadable(&path, &e.to_string()))?;
        let Some(last) = text.lines().rfind(|line| !line.trim().is_empty()) else {
            // Empty file: no one has written a step yet.
            return Ok(None);
        };
        let record: Value =
            serde_json::from_str(last).map_err(|e| unreadable(&path, &e.to_string()))?;
        record
            .get("state")
            .cloned()
            .map(Some)
            .ok_or_else(|| unreadable(&path, "last line has no `state` field"))
    }
}

/// What we say when we don't know where the harness is.
fn unreadable(path: &Path, detail: &str) -> Halt {
    Halt::Unreadable(format!(
        "resume state unreadable at {} ({detail}) — which stages have already run is unknown, \
         and reading it as « none » would make us repay a stage that may have already merged. \
         Inspect or delete the file, or relaunch forcing this task's resume from the start.",
        path.display()
    ))
}

fn wrote_nothing(path: &Path, detail: &str) -> Halt {
    Halt::Failed(format!(
        "couldn't write {} ({detail}) — the harness couldn't resume where it stopped",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A directory of our own, cleaned up at the end. The real disk, not a fake:
    /// this module *is* the filesystem, and a fake wouldn't prove anything.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("harness-checkpoint-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }

        fn store(&self) -> Checkpoint {
            Checkpoint::new(&self.0)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // --- pointer ---

    #[test]
    fn the_pointer_stays_two_readable_lines() {
        assert_eq!(render_pointer("42", "flow-9"), "task=42\nflow_id=flow-9\n");
    }

    #[test]
    fn a_pointer_round_trips() {
        let dir = Dir::new("pointer");
        let store = dir.store();
        store.set_pointer("42", "flow-9").expect("write");
        assert_eq!(
            store.pointer().expect("read back"),
            Pointer {
                task: Some("42".to_string()),
                flow_id: Some("flow-9".to_string()),
            }
        );
    }

    #[test]
    fn no_pointer_yet_is_an_empty_pointer_not_an_error() {
        let dir = Dir::new("absent");
        assert_eq!(dir.store().pointer().expect("absent"), Pointer::default());
    }

    #[test]
    fn a_hand_edited_pointer_survives_a_stray_line() {
        // The file is meant to be hand-edited: a typo must not stop a run.
        let found = parse_pointer("task=42\nan unexpected line\nflow_id=f1\n");
        assert_eq!(found.task.as_deref(), Some("42"));
        assert_eq!(found.flow_id.as_deref(), Some("f1"));
    }

    #[test]
    fn an_empty_value_reads_as_absent_rather_than_as_an_empty_task() {
        assert_eq!(parse_pointer("task=\nflow_id=\n"), Pointer::default());
    }

    #[test]
    fn clearing_a_pointer_that_is_already_gone_is_not_an_error() {
        let dir = Dir::new("clear");
        let store = dir.store();
        store.clear().expect("already absent");
        store.set_pointer("1", "f").expect("write");
        store.clear().expect("delete");
        assert_eq!(store.pointer().expect("read back"), Pointer::default());
    }

    // --- states ---

    #[test]
    fn the_last_step_written_is_the_one_that_comes_back() {
        let dir = Dir::new("states");
        let store = dir.store();
        store
            .save("f1", "pick-task", &json!({ "stages_done": [] }))
            .expect("first step");
        store
            .save(
                "f1",
                "business-analyst",
                &json!({ "stages_done": ["business-analyst"] }),
            )
            .expect("second step");

        let state = store.load("f1").expect("read back").expect("a state");
        assert_eq!(state, json!({ "stages_done": ["business-analyst"] }));
    }

    #[test]
    fn every_step_is_kept_so_a_failed_resume_stays_readable() {
        let dir = Dir::new("history");
        let store = dir.store();
        store.save("f1", "a", &json!({ "n": 1 })).expect("a");
        store.save("f1", "b", &json!({ "n": 2 })).expect("b");
        let raw = std::fs::read_to_string(dir.0.join("flow-f1.jsonl")).expect("read");
        assert_eq!(raw.lines().count(), 2);
    }

    #[test]
    fn nothing_has_run_yet_reads_as_none_in_all_its_harmless_forms() {
        let dir = Dir::new("nothing");
        let store = dir.store();
        // Missing store, unknown flow, empty ID: three ways to say the same harmless thing.
        assert!(store.load("f1").expect("absent").is_none());
        assert!(store.load("").expect("empty").is_none());
        store.save("f1", "a", &json!({})).expect("a");
        assert!(store.load("unknown").expect("other flow").is_none());
    }

    #[test]
    fn a_store_that_does_not_decode_is_unreadable_never_nothing_ran() {
        // Prevents the failure mode: repaying a /code that's already merged.
        let dir = Dir::new("corrupt");
        let store = dir.store();
        std::fs::create_dir_all(&dir.0).expect("mkdir");
        std::fs::write(dir.0.join("flow-f1.jsonl"), "this is not json\n").expect("write");
        let err = store.load("f1").expect_err("must fail");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[test]
    fn a_record_without_a_state_field_is_unreadable_too() {
        let dir = Dir::new("nostate");
        let store = dir.store();
        std::fs::create_dir_all(&dir.0).expect("mkdir");
        std::fs::write(dir.0.join("flow-f1.jsonl"), "{\"step\":\"a\"}\n").expect("write");
        assert!(matches!(
            store.load("f1").expect_err("must fail"),
            Halt::Unreadable(_)
        ));
    }

    #[test]
    fn an_empty_store_file_is_nothing_ran_not_unreadable() {
        let dir = Dir::new("emptyfile");
        let store = dir.store();
        std::fs::create_dir_all(&dir.0).expect("mkdir");
        std::fs::write(dir.0.join("flow-f1.jsonl"), "").expect("write");
        assert!(store.load("f1").expect("empty").is_none());
    }

    #[test]
    fn a_flow_id_cannot_escape_the_store_directory() {
        // The ID comes from a workflow: a `../` in it would write elsewhere.
        let dir = Dir::new("escape");
        let store = dir.store();
        let path = store.flow_path("../../elsewhere");
        assert_eq!(path.parent(), Some(dir.0.as_path()));
        assert!(!path.display().to_string().contains(".."));
    }
}
