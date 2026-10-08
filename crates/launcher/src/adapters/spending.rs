//! The ledger, wired: the clock, the run ID, the machine.
//!
//! The [`Spending`] port implementation lives here, not in `harness-core`,
//! and that is deliberate. `Ledger` knows how to write a line but not what
//! time it is; a ledger line receives its timestamp rather than reading it,
//! which makes it testable. The three facts that are missing — the timestamp,
//! the run name, the machine name — are facts of the **launcher**, and putting
//! them here is what leaves `harness-core` with no dependency on time.

use std::path::{Path, PathBuf};

use harness_core::adapters::shell::process;
use harness_core::adapters::store::ledger::{Ledger, Row};
use harness_core::domain::{Halt, Outcome, quota};
use harness_core::ports::store::spending::{Entry, Spending};

/// The ledger for this run.
pub struct LedgerSpending {
    ledger: Ledger,
    run: String,
    host: String,
    /// Where the latest rate-limit reading is kept, when it is kept at all.
    quota: Option<PathBuf>,
}

impl LedgerSpending {
    /// The ledger at this path, for this run, on this machine.
    #[must_use]
    pub fn new(path: &Path, run: &str, host: &str) -> Self {
        Self {
            ledger: Ledger::new(path),
            run: run.to_string(),
            host: host.to_string(),
            quota: None,
        }
    }

    /// Also keep the carrier's last rate-limit reading at this path.
    ///
    /// Separate from [`Self::new`] so a caller with no use for it — `init-repo`,
    /// every test — says nothing rather than passing `None`.
    #[must_use]
    pub fn keeping_quota_at(mut self, at: PathBuf) -> Self {
        self.quota = Some(at);
        self
    }
}

impl Spending for LedgerSpending {
    /// Overwritten, never appended: only the latest reading means anything, and
    /// a file that grew by a line per turn would need parsing to answer what one
    /// object answers.
    fn remember_quota(&self, reading: &quota::Reading) -> Outcome<()> {
        let Some(at) = self.quota.as_ref() else {
            return Ok(());
        };
        let text = serde_json::to_string(reading)
            .map_err(|e| Halt::Failed(format!("a rate-limit reading would not serialise: {e}")))?;
        if let Some(parent) = at.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Halt::Failed(format!("{} cannot be created: {e}", parent.display()))
            })?;
        }
        std::fs::write(at, text)
            .map_err(|e| Halt::Failed(format!("{} cannot be written: {e}", at.display())))
    }

    fn record(&self, entry: &Entry<'_>) -> Outcome<()> {
        self.ledger.append(&Row {
            when: now(),
            run: self.run.clone(),
            round: entry.round,
            task: entry.task.to_string(),
            stage: entry.stage.to_string(),
            spend: entry.spend.clone(),
            ran_on: self.host.clone(),
            outcome: entry.outcome.to_string(),
            fingerprint: entry.fingerprint.to_string(),
        })
    }

    fn failures(&self, task: &str, stage: &str, fingerprint: &str) -> Outcome<u32> {
        self.ledger.failures(task, stage, fingerprint)
    }
}

/// The timestamp, in ISO-8601 UTC to the second.
///
/// The **format** does not change, so the frozen header holds and old lines
/// remain readable; only the value changes, and a new repository has no
/// history to contradict.
#[must_use]
pub fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// A run ID: the timestamp, compacted.
///
/// It names the log directory, the `run` column of the ledger, and a
/// disposable workspace. Readable by design — it's what a human types to find
/// the traces of a run.
#[must_use]
pub fn run_id() -> String {
    jiff::Timestamp::now().strftime("%Y%m%d-%H%M%S").to_string()
}

/// A run ID for one issue's run: the timestamp, then the issue.
///
/// Lanes of a parallel `watch` start in the same second; on the bare
/// timestamp their runs would write one log directory between them, and the
/// view would show one run where several are going. The suffix is also how
/// the view names the agent after the issue it works.
#[must_use]
pub fn run_id_for(issue: u64) -> String {
    format!("{}-{issue}", run_id())
}

/// The machine name, or `?`.
///
/// Via the `hostname` binary and the process adapter: it's an external call,
/// and external calls go through `adapters`. Its absence does not stop anything
/// — it's a comfort column, not data that a decision depends on.
pub async fn hostname() -> String {
    let said = process::run("hostname", &[], Path::new(".")).await;
    match said {
        Ok(out) if out.ok() && !out.out().is_empty() => out.out().to_string(),
        _ => "?".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::adapters::store::ledger::{self, HEADER};
    use harness_core::domain::{Spend, Tokens};

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("harness-spending-{tag}"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a test directory");
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_timestamp_has_the_shape_the_frozen_header_expects() {
        let stamp = now();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'), "{stamp}");
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[10..11], "T");
    }

    #[test]
    fn a_run_id_is_readable_because_a_human_types_it_to_find_the_logs() {
        let id = run_id();
        assert_eq!(id.len(), 15, "{id}");
        assert_eq!(&id[8..9], "-");
    }

    #[test]
    fn a_recorded_spend_lands_on_the_frozen_columns() {
        let dir = Dir::new("columns");
        let path = dir.0.join("costs.tsv");
        let spending = LedgerSpending::new(&path, "20261002-120000", "le-mac");
        let spend = Spend {
            cost_usd: Some(1.5),
            turns: Some(7),
            tokens: Tokens {
                input: Some(1000),
                ..Tokens::default()
            },
            ..Spend::default()
        };
        spending
            .record(&Entry {
                round: 2,
                task: "11",
                stage: "code",
                spend: &spend,
                outcome: "ok",
                fingerprint: "0123456789abcdef",
            })
            .expect("written");
        let text = std::fs::read_to_string(&path).expect("re-read");
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some(HEADER));
        let row: Vec<&str> = lines.next().expect("a line").split('\t').collect();
        let columns = ledger::columns();
        assert_eq!(row.len(), columns.len());
        let at = |name: &str| row[columns.iter().position(|c| *c == name).expect(name)];
        assert_eq!(at("run"), "20261002-120000");
        assert_eq!(at("round"), "02", "zero-padded, like the log tag");
        assert_eq!(at("task"), "11");
        assert_eq!(at("stage"), "code");
        assert_eq!(at("cost_usd"), "1.500000");
        assert_eq!(at("ran_on"), "le-mac");
        assert_eq!(at("outcome"), "ok");
        assert_eq!(at("fingerprint"), "0123456789abcdef");
        // Unobserved remains empty: a column of zeros would read as a free
        // session.
        assert_eq!(at("out"), "");
        assert_eq!(at("session"), "");
    }

    #[test]
    fn the_same_prompt_failing_three_times_is_what_the_ledger_answers() {
        // What the breaker reads before paying. The whole round trip: the
        // rows this adapter wrote are the rows it counts.
        let dir = Dir::new("failures");
        let path = dir.0.join("costs.tsv");
        let spending = LedgerSpending::new(&path, "20261002-120000", "le-mac");
        for _ in 0..3 {
            spending
                .record(&Entry {
                    round: 1,
                    task: "31",
                    stage: "code",
                    spend: &Spend::default(),
                    outcome: harness_core::domain::breaker::STOP,
                    fingerprint: "abcabcabcabcabca",
                })
                .expect("written");
        }
        assert_eq!(
            spending
                .failures("31", "code", "abcabcabcabcabca")
                .expect("read"),
            3
        );
        assert_eq!(
            spending
                .failures("31", "code", "a-different-prompt")
                .expect("read"),
            0
        );
    }

    #[test]
    fn a_ledger_with_no_file_yet_knows_of_no_failure() {
        let dir = Dir::new("empty");
        let spending = LedgerSpending::new(&dir.0.join("costs.tsv"), "r", "h");
        assert_eq!(spending.failures("31", "code", "abc").expect("read"), 0);
    }

    #[tokio::test]
    async fn a_hostname_that_cannot_be_read_does_not_stop_a_run() {
        // A comfort column, not data that a decision depends on.
        assert_ne!(hostname().await, "");
    }
}
