//! The notifications of the moment: the events' and the facts', together.

use harness_core::ports::store::events::Stored;
use harness_core::traces::Event;

use crate::facts::Facts;
use crate::notification::{Level, Link, Notification};
use crate::rules;

/// How much of a Claude window spent makes a warning, and an error.
const WINDOW_WARNING: f64 = 0.8;
const WINDOW_ERROR: f64 = 0.95;

/// What to tell now, loudest first, then newest: the notifications the
/// `events` tell, oldest event first, and those the `facts` say.
#[must_use]
pub fn feed(events: &[Stored], facts: &Facts) -> Vec<Notification> {
    let mut board = rules::from_events(events);
    // A watch that is not running, and did not say it stopped, was killed or
    // crashed: the plant stopped on its own.
    let last_life = events.iter().rev().find_map(|stored| match &stored.event {
        Event::WatchStarted { .. } | Event::WatchDraining { .. } => Some((false, &stored.at)),
        Event::WatchStopped => Some((true, &stored.at)),
        _ => None,
    });
    if facts.watch_running == Some(false)
        && let Some((false, since)) = last_life
    {
        board.clear("watch");
        board.add(Notification::once(
            "watch-dead",
            Level::Error,
            "The watch is not running",
            "it ended without a soft stop: it crashed, or was killed (a hard stop does that). \
             Nothing new starts until it runs again.",
            &facts.now_at,
            Some(Link::Screen {
                screen: "journal".to_string(),
                at: Some(since.clone()),
            }),
        ));
    }
    for waiting in &facts.waiting {
        board.add(Notification::once(
            format!("human:#{}", waiting.number),
            Level::Warning,
            format!("#{} needs you", waiting.number),
            format!("{} — {}", waiting.title, waiting.label),
            &facts.now_at,
            Some(Link::Issue {
                number: waiting.number,
            }),
        ));
    }
    if let Some(window) = facts
        .claude
        .as_ref()
        .and_then(|reading| reading.tightest(facts.now))
        && window.utilization >= WINDOW_WARNING
    {
        let error = window.utilization >= WINDOW_ERROR;
        board.add(Notification::once(
            format!("window:{}", window.name),
            if error { Level::Error } else { Level::Warning },
            format!(
                "Claude's {} window is {}% spent",
                window.name,
                (window.utilization * 100.0).round()
            ),
            "sessions stop when it runs out; the Rate limits screen says when it resets",
            &facts.now_at,
            Some(Link::Screen {
                screen: "quota".to_string(),
                at: None,
            }),
        ));
    }
    board.into_sorted()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::Waiting;
    use harness_core::domain::quota::{Reading, Window};
    use proptest::prelude::*;

    fn stored(minute: u32, event: Event) -> Stored {
        Stored {
            id: u64::from(minute),
            at: format!("2026-10-09T00:{minute:02}:00Z"),
            source: "watch".to_string(),
            event,
        }
    }

    fn facts() -> Facts {
        Facts {
            now: 1_000,
            now_at: "2026-10-09T01:00:00Z".to_string(),
            ..Facts::default()
        }
    }

    fn started() -> Stored {
        stored(
            1,
            Event::WatchStarted {
                interval: 30,
                target: "t".to_string(),
                journal: "j".to_string(),
            },
        )
    }

    #[test]
    fn a_watch_gone_without_a_word_is_an_error_and_a_stopped_one_is_not() {
        let gone = Facts {
            watch_running: Some(false),
            ..facts()
        };
        let all = feed(&[started()], &gone);
        assert_eq!(all[0].key, "watch-dead");
        assert_eq!(all[0].level, Level::Error);
        let stopped = feed(&[started(), stored(2, Event::WatchStopped)], &gone);
        assert!(stopped.iter().all(|n| n.key != "watch-dead"));
        let unknown = feed(&[started()], &facts());
        assert!(
            unknown.iter().all(|n| n.key != "watch-dead"),
            "nobody looked"
        );
    }

    #[test]
    fn an_issue_waiting_on_a_human_is_a_warning_that_leads_to_it() {
        let waiting = Facts {
            waiting: vec![Waiting {
                number: 31,
                title: "Provision the read-only login".to_string(),
                label: "harness:human".to_string(),
            }],
            ..facts()
        };
        let all = feed(&[], &waiting);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].level, Level::Warning);
        assert_eq!(all[0].link, Some(Link::Issue { number: 31 }));
    }

    #[test]
    fn a_nearly_spent_window_warns_then_errs() {
        let reading = |used: f64| Reading {
            windows: vec![Window {
                name: "five_hour".to_string(),
                utilization: used,
                resets_at: 5_000,
            }],
            at: 900,
        };
        let at = |used: f64| {
            feed(
                &[],
                &Facts {
                    claude: Some(reading(used)),
                    ..facts()
                },
            )
        };
        assert!(at(0.5).is_empty());
        assert_eq!(at(0.85)[0].level, Level::Warning);
        assert_eq!(at(0.97)[0].level, Level::Error);
    }

    proptest! {
        #[test]
        fn any_failures_never_make_more_than_one_notification_per_task(codes in proptest::collection::vec(0i32..4, 0..40)) {
            let events: Vec<Stored> = codes
                .iter()
                .enumerate()
                .map(|(i, code)| stored(
                    u32::try_from(i % 60).unwrap_or(0),
                    Event::LaneEnded { lane: 0, issue: 15, status: format!("exit status: {code}"), code: Some(*code) },
                ))
                .collect();
            let all = feed(&events, &facts());
            prop_assert!(all.iter().filter(|n| n.key == "failing:#15").count() <= 1);
        }
    }
}
