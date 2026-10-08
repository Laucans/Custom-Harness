//! The traces as they sit on disk — the one module of the view that opens a
//! file under `.llocal/logs`.
//!
//! Reuses `harness-core`'s own readers for the two ledgers, so the view
//! cannot disagree with the harness about which column is which. Everything
//! else is a plain file the launcher appends to.

use std::fs::{self, File};
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::SystemTime;

use harness_core::adapters::store::{error_ledger, ledger};
use harness_core::domain::quota::Reading;
use harness_core::domain::workspace::Workspace;

use crate::domain::traces::is_run_id;
use crate::ports::{ErrorRow, LedgerRow, Traces};

/// The traces of one harness checkout.
pub struct FsTraces {
    workspace: Workspace,
}

impl FsTraces {
    /// The traces under this workspace's `.llocal/logs`.
    #[must_use]
    pub const fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }

    fn run_dir(&self, workflow: &str, run: &str) -> PathBuf {
        self.workspace.log_dir(workflow).join(run)
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The last `max_bytes` of a file, the first partial line dropped.
fn tail_of(path: &Path, max_bytes: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = lossy(&bytes);
    if start == 0 {
        return Some(text);
    }
    Some(
        text.split_once('\n')
            .map_or_else(String::new, |(_, rest)| rest.to_string()),
    )
}

/// The first `max_bytes` of a file.
fn head_of(path: &Path, max_bytes: u64) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(max_bytes).read_to_end(&mut bytes).ok()?;
    Some(lossy(&bytes))
}

/// The cell under `name`, trimmed, or empty when the row is short.
fn cell(cells: &[&str], columns: &[&str], name: &str) -> String {
    columns
        .iter()
        .position(|column| *column == name)
        .and_then(|at| cells.get(at))
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}

/// The cell under `name` as a number, or `None` when empty or not one.
fn number<T: FromStr>(cells: &[&str], columns: &[&str], name: &str) -> Option<T> {
    cell(cells, columns, name).parse().ok()
}

impl Traces for FsTraces {
    fn runs(&self, workflow: &str) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.workspace.log_dir(workflow)) else {
            return Vec::new();
        };
        let mut runs: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| is_run_id(name))
            .collect();
        runs.sort();
        runs
    }

    fn read(&self, workflow: &str, run: &str, file: &str) -> Option<String> {
        fs::read(self.run_dir(workflow, run).join(file))
            .ok()
            .map(|bytes| lossy(&bytes))
    }

    fn tail(&self, workflow: &str, run: &str, file: &str, max_bytes: u64) -> Option<String> {
        tail_of(&self.run_dir(workflow, run).join(file), max_bytes)
    }

    fn head(&self, workflow: &str, run: &str, file: &str, max_bytes: u64) -> Option<String> {
        head_of(&self.run_dir(workflow, run).join(file), max_bytes)
    }

    fn alive(&self, workflow: &str, run: &str) -> Option<bool> {
        let lock = fs::File::open(
            self.run_dir(workflow, run)
                .join(harness_core::traces::RUN_ALIVE),
        )
        .ok()?;
        // A shared lock taken and dropped at once: it only succeeds when no
        // live process holds the run's exclusive one.
        match lock.try_lock_shared() {
            Ok(()) => Some(false),
            Err(fs::TryLockError::WouldBlock) => Some(true),
            Err(fs::TryLockError::Error(_)) => None,
        }
    }

    fn age_secs(&self, workflow: &str, run: &str) -> Option<u64> {
        let entries = fs::read_dir(self.run_dir(workflow, run)).ok()?;
        let newest = entries
            .flatten()
            .filter_map(|entry| entry.metadata().ok()?.modified().ok())
            .max()?;
        // A file stamped in the future reads as written just now.
        Some(
            SystemTime::now()
                .duration_since(newest)
                .map_or(0, |age| age.as_secs()),
        )
    }

    fn watch_log(&self, max_bytes: u64) -> Option<String> {
        tail_of(&self.workspace.loop_dir().join("watch.log"), max_bytes)
    }

    fn ledger(&self) -> Vec<LedgerRow> {
        let rows = match ledger::Ledger::new(&self.workspace.ledger()).rows() {
            Ok(rows) => rows,
            Err(halt) => {
                tracing::warn!("cost ledger unreadable: {}", halt.reason());
                return Vec::new();
            }
        };
        let columns = ledger::columns();
        rows.iter()
            .map(|row| {
                let cells: Vec<&str> = row.iter().map(String::as_str).collect();
                LedgerRow {
                    when: cell(&cells, &columns, "when"),
                    run: cell(&cells, &columns, "run"),
                    round: cell(&cells, &columns, "round"),
                    task: cell(&cells, &columns, "task"),
                    stage: cell(&cells, &columns, "stage"),
                    cost_usd: number(&cells, &columns, "cost_usd"),
                    turns: number(&cells, &columns, "turns"),
                    duration_ms: number(&cells, &columns, "duration_ms"),
                    input: number(&cells, &columns, "in"),
                    output: number(&cells, &columns, "out"),
                    cache_read: number(&cells, &columns, "cache_read"),
                    cache_write: number(&cells, &columns, "cache_write"),
                    outcome: cell(&cells, &columns, "outcome"),
                }
            })
            .collect()
    }

    fn errors(&self) -> Vec<ErrorRow> {
        let Ok(text) = fs::read_to_string(self.workspace.error_ledger()) else {
            return Vec::new();
        };
        let columns = error_ledger::columns();
        text.lines()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let cells: Vec<&str> = line.split('\t').collect();
                ErrorRow {
                    when: cell(&cells, &columns, "when"),
                    run: cell(&cells, &columns, "run"),
                    workflow: cell(&cells, &columns, "workflow"),
                    kind: cell(&cells, &columns, "kind"),
                    reason: cell(&cells, &columns, "reason"),
                }
            })
            .collect()
    }

    fn quota(&self) -> Option<Reading> {
        fs::read_to_string(self.workspace.quota())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A checkout of our own, cleaned up at the end. The real disk, not a
    /// fake: this module *is* the filesystem.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "harness-view-{name}-{}-{}",
                std::process::id(),
                jiff::Timestamp::now().as_nanosecond()
            ));
            fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(path, text).expect("write");
        }

        fn traces(&self) -> FsTraces {
            FsTraces::new(Workspace::new(&self.0))
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn runs_are_the_run_folders_sorted_and_nothing_else() {
        let dir = Dir::new("runs");
        dir.write(".llocal/logs/agent-loop/20261006-202608/run.log", "x");
        dir.write(".llocal/logs/agent-loop/20261006-150000/run.log", "x");
        dir.write(".llocal/logs/agent-loop/costs.tsv", "x");
        dir.write(".llocal/logs/agent-loop/flow-20261007-055756.jsonl", "x");
        fs::create_dir_all(dir.0.join(".llocal/logs/agent-loop/notarun")).expect("mkdir");
        assert_eq!(
            dir.traces().runs("agent-loop"),
            ["20261006-150000", "20261006-202608"]
        );
        assert!(dir.traces().runs("planner").is_empty());
    }

    #[test]
    fn a_tail_cuts_at_a_line_boundary_and_a_head_does_not() {
        let dir = Dir::new("tail");
        dir.write(
            ".llocal/logs/split/20261006-152700/session.log",
            "first line\nsecond line\nthird line\n",
        );
        let traces = dir.traces();
        let tail = traces
            .tail("split", "20261006-152700", "session.log", 16)
            .expect("tail");
        assert_eq!(tail, "third line\n");
        let whole = traces
            .tail("split", "20261006-152700", "session.log", 1024)
            .expect("tail");
        assert!(whole.starts_with("first line"));
        let head = traces
            .head("split", "20261006-152700", "session.log", 5)
            .expect("head");
        assert_eq!(head, "first");
        assert!(
            traces
                .tail("split", "20261006-152700", "nope", 16)
                .is_none()
        );
    }

    #[test]
    fn age_is_measured_on_the_newest_file_of_the_run() {
        let dir = Dir::new("age");
        dir.write(".llocal/logs/agent-loop/20261006-202608/run.log", "x");
        let age = dir
            .traces()
            .age_secs("agent-loop", "20261006-202608")
            .expect("age");
        assert!(age < 5, "{age}");
        assert!(
            dir.traces()
                .age_secs("agent-loop", "20261006-000000")
                .is_none()
        );
    }

    #[test]
    fn the_ledgers_are_read_through_the_frozen_headers() {
        let dir = Dir::new("ledgers");
        dir.write(
            ".llocal/logs/agent-loop/costs.tsv",
            &format!(
                "{}\n2026-10-06T15:01:07Z\tr1\t01\t50\tcode\t0.833390\t17\t114905\t26\t12867\tsess\thost\t1353878\t108473\tSTOP\tf846\n\
                 2026-10-06T15:21:14Z\tr2\t01\t\tcode\t\t\t\t\t\tsess\thost\t\t\tok\n",
                ledger::HEADER
            ),
        );
        dir.write(
            ".llocal/logs/agent-loop/errors.tsv",
            &format!(
                "{}\n2026-10-06T15:01:07Z\tr1\tdev_loop\tSTOP\tthe workspace was deleted\n",
                error_ledger::HEADER
            ),
        );
        dir.write(
            ".llocal/logs/agent-loop/quota.json",
            r#"{"windows":[{"name":"five_hour","utilization":0.58,"resets_at":1791394800}],"at":1791380801}"#,
        );
        let traces = dir.traces();
        let rows = traces.ledger();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].task, "50");
        assert_eq!(rows[0].cost_usd, Some(0.833_39));
        assert_eq!(rows[0].cache_read, Some(1_353_878));
        assert_eq!(rows[0].outcome, "STOP");
        assert_eq!(rows[1].cost_usd, None, "empty is not observed, not zero");
        assert_eq!(rows[1].outcome, "ok");
        let errors = traces.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, "STOP");
        assert_eq!(errors[0].workflow, "dev_loop");
        let quota = traces.quota().expect("quota");
        assert_eq!(quota.windows[0].name, "five_hour");
    }

    #[test]
    fn a_missing_checkout_reads_as_nothing_rather_than_failing() {
        let dir = Dir::new("empty");
        let traces = dir.traces();
        assert!(traces.ledger().is_empty());
        assert!(traces.errors().is_empty());
        assert!(traces.quota().is_none());
        assert!(traces.watch_log(1024).is_none());
        assert!(
            traces
                .read("agent-loop", "20261006-202608", "run.log")
                .is_none()
        );
    }
}
