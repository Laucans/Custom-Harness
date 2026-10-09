//! What happened, as data: the events a run or the watch records once, at the
//! moment they happen.
//!
//! An event is written **once**: [`Logbook::event`](super::Logbook::event)
//! records it and writes its [`Event::line`] to the journal. The prose line is
//! a reading of the event, not a second account of it, so the two cannot drift
//! apart. Some events have no line — a session's end is already a row of the
//! cost ledger — and are only recorded ([`Logbook::record`](super::Logbook::record)).
//!
//! The variants are stored by name (`event`), so a variant is never renamed: a
//! reader meets every event ever written. A new fact is a new variant.

use serde::{Deserialize, Serialize};

/// How bad an event's line is — what the journal writes at its head, and
/// what the view colours it by.
///
/// Not `domain::Severity`: `traces` imports nothing else from core. The two
/// agree — a `STOP` or a `QUOTA` is waited on, only a `FAILED` is a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The plant at work: no prefix.
    Info,
    /// Something went wrong and the run goes on: `warning: `.
    Warning,
    /// Something failed or stopped: `error: `.
    Error,
}

/// One thing that happened in the plant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The watch started polling.
    WatchStarted {
        /// Its `--interval`, in seconds.
        interval: u64,
        /// The repository it watches, as it names it.
        target: String,
        /// Its journal, relative to the checkout.
        journal: String,
    },
    /// A soft stop was asked: no new task, the running lanes are waited for.
    WatchDraining {
        /// The tasks still on a lane.
        lanes: Vec<u64>,
    },
    /// The watch stopped after a soft stop.
    WatchStopped,
    /// A poll could not read the board.
    TickFailed {
        /// Why, in the read's own words.
        reason: String,
    },
    /// A lane took a task of the dev loop.
    LaneTaken {
        /// The lane.
        lane: u32,
        /// The task.
        task: u64,
        /// The milestone branch it works on.
        branch: String,
        /// The lane's process.
        pid: u32,
    },
    /// A lane took a milestone to split or an issue to refine.
    PreparationTaken {
        /// The lane.
        lane: u32,
        /// `refine` or `split`.
        what: String,
        /// The issue.
        issue: u64,
        /// The lane's process.
        pid: u32,
    },
    /// A lane could not even start.
    LaneNotStarted {
        /// `refine`, `split`, or `None` for a dev-loop task.
        what: Option<String>,
        /// The issue.
        issue: u64,
        /// Why.
        reason: String,
    },
    /// A lane's process ended.
    LaneEnded {
        /// The lane.
        lane: u32,
        /// The issue it was on.
        issue: u64,
        /// The exit, as the OS reports it (`exit status: 1`).
        status: String,
        /// The exit code; `None` when a signal ended it.
        code: Option<i32>,
    },
    /// A task whose lane stopped for a human is not taken again until its
    /// issue changes.
    TaskParked {
        /// The task.
        task: u64,
    },
    /// A paid session ended. Recorded only: the cost ledger is its line.
    SessionEnded {
        /// The task, empty for a round that worked none.
        task: String,
        /// The stage.
        stage: String,
        /// `ok`, `STOP`, `FAILED`, `QUOTA`.
        outcome: String,
        /// Claude's estimate, when it gave one.
        cost_usd: Option<f64>,
    },
    /// The breaker refused to pay for a prompt that already failed.
    /// Recorded only: the halt that follows says it.
    BreakerRefused {
        /// The task, empty for none.
        task: String,
        /// The stage.
        stage: String,
        /// How many times in a row the same prompt failed.
        failures: u32,
    },
    /// A workflow stopped. Told in the run's own log as `STOP: …` — the line
    /// a reader of that log jumps to — and kept.
    Halted {
        /// The workflow, or the task a lane ran.
        workflow: String,
        /// `STOP`, `FAILED`, `QUOTA`, or what stands for an unreadable input.
        kind: String,
        /// Why, in the run's own words.
        reason: String,
    },
}

impl Event {
    /// The journal line that tells this event to a human — `None` for an
    /// event whose line is a ledger row instead.
    #[must_use]
    pub fn line(&self) -> Option<String> {
        let tasks = |list: &[u64]| {
            list.iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        Some(match self {
            Self::WatchStarted {
                interval,
                target,
                journal,
            } => format!("watch: every {interval}s on {target} — journal {journal}"),
            Self::WatchDraining { lanes } => format!(
                "watch: draining — soft stop asked, no new task; waiting for {} lane(s){}",
                lanes.len(),
                if lanes.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", tasks(lanes))
                }
            ),
            Self::WatchStopped => "watch: stopped — soft stop, every lane done".to_string(),
            Self::TickFailed { reason } => format!("watch: tick -> {reason}"),
            Self::LaneTaken {
                lane,
                task,
                branch,
                pid,
            } => format!("watch: lanes -> lane {lane} takes #{task} on {branch} (pid {pid})"),
            Self::PreparationTaken {
                lane,
                what,
                issue,
                pid,
            } => format!("watch: router lanes -> lane {lane} {what}s #{issue} (pid {pid})"),
            Self::LaneNotStarted {
                what: None,
                issue,
                reason,
            } => format!("watch: lanes -> cannot start #{issue}: {reason}"),
            Self::LaneNotStarted {
                what: Some(what),
                issue,
                reason,
            } => format!("watch: router lanes -> cannot start {what} #{issue}: {reason}"),
            Self::LaneEnded {
                lane,
                issue,
                status,
                ..
            } => format!("watch: lanes -> lane {lane} done with #{issue} ({status})"),
            Self::TaskParked { task } => format!(
                "watch: lanes -> #{task} parked: it stopped for a human, and is not taken \
                 again until its issue changes"
            ),
            Self::Halted { kind, reason, .. } => format!("{kind}: {reason}"),
            Self::SessionEnded { .. } | Self::BreakerRefused { .. } => return None,
        })
    }

    /// How bad its line is.
    ///
    /// A failed board read is retried at the next tick, a `STOP` waits for a
    /// human, a `QUOTA` for the window: warnings. A `FAILED`, a lane that
    /// could not start, or that ended on a signal or on an exit code other
    /// than 0, 1 (`STOP`) and 3 (`QUOTA`) — the frozen contract — is an error.
    #[must_use]
    pub fn level(&self) -> Level {
        match self {
            Self::Halted { kind, .. } if kind == "FAILED" => Level::Error,
            Self::LaneEnded { code: Some(0), .. } => Level::Info,
            Self::TickFailed { .. }
            | Self::Halted { .. }
            | Self::LaneEnded {
                code: Some(1 | 3), ..
            } => Level::Warning,
            Self::LaneNotStarted { .. } | Self::LaneEnded { .. } => Level::Error,
            _ => Level::Info,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_are_the_journal_s_own() {
        // The view reads `watch.log`: an event's line is exactly the line the
        // watch wrote before events existed.
        let taken = Event::LaneTaken {
            lane: 0,
            task: 15,
            branch: "milestone/3-x".to_string(),
            pid: 61956,
        };
        assert_eq!(
            taken.line().as_deref(),
            Some("watch: lanes -> lane 0 takes #15 on milestone/3-x (pid 61956)")
        );
        let refine = Event::PreparationTaken {
            lane: 1,
            what: "refine".to_string(),
            issue: 4,
            pid: 2,
        };
        assert_eq!(
            refine.line().as_deref(),
            Some("watch: router lanes -> lane 1 refines #4 (pid 2)")
        );
        let ended = Event::LaneEnded {
            lane: 0,
            issue: 15,
            status: "exit status: 1".to_string(),
            code: Some(1),
        };
        assert_eq!(
            ended.line().as_deref(),
            Some("watch: lanes -> lane 0 done with #15 (exit status: 1)")
        );
        let draining = Event::WatchDraining {
            lanes: vec![15, 16],
        };
        assert_eq!(
            draining.line().as_deref(),
            Some(
                "watch: draining — soft stop asked, no new task; waiting for 2 lane(s) (#15, #16)"
            )
        );
    }

    #[test]
    fn a_ledger_event_has_no_line_and_a_stop_warns() {
        let ended = Event::SessionEnded {
            task: "15".to_string(),
            stage: "code".to_string(),
            outcome: "ok".to_string(),
            cost_usd: None,
        };
        assert_eq!(ended.line(), None);
        let halted = Event::Halted {
            workflow: "dev_loop #15".to_string(),
            kind: "STOP".to_string(),
            reason: "refusing to pay".to_string(),
        };
        assert_eq!(halted.line().as_deref(), Some("STOP: refusing to pay"));
        assert_eq!(halted.level(), Level::Warning, "a STOP waits for a human");
        let failed = Event::Halted {
            workflow: "dev_loop #15".to_string(),
            kind: "FAILED".to_string(),
            reason: "the session printed nothing".to_string(),
        };
        assert_eq!(failed.level(), Level::Error);
        assert_eq!(
            Event::TickFailed {
                reason: "x".to_string()
            }
            .level(),
            Level::Warning
        );
        assert_eq!(Event::WatchStopped.level(), Level::Info);
    }

    #[test]
    fn a_lane_is_an_error_when_it_failed_or_was_killed() {
        let ended = |code| Event::LaneEnded {
            lane: 0,
            issue: 15,
            status: String::new(),
            code,
        };
        assert_eq!(ended(Some(0)).level(), Level::Info);
        assert_eq!(ended(Some(1)).level(), Level::Warning, "STOP");
        assert_eq!(ended(Some(3)).level(), Level::Warning, "QUOTA");
        assert_eq!(ended(Some(2)).level(), Level::Error, "FAILED");
        assert_eq!(ended(None).level(), Level::Error, "killed by a signal");
        let refused = Event::LaneNotStarted {
            what: None,
            issue: 15,
            reason: "no such file".to_string(),
        };
        assert_eq!(refused.level(), Level::Error);
    }

    #[test]
    fn an_event_is_stored_by_its_name() {
        let json = serde_json::to_string(&Event::TaskParked { task: 15 }).expect("json");
        assert_eq!(json, r#"{"event":"task_parked","task":15}"#);
        let back: Event = serde_json::from_str(&json).expect("back");
        assert_eq!(back, Event::TaskParked { task: 15 });
    }
}
