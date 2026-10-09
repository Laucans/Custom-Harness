//! From the harness's events to what a human should hear.
//!
//! Each rule names its key, so the same subject told again counts up rather
//! than piling up, and an event that ends a subject clears it: a task that
//! succeeds is no longer "failing".

use harness_core::ports::store::events::Stored;
use harness_core::traces::Event;

use crate::notification::{Board, Level, Link, Notification};

/// How many failed lane runs in a row make a task an error rather than a
/// warning: one can be a flake, three is a loop.
pub const FAILING_LOOP: u32 = 3;

const fn issue(number: u64) -> Link {
    Link::Issue { number }
}

fn screen(name: &str, at: &str) -> Link {
    Link::Screen {
        screen: name.to_string(),
        at: Some(at.to_string()),
    }
}

/// The board of notifications the events tell, oldest event first.
#[must_use]
pub fn from_events(events: &[Stored]) -> Board {
    let mut board = Board::default();
    for stored in events {
        tell(&mut board, &stored.event, &stored.at);
    }
    board
}

fn tell(board: &mut Board, event: &Event, at: &str) {
    match event {
        Event::WatchStarted { target, .. } => {
            board.clear("watch");
            watch_said(board, "The watch started", format!("polling {target}"), at);
        }
        Event::WatchDraining { lanes } => watch_said(
            board,
            "The watch is stopping",
            format!(
                "soft stop: no new task; {} lane(s) still finishing",
                lanes.len()
            ),
            at,
        ),
        Event::WatchStopped => watch_said(
            board,
            "The watch stopped",
            "every lane finished; start it again from the status button".to_string(),
            at,
        ),
        Event::TickFailed { reason } => board.add(Notification::once(
            "tick-failed",
            Level::Error,
            "The watch cannot read the board",
            reason.clone(),
            at,
            Some(screen("journal", at)),
        )),
        Event::LaneNotStarted {
            issue: n, reason, ..
        } => board.add(Notification::once(
            format!("not-started:#{n}"),
            Level::Error,
            format!("A lane for #{n} cannot start"),
            reason.clone(),
            at,
            Some(issue(*n)),
        )),
        Event::LaneEnded {
            issue: n,
            status,
            code,
            ..
        } => lane_ended(board, *n, status, *code, at),
        Event::TaskParked { task } => board.add(Notification::once(
            format!("parked:#{task}"),
            Level::Warning,
            format!("#{task} is waiting for a human"),
            "its lane stopped for a question, a gate, or the breaker; it is not taken again \
             until its issue changes",
            at,
            Some(issue(*task)),
        )),
        Event::BreakerRefused {
            task,
            stage,
            failures,
        } => board.add(Notification::once(
            format!("breaker:#{task}:{stage}"),
            Level::Warning,
            format!("Not paying again for /{stage} on #{task}"),
            format!(
                "the same prompt already failed {failures} times in a row; change the issue \
                 and it runs again"
            ),
            at,
            task.parse().ok().map(issue),
        )),
        Event::SessionEnded {
            outcome,
            stage,
            task,
            ..
        } if outcome == "QUOTA" => {
            board.add(Notification::once(
                "quota",
                Level::Error,
                "Claude's quota is spent",
                format!("/{stage} on #{task} was refused; work resumes when the window resets"),
                at,
                Some(screen("quota", at)),
            ));
        }
        Event::Halted {
            workflow,
            kind,
            reason,
        } => halted(board, workflow, kind, reason, at),
        // A gate's verdict is the line's business, not a sign: a halt that matters
        // arrives as the `Halted` event that follows it.
        Event::SessionEnded { .. }
        | Event::PreparationTaken { .. }
        | Event::LaneTaken { .. }
        | Event::GateChecked { .. } => {}
    }
}

/// The watch's life is one line, telling its last state.
fn watch_said(board: &mut Board, title: &str, detail: String, at: &str) {
    board.add(Notification::once(
        "watch",
        Level::Info,
        title,
        detail,
        at,
        Some(screen("journal", at)),
    ));
}

fn lane_ended(board: &mut Board, n: u64, status: &str, code: Option<i32>, at: &str) {
    let key = format!("failing:#{n}");
    match code {
        // Done well: whatever was failing is over.
        Some(0) => board.clear(&key),
        // Stopped for a human: the parked notification says it.
        Some(1) => {}
        None => board.add(Notification::once(
            format!("killed:#{n}"),
            Level::Warning,
            format!("The lane of #{n} was killed"),
            format!("{status}; the session it ran is lost and is paid again on its next run"),
            at,
            Some(issue(n)),
        )),
        Some(_) => {
            let runs = board.get(&key).map_or(0, |known| known.count) + 1;
            let level = if runs >= FAILING_LOOP {
                Level::Error
            } else {
                Level::Warning
            };
            board.add(Notification::once(
                key,
                level,
                if runs >= FAILING_LOOP {
                    format!("#{n} keeps failing")
                } else {
                    format!("A run of #{n} failed")
                },
                format!("{runs} failed run(s) in a row, the last with {status}"),
                at,
                Some(issue(n)),
            ));
        }
    }
}

fn halted(board: &mut Board, workflow: &str, kind: &str, reason: &str, at: &str) {
    match kind {
        // The watch's own halt is a failed read, and `tick-failed` says it.
        _ if workflow == "watch" => {}
        "QUOTA" => board.add(Notification::once(
            "quota",
            Level::Error,
            "Claude's quota is spent",
            reason.to_string(),
            at,
            Some(screen("quota", at)),
        )),
        "FAILED" => board.add(Notification::once(
            format!("failed:{workflow}"),
            Level::Error,
            format!("{workflow} failed"),
            reason.to_string(),
            at,
            Some(screen("errors", at)),
        )),
        // A lane's dev loop that stops is parked, and that one says it.
        _ if workflow.starts_with("dev_loop") => {}
        _ => board.add(Notification::once(
            format!("stopped:{workflow}"),
            Level::Warning,
            format!("{workflow} stopped for a human"),
            reason.to_string(),
            at,
            Some(screen("errors", at)),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(n: u32) -> String {
        format!("2026-10-09T00:{n:02}:00Z")
    }

    fn stored(minute: u32, event: Event) -> Stored {
        Stored {
            id: u64::from(minute),
            at: at(minute),
            source: "watch".to_string(),
            event,
        }
    }

    fn ended(minute: u32, code: Option<i32>) -> Stored {
        stored(
            minute,
            Event::LaneEnded {
                lane: 0,
                issue: 15,
                status: code
                    .map_or_else(|| "signal: 9".to_string(), |c| format!("exit status: {c}")),
                code,
            },
        )
    }

    #[test]
    fn a_task_failing_again_and_again_becomes_an_error() {
        let one = from_events(&[ended(1, Some(2))]).into_sorted();
        assert_eq!(
            (one[0].key.as_str(), one[0].level),
            ("failing:#15", Level::Warning)
        );
        let loop_ =
            from_events(&[ended(1, Some(2)), ended(2, Some(2)), ended(3, Some(2))]).into_sorted();
        assert_eq!(loop_.len(), 1);
        assert_eq!((loop_[0].level, loop_[0].count), (Level::Error, 3));
        assert_eq!(loop_[0].title, "#15 keeps failing");
        assert_eq!(loop_[0].link, Some(Link::Issue { number: 15 }));
    }

    #[test]
    fn a_success_clears_the_failures() {
        let board = from_events(&[ended(1, Some(2)), ended(2, Some(2)), ended(3, Some(0))]);
        assert!(board.get("failing:#15").is_none());
    }

    #[test]
    fn a_stop_for_a_human_is_one_warning_not_two() {
        let events = [
            ended(1, Some(1)),
            stored(1, Event::TaskParked { task: 15 }),
            stored(
                1,
                Event::Halted {
                    workflow: "dev_loop #15".to_string(),
                    kind: "STOP".to_string(),
                    reason: "refusing to pay".to_string(),
                },
            ),
        ];
        let all = from_events(&events).into_sorted();
        let keys: Vec<&str> = all.iter().map(|n| n.key.as_str()).collect();
        assert_eq!(keys, ["parked:#15"]);
        assert_eq!(all[0].level, Level::Warning);
    }

    #[test]
    fn a_failed_read_is_told_once_not_as_a_halt_too() {
        let events = [
            stored(
                1,
                Event::TickFailed {
                    reason: "no remote".to_string(),
                },
            ),
            stored(
                1,
                Event::Halted {
                    workflow: "watch".to_string(),
                    kind: "STOP".to_string(),
                    reason: "no remote".to_string(),
                },
            ),
        ];
        let keys: Vec<String> = from_events(&events)
            .into_sorted()
            .into_iter()
            .map(|n| n.key)
            .collect();
        assert_eq!(keys, ["tick-failed"]);
    }

    #[test]
    fn a_spent_quota_and_a_blind_watch_are_errors() {
        let events = [
            stored(
                1,
                Event::Halted {
                    workflow: "dev_loop #15".to_string(),
                    kind: "QUOTA".to_string(),
                    reason: "five_hour exhausted".to_string(),
                },
            ),
            stored(
                2,
                Event::TickFailed {
                    reason: "gh: API rate limit exceeded".to_string(),
                },
            ),
            stored(
                3,
                Event::TickFailed {
                    reason: "gh: API rate limit exceeded".to_string(),
                },
            ),
        ];
        let all = from_events(&events).into_sorted();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|n| n.level == Level::Error));
        let blind = all.iter().find(|n| n.key == "tick-failed").expect("tick");
        assert_eq!(blind.count, 2);
    }

    #[test]
    fn the_watch_s_life_is_one_line_that_tells_its_last_state() {
        let events = [
            stored(
                1,
                Event::WatchStarted {
                    interval: 30,
                    target: "Laucans/dnd_helper2".to_string(),
                    journal: "x".to_string(),
                },
            ),
            stored(2, Event::WatchDraining { lanes: vec![15] }),
            stored(3, Event::WatchStopped),
        ];
        let all = from_events(&events).into_sorted();
        assert_eq!(all.len(), 1);
        assert_eq!(
            (all[0].title.as_str(), all[0].level),
            ("The watch stopped", Level::Info)
        );
    }
}
