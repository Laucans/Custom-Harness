//! The watch loop, read from `watch.log`: what it triggered and how each one
//! ended, and how many of its ticks found nothing to do.
//!
//! A tick opens with `saw:` (a full read of the board) or `quiet:` (nothing
//! moved, no read), and its route follows as `tick: <Route>`. What it
//! triggers is either a **lane** — `lanes -> lane k takes #n`, `router lanes
//! -> lane k refines #n`, closed later by `lanes -> lane k done with #n
//! (exit status: c)` — or an **inline** workflow run inside the tick and
//! closed by `watch: <workflow> -> <outcome>`. A tick that triggers nothing
//! is an empty tick: a quiet one, a `Nothing` route, or a lane route whose
//! every task was already on a lane.

use serde::Serialize;

use crate::domain::traces::{Stamped, route_subject, stamped};

/// How many triggers the journal keeps, newest first.
pub const TRIGGERS_KEPT: usize = 60;

/// How a trigger stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Started, not reported back yet.
    Running,
    /// Ended well.
    Ok,
    /// Ended in error, or never started.
    Failed,
    /// Killed by a signal.
    Killed,
    /// The watch restarted before it reported back.
    Unknown,
}

/// One thing the watch started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Trigger {
    /// When it started.
    pub at: String,
    /// `dev loop`, `refinement`, `split`, `planner`, `pr review`, …
    pub what: String,
    /// The issue it works, when it names one.
    pub issue: Option<u64>,
    /// `pr 71`, `milestone 3`, … for an inline route that names no issue.
    pub subject: Option<String>,
    /// The lane it runs on; `None` inline.
    pub lane: Option<u32>,
    /// How it stands.
    pub state: State,
    /// The exit status, the outcome, or the error.
    pub detail: Option<String>,
    /// When it reported back.
    pub ended_at: Option<String>,
}

/// The loop's counts since the watch last started, and its triggers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Journal {
    /// When the watch last started — `None` when the start fell out of the
    /// part of the journal that was read, and the counts are a lower bound.
    pub since: Option<String>,
    /// Ticks since then.
    pub ticks: u32,
    /// Of those, the ones that triggered nothing.
    pub empty_ticks: u32,
    /// Of those, the ones whose read of the board failed.
    pub failed_ticks: u32,
    /// The last empty tick.
    pub last_empty_at: Option<String>,
    /// The triggers, newest first.
    pub triggers: Vec<Trigger>,
}

/// The tick under way.
#[derive(Default)]
struct Tick {
    route: Option<String>,
    triggered: bool,
}

/// The tag an inline workflow reports back under: `watch: pr_review -> …`.
fn tag(route: &str) -> &'static str {
    match route_name(route) {
        "DevLoop" => "dev_loop",
        "Refinement" | "TechRefinement" => "refinement",
        "Split" => "split",
        "Planner" => "planner",
        "PrReview" => "pr_review",
        "PrFix" => "pr_fix",
        "MergeMainAgent" => "main_agent_merge",
        "MergeIntoMilestone" => "milestone_merge",
        _ => "",
    }
}

fn route_name(route: &str) -> &str {
    route
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or_default()
}

/// A route whose work goes to lanes under `--parallel`.
fn lane_route(route: &str) -> bool {
    matches!(
        route_name(route),
        "DevLoop" | "Refinement" | "TechRefinement" | "Split"
    )
}

/// `dev_loop` → `dev loop`.
fn spoken(tag: &str) -> String {
    tag.replace('_', " ")
}

/// `#15 …` → 15.
fn issue_at(text: &str) -> Option<u64> {
    let digits: String = text
        .strip_prefix('#')?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// `exit status: 0` → ok; `exit status: 2` → failed; `signal: 9 …` → killed.
fn ended(status: &str) -> State {
    if status == "exit status: 0" {
        State::Ok
    } else if status.starts_with("signal") {
        State::Killed
    } else {
        State::Failed
    }
}

impl Journal {
    fn close_tick(&mut self, tick: &mut Option<Tick>, at: &str) {
        if let Some(done) = tick.take() {
            let inline = done
                .route
                .as_deref()
                .is_some_and(|route| route != "Nothing" && !lane_route(route));
            if !done.triggered && !inline {
                self.empty_ticks += 1;
                self.last_empty_at = Some(at.to_string());
            }
        }
    }

    fn running_mut(&mut self, matches: impl Fn(&Trigger) -> bool) -> Option<&mut Trigger> {
        self.triggers
            .iter_mut()
            .rev()
            .find(|t| t.state == State::Running && matches(t))
    }

    fn lane_line(&mut self, at: &str, body: &str, tick: &mut Option<Tick>, warned: bool) {
        // `lane 0 takes #15 on …`, `lane 1 refines #4 (pid …)`,
        // `lane 0 done with #15 (exit status: 1)`, `cannot start #15: …`.
        if let Some(rest) = body.strip_prefix("cannot start ") {
            let (what, issue_text) =
                rest.split_once(" #")
                    .map_or(("dev loop", rest), |(w, i)| match w {
                        "refine" => ("refinement", i),
                        other => (other, i),
                    });
            let issue_text = format!("#{}", issue_text.trim_start_matches('#'));
            self.triggers.push(Trigger {
                at: at.to_string(),
                what: what.to_string(),
                issue: issue_at(&issue_text),
                subject: None,
                lane: None,
                state: State::Failed,
                detail: issue_text.split_once(": ").map(|(_, e)| e.to_string()),
                ended_at: Some(at.to_string()),
            });
            if let Some(open) = tick.as_mut() {
                open.triggered = true;
            }
            return;
        }
        let Some(rest) = body.strip_prefix("lane ") else {
            return;
        };
        let Some((slot, rest)) = rest.split_once(' ') else {
            return;
        };
        let Ok(lane) = slot.parse::<u32>() else {
            return;
        };
        if let Some(done) = rest.strip_prefix("done with ") {
            let issue = issue_at(done);
            let status = done
                .split_once(" (")
                .map(|(_, s)| s.trim_end_matches(')').to_string());
            if let Some(open) = self.running_mut(|t| t.lane == Some(lane) && t.issue == issue) {
                open.state = status.as_deref().map_or(State::Unknown, ended);
                open.detail = status;
                open.ended_at = Some(at.to_string());
            }
            return;
        }
        if warned || rest.starts_with('(') {
            // `lane 0 (#15) cannot be waited on: …`
            if let Some(open) = self.running_mut(|t| t.lane == Some(lane)) {
                open.state = State::Unknown;
                open.detail = Some(rest.to_string());
                open.ended_at = Some(at.to_string());
            }
            return;
        }
        let Some((verb, target)) = rest.split_once(' ') else {
            return;
        };
        let what = match verb {
            "takes" => "dev loop".to_string(),
            "refines" => "refinement".to_string(),
            other => other.trim_end_matches('s').to_string(),
        };
        self.triggers.push(Trigger {
            at: at.to_string(),
            what,
            issue: issue_at(target),
            subject: None,
            lane: Some(lane),
            state: State::Running,
            detail: None,
            ended_at: None,
        });
        if let Some(open) = tick.as_mut() {
            open.triggered = true;
        }
    }

    fn reported(&mut self, at: &str, name: &str, outcome: &str, warned: bool, tick: Option<&Tick>) {
        let state = if warned { State::Failed } else { State::Ok };
        if let Some(open) = self.running_mut(|t| t.lane.is_none() && t.what == spoken(name)) {
            open.state = state;
            open.detail = Some(outcome.to_string());
            open.ended_at = Some(at.to_string());
            return;
        }
        // A lane route run inline (no `--parallel`): its row is born closed.
        let route = tick.and_then(|t| t.route.clone());
        if let Some(route) = route.filter(|r| tag(r) == name) {
            self.triggers.push(Trigger {
                at: at.to_string(),
                what: spoken(name),
                issue: None,
                subject: route_subject(&route),
                lane: None,
                state,
                detail: Some(outcome.to_string()),
                ended_at: Some(at.to_string()),
            });
        }
    }
}

/// Reads the loop out of `watch.log`.
#[must_use]
pub fn read(text: &str) -> Journal {
    let mut journal = Journal::default();
    let mut tick: Option<Tick> = None;
    for line in text.lines() {
        let Some(Stamped { at, rest }) = stamped(line) else {
            continue;
        };
        let (warned, rest) = rest
            .strip_prefix("warning: ")
            .map_or((false, rest), |r| (true, r));
        if rest.starts_with("watch: every ") {
            tick = None;
            for open in &mut journal.triggers {
                if open.state == State::Running {
                    open.state = State::Unknown;
                    open.detail = Some("the watch restarted before it reported back".to_string());
                }
            }
            journal.since = Some(at.to_string());
            journal.ticks = 0;
            journal.empty_ticks = 0;
            journal.failed_ticks = 0;
            journal.last_empty_at = None;
        } else if rest.starts_with("quiet:") {
            journal.close_tick(&mut tick, at);
            journal.ticks += 1;
            journal.empty_ticks += 1;
            journal.last_empty_at = Some(at.to_string());
        } else if rest.starts_with("saw: ") {
            journal.close_tick(&mut tick, at);
            journal.ticks += 1;
            tick = Some(Tick::default());
        } else if let Some(route) = rest.strip_prefix("tick: ") {
            if tick.as_ref().is_none_or(|t| t.route.is_some()) {
                // An older journal, without `saw:` lines.
                journal.close_tick(&mut tick, at);
                journal.ticks += 1;
                tick = Some(Tick::default());
            }
            if let Some(open) = tick.as_mut() {
                open.route = Some(route.to_string());
            }
            if route != "Nothing" && !lane_route(route) {
                journal.triggers.push(Trigger {
                    at: at.to_string(),
                    what: spoken(tag(route)),
                    issue: None,
                    subject: route_subject(route),
                    lane: None,
                    state: State::Running,
                    detail: None,
                    ended_at: None,
                });
            }
        } else if rest.starts_with("watch: tick -> ") {
            journal.close_tick(&mut tick, at);
            journal.ticks += 1;
            journal.failed_ticks += 1;
        } else if let Some(body) = rest
            .strip_prefix("watch: lanes -> ")
            .or_else(|| rest.strip_prefix("watch: router lanes -> "))
        {
            journal.lane_line(at, body, &mut tick, warned);
        } else if let Some(body) = rest.strip_prefix("watch: ")
            && let Some((name, outcome)) = body.split_once(" -> ")
            && !name.contains(' ')
            && name != "doctor"
        {
            journal.reported(at, name, outcome, warned, tick.as_ref());
        }
    }
    journal.triggers.reverse();
    journal.triggers.truncate(TRIGGERS_KEPT);
    journal
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Cut from the `dnd_helper2` run's own `watch.log`.
    const LOG: &str = "\
[2026-10-08T19:55:36Z] watch: every 30s on Laucans/dnd_helper2 — journal .llocal/logs/agent-loop/watch.log
[2026-10-08T19:55:40Z] saw: 1 roadmap, 0 milestone(s), 0 refining
[2026-10-08T19:55:40Z] tick: Planner { roadmap: 2 }
[2026-10-08T19:58:18Z] watch: planner -> Continue
[2026-10-08T19:58:56Z] saw: 1 roadmap, 10 milestone(s), 10 refining
[2026-10-08T19:58:56Z] tick: Refinement { issue: 3 }
[2026-10-08T19:59:29Z] watch: router lanes -> lane 0 refines #3 (pid 27200)
[2026-10-08T19:59:29Z] watch: router lanes -> lane 1 refines #4 (pid 27201)
[2026-10-08T19:59:29Z] watch: router lanes -> every lane is busy
[2026-10-08T20:00:07Z] saw: 1 roadmap, 10 milestone(s), 10 refining
[2026-10-08T20:00:07Z] tick: Refinement { issue: 3 }
[2026-10-08T20:00:32Z] watch: router lanes -> every lane is busy
[2026-10-08T20:00:40Z] quiet: nothing moved on the board — no snapshot read
[2026-10-08T20:05:00Z] watch: lanes -> lane 0 done with #3 (exit status: 0)
[2026-10-08T20:06:00Z] watch: lanes -> lane 1 done with #4 (exit status: 2)
[2026-10-08T20:06:30Z] warning: watch: tick -> impossible to read: sub-issues of #7 (gh: API rate limit exceeded)
[2026-10-08T20:07:00Z] saw: 1 roadmap, 10 milestone(s), 0 refining
[2026-10-08T20:07:00Z] tick: DevLoop { milestone: 3 }
[2026-10-08T20:07:07Z] watch: lanes -> lane 0 takes #15 on milestone/3-campaign (pid 25048)
[2026-10-08T20:07:08Z] warning: watch: lanes -> cannot start #16: No such file or directory (os error 2)
[2026-10-08T20:07:30Z] saw: 1 roadmap, 10 milestone(s), 0 refining
[2026-10-08T20:07:30Z] tick: PrReview { pr: \"29\", base: \"milestone/3\" }
[2026-10-08T20:09:00Z] warning: watch: pr_review -> the review session failed
[2026-10-08T20:10:00Z] quiet: nothing moved on the board — no snapshot read
";

    #[test]
    fn every_trigger_is_listed_newest_first_with_how_it_ended() {
        let journal = read(LOG);
        let rows: Vec<(&str, Option<u64>, Option<u32>, State)> = journal
            .triggers
            .iter()
            .map(|t| (t.what.as_str(), t.issue, t.lane, t.state))
            .collect();
        assert_eq!(
            rows,
            [
                ("pr review", None, None, State::Failed),
                ("dev loop", Some(16), None, State::Failed),
                ("dev loop", Some(15), Some(0), State::Running),
                ("refinement", Some(4), Some(1), State::Failed),
                ("refinement", Some(3), Some(0), State::Ok),
                ("planner", None, None, State::Ok),
            ]
        );
        let review = &journal.triggers[0];
        assert_eq!(review.subject.as_deref(), Some("pr 29"));
        assert_eq!(review.detail.as_deref(), Some("the review session failed"));
        assert_eq!(
            journal.triggers[3].detail.as_deref(),
            Some("exit status: 2")
        );
        assert_eq!(
            journal.triggers[3].ended_at.as_deref(),
            Some("2026-10-08T20:06:00Z")
        );
    }

    #[test]
    fn empty_ticks_are_counted_not_listed() {
        let journal = read(LOG);
        assert_eq!(journal.since.as_deref(), Some("2026-10-08T19:55:36Z"));
        // planner, refinement, refinement (nothing new), quiet, failed read,
        // dev loop, pr review, quiet.
        assert_eq!(journal.ticks, 8);
        assert_eq!(journal.empty_ticks, 3);
        assert_eq!(journal.failed_ticks, 1);
        assert_eq!(
            journal.last_empty_at.as_deref(),
            Some("2026-10-08T20:10:00Z")
        );
    }

    #[test]
    fn a_restart_forgets_the_counts_and_loses_what_was_running() {
        let text = format!(
            "{LOG}[2026-10-08T21:00:00Z] watch: every 30s on Laucans/dnd_helper2 — journal x\n"
        );
        let journal = read(&text);
        assert_eq!((journal.ticks, journal.empty_ticks), (0, 0));
        let lost = journal
            .triggers
            .iter()
            .find(|t| t.issue == Some(15))
            .expect("#15");
        assert_eq!(lost.state, State::Unknown);
    }

    #[test]
    fn a_killed_lane_reads_as_killed() {
        let text = "\
[2026-10-08T20:07:00Z] saw: x
[2026-10-08T20:07:00Z] tick: DevLoop { milestone: 3 }
[2026-10-08T20:07:07Z] watch: lanes -> lane 2 takes #15 on b (pid 1)
[2026-10-08T20:08:00Z] watch: lanes -> lane 2 done with #15 (signal: 9 (SIGKILL))
";
        let journal = read(text);
        assert_eq!(journal.triggers[0].state, State::Killed);
        assert_eq!(journal.since, None);
    }

    #[test]
    fn a_lane_route_run_inline_is_one_closed_row() {
        let text = "\
[2026-10-08T20:07:00Z] tick: DevLoop { milestone: 3 }
[2026-10-08T20:30:00Z] watch: dev_loop -> Continue
";
        let journal = read(text);
        assert_eq!(journal.triggers.len(), 1);
        let row = &journal.triggers[0];
        assert_eq!(row.what, "dev loop");
        assert_eq!(row.subject.as_deref(), Some("milestone 3"));
        assert_eq!(row.state, State::Ok);
        assert_eq!(journal.empty_ticks, 0);
    }

    proptest! {
        #[test]
        fn any_journal_reads_without_panicking(text in "\\PC{0,400}") {
            let _ = read(&text);
        }
    }
}
