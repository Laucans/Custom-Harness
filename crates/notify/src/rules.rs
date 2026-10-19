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

/// The watch's own life: started, stopping, stopped.
fn watch(board: &mut Board, event: &Event, at: &str) {
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
        _ => {}
    }
}

/// The plant's progress the harness made without asking: told, with the
/// gesture that takes it back.
fn progress(board: &mut Board, event: &Event, at: &str) {
    match event {
        Event::MilestonesPlanned {
            roadmap,
            milestones,
        } => board.add(Notification::once(
            format!("planned:#{roadmap}"),
            Level::Info,
            format!(
                "#{roadmap}: {} milestone(s) planned, ready to split",
                milestones.len()
            ),
            format!(
                "{} — to hold one, take its harness:ready off",
                numbers(milestones)
            ),
            at,
            Some(issue(*roadmap)),
        )),
        Event::MilestoneSplit {
            milestone,
            tasks,
            human_tasks,
        } => board.add(Notification::once(
            format!("split:#{milestone}"),
            if human_tasks.is_empty() {
                Level::Info
            } else {
                Level::Warning
            },
            format!("#{milestone}: {} task(s) opened", tasks.len()),
            if human_tasks.is_empty() {
                "each agent task is ready to build — to hold one, take its harness:ready off; \
                     to cut the milestone again, take harness:triggered off it"
                    .to_string()
            } else {
                format!(
                    "{} only a human can do — do it, then close it; the others are ready to build",
                    numbers(human_tasks)
                )
            },
            at,
            Some(issue(*milestone)),
        )),
        Event::DecisionTaken { .. } | Event::HumanTaskOpened { .. } => decisions(board, event, at),
        _ => delivery(board, event, at),
    }
}

/// A task, a milestone, a roadmap item delivered — or a PR on its way.
fn delivery(board: &mut Board, event: &Event, at: &str) {
    match event {
        Event::PrOpened { task, pr, base } => board.add(Notification::once(
            format!("pr:#{task}"),
            Level::Info,
            format!("#{task}: PR {pr} open on the write side"),
            format!(
                "its review runs, and it is merged into {base} once the review is clean and \
                 every check green — to stop it, close the PR"
            ),
            at,
            Some(issue(*task)),
        )),
        Event::TaskDelivered { task, pr, branch } => {
            for key in ["failing", "parked", "pr", "repairs", "not-started"] {
                board.clear(&format!("{key}:#{task}"));
            }
            board.add(Notification::once(
                format!("delivered:#{task}"),
                Level::Info,
                format!("#{task} delivered"),
                format!("PR {pr} merged on {branch} — to take it back, revert the PR"),
                at,
                Some(issue(*task)),
            ));
        }
        Event::RepairsExhausted { task, pr, fixes } => board.add(Notification::once(
            format!("repairs:#{task}"),
            Level::Info,
            format!("#{task}: PR {pr} still blocked after {fixes} repair(s)"),
            "the harness decides now — merge as it is, repair once more, or cut the task — \
             and tells you what it decided"
                .to_string(),
            at,
            Some(issue(*task)),
        )),
        Event::MilestoneLanded { milestone, pr } => board.add(Notification::once(
            format!("release:#{milestone}"),
            Level::Warning,
            format!("#{milestone} landed — ready to release"),
            format!(
                "PR {pr} merged it on the integration branch: merge the integration branch \
                 into main when you release, then close #{milestone}"
            ),
            at,
            Some(issue(*milestone)),
        )),
        Event::RoadmapDelivered { roadmap } => board.add(Notification::once(
            format!("release:roadmap:#{roadmap}"),
            Level::Warning,
            format!("#{roadmap}: every milestone landed"),
            format!("the roadmap item is delivered: release it, then close #{roadmap}"),
            at,
            Some(issue(*roadmap)),
        )),
        _ => {}
    }
}

/// What the harness decided, and what it handed to a human.
fn decisions(board: &mut Board, event: &Event, at: &str) {
    match event {
        Event::DecisionTaken {
            issue: n,
            kind,
            action,
            confidence,
            summary,
        } => {
            board.clear(&format!("repairs:#{n}"));
            board.add(Notification::once(
                format!("decided:#{n}"),
                Level::Warning,
                format!("#{n}: the harness decided ({kind}) — {action}"),
                format!(
                    "{summary} ({confidence}/5). To overrule it: edit ## Decision \
                     (decided-by: owner) and pose harness:needs-decision again; take \
                     harness:ai-decided off once read"
                ),
                at,
                Some(issue(*n)),
            ));
        }
        Event::HumanTaskOpened { task, blocks, why } => board.add(Notification::once(
            format!("human:#{task}"),
            Level::Warning,
            format!("#{task} needs you"),
            format!("{why} — do it, then close #{task}; #{blocks} waits on it"),
            at,
            Some(issue(*task)),
        )),
        _ => {}
    }
}

/// Issue numbers as a reader names them: `#3, #4`.
fn numbers(list: &[u64]) -> String {
    list.iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn tell(board: &mut Board, event: &Event, at: &str) {
    match event {
        Event::WatchStarted { .. } | Event::WatchDraining { .. } | Event::WatchStopped => {
            watch(board, event, at);
        }
        Event::ChangesApplied { prompts } => changes_applied(board, prompts, at),
        Event::QuotaRest { ticks, strikes } => quota_rest(board, *ticks, *strikes, at),
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
        Event::MilestonesPlanned { .. }
        | Event::MilestoneSplit { .. }
        | Event::PrOpened { .. }
        | Event::TaskDelivered { .. }
        | Event::RepairsExhausted { .. }
        | Event::MilestoneLanded { .. }
        | Event::RoadmapDelivered { .. }
        | Event::DecisionTaken { .. }
        | Event::HumanTaskOpened { .. }
        | Event::ContractChangeAsked { .. }
        | Event::ContractEvolved { .. } => progress(board, event, at),
        Event::BaseRed { branch, sha } => board.add(Notification::once(
            format!("base-red:{branch}"),
            Level::Warning,
            format!("Integration branch {branch} is red"),
            format!(
                "its CI failed at {}: no task starts and nothing merges until it is green — \
                 look at its last workflow runs, fix or revert the push; the harness closes \
                 the issue \"Integration branch {branch} is red\" once it is green",
                sha.as_deref().unwrap_or("its newest head")
            ),
            at,
            Some(screen("journal", at)),
        )),
        Event::BaseGreen { branch } => board.clear(&format!("base-red:{branch}")),
        Event::DataSnapshotFailed { .. }
        | Event::DataSnapshotTaken { .. }
        | Event::DataModelDiverged { .. }
        | Event::DataModelAgreed { .. } => data(board, event, at),
        // A gate's verdict is the line's business, not a sign: a halt that matters
        // arrives as the `Halted` event that follows it. Failing checks are
        // being repaired; the repair that gives up says so (`RepairsExhausted`).
        Event::SessionEnded { .. }
        | Event::PreparationTaken { .. }
        | Event::LaneTaken { .. }
        | Event::TapOpened { .. }
        | Event::GateChecked { .. }
        | Event::ChecksFailed { .. } => {}
    }
}

/// What the data model's watch tells: an archive that failed, a version
/// that diverges, and what clears each.
fn data(board: &mut Board, event: &Event, at: &str) {
    match event {
        Event::DataSnapshotFailed {
            branch,
            sha,
            reason,
        } => snapshot_failed(board, branch, sha, reason, at),
        Event::DataModelDiverged {
            branch,
            dir,
            divergences,
            diagnosis,
            ..
        } => diverged(board, branch, dir, divergences.len(), diagnosis, at),
        Event::DataModelAgreed { branch, .. } => board.clear(&format!("data-diverged:{branch}")),
        // A later snapshot of the branch clears its failure.
        Event::DataSnapshotTaken { branch, .. } => board.clear(&format!("data-snapshot:{branch}")),
        _ => {}
    }
}

/// A branch whose database and model disagree: one error per branch, its
/// diagnosis's first paragraph, cleared when the two agree again.
fn diverged(board: &mut Board, branch: &str, dir: &str, count: usize, diagnosis: &str, at: &str) {
    let detail = diagnosis
        .split("\n\n")
        .next()
        .filter(|first| !first.trim().is_empty())
        .map_or_else(
            || format!("{count} divergence(s) between the database and the model"),
            str::to_string,
        );
    board.add(Notification::once(
        format!("data-diverged:{branch}"),
        Level::Error,
        format!("The data model of {branch} diverges"),
        detail,
        at,
        Some(Link::DataModel {
            branch: branch.to_string(),
            dir: dir.to_string(),
            at: at.to_string(),
        }),
    ));
}

/// A branch's data model not archived: one warning per branch, cleared by
/// its next snapshot.
fn snapshot_failed(board: &mut Board, branch: &str, sha: &str, reason: &str, at: &str) {
    board.add(Notification::once(
        format!("data-snapshot:{branch}"),
        Level::Warning,
        format!("The data model of {branch} was not archived"),
        format!("{sha}: {reason}"),
        at,
        Some(screen("journal", at)),
    ));
}

/// The watch's life is one line, telling its last state.
/// The watch started on changes the human left waiting.
fn quota_rest(board: &mut Board, ticks: u32, strikes: u32, at: &str) {
    board.add(Notification::once(
        "quota-rest",
        Level::Warning,
        "The subscription window is exhausted",
        format!("the watch rests {ticks} tick(s) before paying again — {strikes} stop(s) in a row"),
        at,
        Some(screen("quota", at)),
    ));
}

fn changes_applied(board: &mut Board, prompts: &[String], at: &str) {
    board.add(Notification::once(
        format!("changes-applied:{at}"),
        Level::Info,
        "The factory's changes are applied",
        format!(
            "{} prompt(s) now run as edited: {}",
            prompts.len(),
            prompts.join(", ")
        ),
        at,
        None,
    ));
}

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
    fn the_taps_the_harness_opens_are_told_with_the_gesture_that_closes_them() {
        let board = from_events(&[
            stored(
                1,
                Event::MilestonesPlanned {
                    roadmap: 2,
                    milestones: vec![3, 4],
                },
            ),
            stored(
                2,
                Event::MilestoneSplit {
                    milestone: 3,
                    tasks: vec![5, 6],
                    human_tasks: vec![],
                },
            ),
            stored(
                3,
                Event::MilestoneSplit {
                    milestone: 4,
                    tasks: vec![7, 8],
                    human_tasks: vec![8],
                },
            ),
        ]);
        let planned = board.get("planned:#2").expect("planned");
        assert_eq!(planned.level, Level::Info);
        assert!(planned.detail.contains("#3, #4") && planned.detail.contains("harness:ready off"));
        assert_eq!(planned.link, Some(Link::Issue { number: 2 }));
        let split = board.get("split:#3").expect("split");
        assert_eq!(split.level, Level::Info);
        assert!(split.detail.contains("harness:triggered off"));
        let human = board.get("split:#4").expect("split with a human task");
        assert_eq!(human.level, Level::Warning, "a human has something to do");
        assert!(
            human.detail.contains("#8 only a human can do"),
            "{}",
            human.detail
        );
    }

    #[test]
    fn a_red_base_is_told_until_it_is_green() {
        let red = from_events(&[stored(
            1,
            Event::BaseRed {
                branch: "main_agent".to_string(),
                sha: Some("abc1234".to_string()),
            },
        )]);
        let told = red.get("base-red:main_agent").expect("told");
        assert_eq!(told.level, Level::Warning);
        assert_eq!(told.title, "Integration branch main_agent is red");
        assert!(
            told.detail.contains("abc1234") && told.detail.contains("nothing merges"),
            "{}",
            told.detail
        );
        let green = from_events(&[
            stored(
                1,
                Event::BaseRed {
                    branch: "main_agent".to_string(),
                    sha: None,
                },
            ),
            stored(
                2,
                Event::BaseGreen {
                    branch: "main_agent".to_string(),
                },
            ),
        ]);
        assert!(green.get("base-red:main_agent").is_none(), "taken back");
    }

    #[test]
    fn a_delivery_takes_back_what_was_told_of_its_task() {
        let board = from_events(&[
            ended(1, Some(2)),
            stored(
                2,
                Event::PrOpened {
                    task: 15,
                    pr: "#40".to_string(),
                    base: "milestone/3-x".to_string(),
                },
            ),
            stored(
                3,
                Event::RepairsExhausted {
                    task: 15,
                    pr: "#40".to_string(),
                    fixes: 2,
                },
            ),
        ]);
        assert!(board.get("failing:#15").is_some());
        assert!(
            board
                .get("pr:#15")
                .expect("open")
                .detail
                .contains("close the PR")
        );
        assert!(
            board
                .get("repairs:#15")
                .expect("told")
                .detail
                .contains("harness decides")
        );
        let mut events = vec![ended(1, Some(2))];
        events.push(stored(
            4,
            Event::TaskDelivered {
                task: 15,
                pr: "#40".to_string(),
                branch: "milestone/3-x".to_string(),
            },
        ));
        let delivered = from_events(&events);
        assert!(delivered.get("failing:#15").is_none(), "taken back");
        let told = delivered.get("delivered:#15").expect("told");
        assert_eq!(told.level, Level::Info);
        assert!(told.detail.contains("revert the PR"));
        let landed = from_events(&[
            stored(
                5,
                Event::MilestoneLanded {
                    milestone: 3,
                    pr: "#41".to_string(),
                },
            ),
            stored(6, Event::RoadmapDelivered { roadmap: 2 }),
        ]);
        assert_eq!(
            landed.get("release:#3").expect("landed").level,
            Level::Warning
        );
        assert!(landed.get("release:roadmap:#2").is_some());
    }

    fn diverged(minute: u32, branch: &str, diagnosis: &str) -> Stored {
        stored(
            minute,
            Event::DataModelDiverged {
                branch: branch.to_string(),
                sha: "abcdef0".to_string(),
                dir: format!("{branch}/v1"),
                divergences: vec!["a".to_string(), "b".to_string()],
                diagnosis: diagnosis.to_string(),
                actions: vec![],
                failure: String::new(),
            },
        )
    }

    #[test]
    fn a_quota_rest_is_one_warning_that_counts_up() {
        let rest = |id, ticks, strikes| Stored {
            id,
            at: "2026-10-10T10:00:00Z".to_string(),
            source: "watch".to_string(),
            event: Event::QuotaRest { ticks, strikes },
        };
        let told = from_events(&[rest(1, 4, 1), rest(2, 8, 2)]).into_sorted();
        assert_eq!(told.len(), 1);
        assert_eq!((told[0].level, told[0].count), (Level::Warning, 2));
        assert!(told[0].detail.contains("8 tick(s)"));
    }

    #[test]
    fn a_diverging_model_is_an_error_until_the_two_sides_agree() {
        let board = from_events(&[
            diverged(
                1,
                "main_agent",
                "The model is behind.\n\n**x** — wrong: model",
            ),
            diverged(2, "main", ""),
        ]);
        let agent = board.get("data-diverged:main_agent").expect("told");
        assert_eq!(agent.level, Level::Error);
        assert_eq!(agent.detail, "The model is behind.");
        assert!(matches!(&agent.link, Some(Link::DataModel { dir, .. }) if dir == "main_agent/v1"));
        let main = board.get("data-diverged:main").expect("told");
        assert_eq!(
            main.detail,
            "2 divergence(s) between the database and the model"
        );

        let board = from_events(&[
            diverged(1, "main_agent", "x"),
            stored(
                2,
                Event::DataModelAgreed {
                    branch: "main_agent".to_string(),
                    sha: "f".to_string(),
                },
            ),
        ]);
        assert!(board.get("data-diverged:main_agent").is_none());
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
