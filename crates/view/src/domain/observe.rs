//! One read of everything the plant shows, through the `Traces` port.
//!
//! Reads the latest run of every line and nothing older: what the view
//! answers is "where is the plant now", and the ledgers already carry the
//! history. The parsing is `traces`'; this module only decides what to read.

use harness_core::domain::quota::Reading;

use crate::domain::blueprint::Line;
use crate::domain::traces::{
    RunLog, Turn, Watch, model_from_stream_head, parse_prompt_headers, parse_run_log, parse_watch,
};
use crate::ports::{ErrorRow, LedgerRow, Traces};

/// How much of `watch.log` is read back: enough for the last tick and its
/// warnings, not the whole night.
const WATCH_TAIL_BYTES: u64 = 64 * 1024;

/// How many journal lines the picture carries.
const RECENT_LINES: usize = 40;

/// The first event of a stream names the model; it fits in this much.
const STREAM_HEAD_BYTES: u64 = 4 * 1024;

/// The last line of a session log is somewhere in this much.
const SESSION_TAIL_BYTES: u64 = 2 * 1024;

/// The latest run of a line, as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservedRun {
    /// The run id.
    pub run_id: String,
    /// Its `run.log`, parsed.
    pub log: RunLog,
    /// Its `prompts.md` headers.
    pub turns: Vec<Turn>,
    /// Seconds since it last wrote.
    pub age_secs: Option<u64>,
    /// The model family its stream announced.
    pub model_seen: Option<String>,
    /// The last line of its `session.log`.
    pub session_tail: String,
}

/// One line, as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservedLine {
    /// The log folder name.
    pub workflow: String,
    /// How many runs it has logged.
    pub runs: u32,
    /// The latest one.
    pub latest: Option<ObservedRun>,
}

/// Everything one tick reads.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observed {
    /// The watch journal.
    pub watch: Watch,
    /// One per blueprint line, in the same order.
    pub lines: Vec<ObservedLine>,
    /// The cost ledger.
    pub ledger: Vec<LedgerRow>,
    /// The error ledger.
    pub errors: Vec<ErrorRow>,
    /// The last rate-limit reading.
    pub quota: Option<Reading>,
}

fn observe_run(traces: &dyn Traces, line: &Line, run: &str) -> ObservedRun {
    let known: Vec<String> = line
        .stations
        .iter()
        .filter_map(|station| station.stage.clone())
        .collect();
    let log = traces
        .read(&line.id, run, "run.log")
        .map(|text| parse_run_log(&text, &known))
        .unwrap_or_default();
    let turns = traces
        .read(&line.id, run, "prompts.md")
        .map(|text| parse_prompt_headers(&text))
        .unwrap_or_default();
    let model_seen = traces
        .head(&line.id, run, "stream.jsonl", STREAM_HEAD_BYTES)
        .and_then(|head| model_from_stream_head(&head));
    let session_tail = traces
        .tail(&line.id, run, "session.log", SESSION_TAIL_BYTES)
        .and_then(|text| {
            text.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_default();
    ObservedRun {
        run_id: run.to_string(),
        log,
        turns,
        age_secs: traces.age_secs(&line.id, run),
        model_seen,
        session_tail,
    }
}

/// Reads the plant once.
#[must_use]
pub fn observe(traces: &dyn Traces, lines: &[Line]) -> Observed {
    let watch = traces
        .watch_log(WATCH_TAIL_BYTES)
        .map(|text| parse_watch(&text, RECENT_LINES))
        .unwrap_or_default();
    let lines = lines
        .iter()
        .map(|line| {
            let runs = traces.runs(&line.id);
            ObservedLine {
                workflow: line.id.clone(),
                runs: u32::try_from(runs.len()).unwrap_or(u32::MAX),
                latest: runs.last().map(|run| observe_run(traces, line, run)),
            }
        })
        .collect();
    Observed {
        watch,
        lines,
        ledger: traces.ledger(),
        errors: traces.errors(),
        quota: traces.quota(),
    }
}

#[cfg(test)]
pub mod fake {
    //! An in-memory `Traces`: what a test puts in reads back.

    use std::collections::HashMap;

    use super::*;

    /// Files keyed by `(workflow, run, file)`, plus the loose ones.
    #[derive(Default)]
    pub struct Shelf {
        pub files: HashMap<(String, String, String), String>,
        pub ages: HashMap<(String, String), u64>,
        pub watch: Option<String>,
        pub ledger: Vec<LedgerRow>,
        pub errors: Vec<ErrorRow>,
        pub quota: Option<Reading>,
    }

    impl Shelf {
        pub fn put(&mut self, workflow: &str, run: &str, file: &str, text: &str) -> &mut Self {
            self.files.insert(
                (workflow.to_string(), run.to_string(), file.to_string()),
                text.to_string(),
            );
            self
        }

        pub fn age(&mut self, workflow: &str, run: &str, secs: u64) -> &mut Self {
            self.ages
                .insert((workflow.to_string(), run.to_string()), secs);
            self
        }

        fn get(&self, workflow: &str, run: &str, file: &str) -> Option<String> {
            self.files
                .get(&(workflow.to_string(), run.to_string(), file.to_string()))
                .cloned()
        }
    }

    impl Traces for Shelf {
        fn runs(&self, workflow: &str) -> Vec<String> {
            let mut runs: Vec<String> = self
                .files
                .keys()
                .filter(|(w, _, _)| w == workflow)
                .map(|(_, run, _)| run.clone())
                .collect();
            runs.sort();
            runs.dedup();
            runs
        }

        fn read(&self, workflow: &str, run: &str, file: &str) -> Option<String> {
            self.get(workflow, run, file)
        }

        fn tail(&self, workflow: &str, run: &str, file: &str, _max: u64) -> Option<String> {
            self.get(workflow, run, file)
        }

        fn head(&self, workflow: &str, run: &str, file: &str, _max: u64) -> Option<String> {
            self.get(workflow, run, file)
        }

        fn age_secs(&self, workflow: &str, run: &str) -> Option<u64> {
            self.ages
                .get(&(workflow.to_string(), run.to_string()))
                .copied()
        }

        fn watch_log(&self, _max: u64) -> Option<String> {
            self.watch.clone()
        }

        fn ledger(&self) -> Vec<LedgerRow> {
            self.ledger.clone()
        }

        fn errors(&self) -> Vec<ErrorRow> {
            self.errors.clone()
        }

        fn quota(&self) -> Option<Reading> {
            self.quota.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Shelf;
    use super::*;
    use crate::domain::blueprint;

    #[test]
    fn only_the_latest_run_of_each_line_is_read() {
        let mut shelf = Shelf::default();
        shelf
            .put(
                "agent-loop",
                "20261006-150000",
                "run.log",
                "[2026-10-06T15:00:00Z] old\n",
            )
            .put(
                "agent-loop",
                "20261006-202608",
                "run.log",
                "[2026-10-06T20:26:21Z] task #62: Asset folder rule [auto]\n",
            )
            .put(
                "agent-loop",
                "20261006-202608",
                "prompts.md",
                "## turn 1 — /tech-analyst (opus/high)\n",
            )
            .put(
                "agent-loop",
                "20261006-202608",
                "stream.jsonl",
                r#"{"type":"system","model":"claude-opus-5-5"}"#,
            )
            .put(
                "agent-loop",
                "20261006-202608",
                "session.log",
                "[  0m01s] → Bash: ls\n\n",
            )
            .age("agent-loop", "20261006-202608", 12);
        shelf.watch = Some("[2026-10-06T20:26:00Z] tick: DevLoop { milestone: 17 }\n".to_string());

        let observed = observe(&shelf, &blueprint::lines());
        let dev = observed
            .lines
            .iter()
            .find(|l| l.workflow == "agent-loop")
            .expect("agent-loop");
        assert_eq!(dev.runs, 2);
        let latest = dev.latest.as_ref().expect("latest");
        assert_eq!(latest.run_id, "20261006-202608");
        assert_eq!(latest.log.task.as_ref().map(|t| t.number), Some(62));
        assert_eq!(latest.turns.len(), 1);
        assert_eq!(latest.model_seen.as_deref(), Some("opus"));
        assert_eq!(latest.session_tail, "[  0m01s] → Bash: ls");
        assert_eq!(latest.age_secs, Some(12));
        assert_eq!(
            observed
                .watch
                .in_flight
                .as_ref()
                .map(|f| f.workflow.as_str()),
            Some("agent-loop")
        );
        let split = observed
            .lines
            .iter()
            .find(|l| l.workflow == "split")
            .expect("split");
        assert_eq!(split.runs, 0);
        assert!(split.latest.is_none());
    }
}
