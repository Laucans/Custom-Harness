//! What a task is for this round, and which one comes next.
//!
//! How **this workflow** reads an `Issue`: its seven labels, the questions they
//! enable, and the four rules that choose the next task. Given already-read
//! `Issue`s, it returns the one that can run — this is what makes the rules
//! testable without network and without doubles.
//!
//! Here, not in `harness-core`: `harness:ready` means nothing for an issue
//! generally, only for this round. Core carries the form of an issue, this
//! module carries what this workflow makes of it.
//!
//! # The four rules, and what each prevents
//!
//! 1. **the current milestone** is the open `harness:milestone` issue with
//!    the smallest number. A total order, so a second milestone opened by
//!    accident doesn't make the choice depend on API order;
//! 2. **the next task** is an open `harness:agent` issue, a sub-issue of
//!    this milestone, carrying `harness:ready`, and whose **all** `blocked_by`
//!    are closed. A real dependency walk, not "is issue N-1 closed": parallelism
//!    is intended, and it must not ask for a data migration to work;
//! 3. **a `harness:human` issue is not a separate mechanism.** It blocks
//!    because it's in `blocked_by`, like any other dependency — the round's
//!    human gate disappeared in this rule;
//! 4. **`harness:ready` commands everything.** An open but not-ready task
//!    stops the run; it especially does not read as "nothing left", which
//!    would wrongly say the milestone is finished.
//!
//! Pure business logic: no I/O, no subprocess.

use harness_core::domain::Issue;

use crate::common::labels;

/// True if the issue is a task the agent can run.
#[must_use]
pub fn is_agent(issue: &Issue) -> bool {
    issue.has(labels::AGENT)
}

/// True if the issue is an action only the human can do.
#[must_use]
pub fn is_human(issue: &Issue) -> bool {
    issue.has(labels::HUMAN)
}

/// True if the human checked the box.
#[must_use]
pub fn is_ready(issue: &Issue) -> bool {
    issue.has(labels::READY)
}

/// True if the SPEC is already written in the body.
#[must_use]
pub fn spec_written(issue: &Issue) -> bool {
    issue.has(labels::SPEC_WRITTEN)
}

/// True if the technical sections are already written in the body.
#[must_use]
pub fn tech_written(issue: &Issue) -> bool {
    issue.has(labels::TECH_WRITTEN)
}

/// True if the task is delivered and waiting for a merge.
#[must_use]
pub fn waiting_merge(issue: &Issue) -> bool {
    issue.has(labels::WAITING_MERGE)
}

/// `human` or `auto` — how the log names the task kind.
#[must_use]
pub fn kind(issue: &Issue) -> &'static str {
    if is_human(issue) { "human" } else { "auto" }
}

/// Blockers that **still** block.
///
/// A task in `waiting-merge` is not one: its code is on the integration
/// branch, so the next one can build on it. Without this exception the chain
/// would stop after one task, and you'd need a merge to `main` per round.
///
/// A human action has no such state — it doesn't deliver on a branch,
/// it closes.
#[must_use]
pub fn blockers_pending(issue: &Issue) -> Vec<&Issue> {
    issue
        .blocked_by
        .iter()
        .filter(|blocker| blocker.is_open() && !waiting_merge(blocker))
        .collect()
}

/// Open, ready, not already delivered, and nothing blocking it.
///
/// `waiting_merge` is what prevents replaying a finished task: it stays open
/// until merge, so without this test the next round would pick it and repay
/// its three stages.
#[must_use]
pub fn runnable(issue: &Issue) -> bool {
    issue.is_open()
        && is_agent(issue)
        && is_ready(issue)
        && !waiting_merge(issue)
        && blockers_pending(issue).is_empty()
}

/// Why this task cannot run, stated for a human.
///
/// A run that stops must name the gesture that unblocks it. "no ready task"
/// names none; "#12 waits for #11 (open)" names one.
#[must_use]
pub fn why_not(issue: &Issue) -> String {
    if issue.is_closed() {
        return format!("{} is closed", issue.reference());
    }
    if waiting_merge(issue) {
        return format!(
            "{} {}: delivered on the integration branch, waiting for the merge \
             that closes it",
            issue.reference(),
            issue.title
        );
    }
    let mut reasons = Vec::new();
    if !is_ready(issue) {
        reasons.push(format!("no {} label", labels::READY));
    }
    for blocker in blockers_pending(issue) {
        let said = if is_human(blocker) {
            "human action"
        } else {
            "task"
        };
        reasons.push(format!(
            "blocked by {} ({said}, still open)",
            blocker.reference()
        ));
    }
    if reasons.is_empty() {
        reasons.push("ready".to_string());
    }
    format!(
        "{} {}: {}",
        issue.reference(),
        issue.title,
        reasons.join(", ")
    )
}

/// Rule 1: the lowest-numbered open `harness:milestone`.
#[must_use]
pub fn current_milestone(issues: &[Issue]) -> Option<&Issue> {
    issues
        .iter()
        .filter(|issue| issue.is_open() && issue.has(labels::MILESTONE))
        .min_by_key(|issue| issue.number)
}

/// Sub-issues that are agent tasks, in number order.
#[must_use]
pub fn agent_tasks(issues: &[Issue]) -> Vec<&Issue> {
    let mut found: Vec<&Issue> = issues.iter().filter(|i| is_agent(i)).collect();
    found.sort_by_key(|issue| issue.number);
    found
}

/// Open agent tasks — empty means the milestone has nothing left to run.
#[must_use]
pub fn open_agent_tasks(issues: &[Issue]) -> Vec<&Issue> {
    agent_tasks(issues)
        .into_iter()
        .filter(|issue| issue.is_open())
        .collect()
}

/// Rule 2: the smallest runnable open task.
#[must_use]
pub fn next_task(issues: &[Issue]) -> Option<&Issue> {
    open_agent_tasks(issues)
        .into_iter()
        .filter(|issue| runnable(issue))
        .min_by_key(|issue| issue.number)
}

/// Why no task can run, task by task.
///
/// The case this text exists for, not to confuse with "nothing left": tasks
/// remain open, so the milestone is not done — it is only blocked.
#[must_use]
pub fn stuck_report(issues: &[Issue]) -> String {
    open_agent_tasks(issues)
        .into_iter()
        .map(|issue| format!("  - {}", why_not(issue)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What counts as proof a task shipped — moved to
/// [`crate::common::delivery`] once the milestone merge needed the same
/// reading, re-exported here so the loop keeps naming it where it reads it.
pub use crate::common::delivery::{closes, first_closing};

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("task {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn task(number: u64) -> Issue {
        issue(number, &[labels::AGENT, labels::READY])
    }

    fn closed(mut issue: Issue) -> Issue {
        issue.state = "closed".to_string();
        issue
    }

    // --- rule 1: the milestone -----------------------------------------

    #[test]
    fn the_current_milestone_is_the_lowest_numbered_open_one() {
        // Total order: a second milestone opened by accident must not make the
        // choice depend on API order.
        let issues = vec![
            issue(9, &[labels::MILESTONE]),
            issue(4, &[labels::MILESTONE]),
            issue(7, &[labels::MILESTONE]),
        ];
        assert_eq!(current_milestone(&issues).expect("a milestone").number, 4);
    }

    #[test]
    fn a_closed_milestone_is_not_the_current_one() {
        let issues = vec![
            closed(issue(4, &[labels::MILESTONE])),
            issue(9, &[labels::MILESTONE]),
        ];
        assert_eq!(current_milestone(&issues).expect("a milestone").number, 9);
    }

    #[test]
    fn no_milestone_at_all_is_none_rather_than_a_guess() {
        assert!(current_milestone(&[issue(1, &[labels::AGENT])]).is_none());
    }

    // --- rule 2: the next task -----------------------------------------

    #[test]
    fn the_next_task_is_the_lowest_numbered_runnable_one() {
        let issues = vec![task(12), task(7), task(20)];
        assert_eq!(next_task(&issues).expect("a task").number, 7);
    }

    #[test]
    fn a_task_blocked_by_an_open_issue_is_not_runnable() {
        let mut blocked = task(12);
        blocked.blocked_by = vec![issue(11, &[labels::AGENT])];
        assert!(!runnable(&blocked));
        assert!(next_task(&[blocked]).is_none());
    }

    #[test]
    fn a_task_whose_blockers_are_all_closed_is_runnable() {
        let mut freed = task(12);
        freed.blocked_by = vec![closed(issue(11, &[labels::AGENT]))];
        assert!(runnable(&freed));
    }

    #[test]
    fn a_dependency_chain_is_walked_not_assumed_from_numbering() {
        // Parallelism is intended: #20 can run before #12 if #12 is blocked.
        // "is issue N-1 closed" would give #12.
        let mut blocked = task(12);
        blocked.blocked_by = vec![issue(11, &[labels::AGENT])];
        let issues = vec![blocked, task(20)];
        assert_eq!(next_task(&issues).expect("a task").number, 20);
    }

    // --- rule 3: human blocks through dependency -------------------------

    #[test]
    fn a_human_issue_blocks_through_the_dependency_not_a_mechanism_of_its_own() {
        let mut waiting = task(12);
        waiting.blocked_by = vec![issue(11, &[labels::HUMAN])];
        assert!(!runnable(&waiting));
        assert!(why_not(&waiting).contains("human action"));
    }

    #[test]
    fn a_closed_human_issue_stops_blocking_like_any_other() {
        let mut freed = task(12);
        freed.blocked_by = vec![closed(issue(11, &[labels::HUMAN]))];
        assert!(runnable(&freed));
    }

    // --- rule 4: ready commands everything --------------------------------

    #[test]
    fn an_open_task_without_ready_is_not_runnable() {
        let not_ready = issue(12, &[labels::AGENT]);
        assert!(!runnable(&not_ready));
        assert!(why_not(&not_ready).contains(labels::READY));
    }

    #[test]
    fn a_ready_label_alone_is_not_enough_without_the_agent_label() {
        assert!(!runnable(&issue(12, &[labels::READY])));
    }

    // --- waiting-merge: the third state -----------------------------------

    #[test]
    fn a_delivered_task_is_never_picked_again() {
        // Without this test, the next round would pick it and repay its three
        // stages.
        let delivered = issue(12, &[labels::AGENT, labels::READY, labels::WAITING_MERGE]);
        assert!(!runnable(&delivered));
        assert!(next_task(&[delivered]).is_none());
    }

    #[test]
    fn a_delivered_blocker_stops_blocking_so_the_chain_continues() {
        // Its code is on the integration branch: the next one can build on it.
        // Otherwise you'd need a merge to `main` per round.
        let mut next = task(12);
        next.blocked_by = vec![issue(11, &[labels::AGENT, labels::WAITING_MERGE])];
        assert!(runnable(&next));
    }

    #[test]
    fn a_delivered_task_says_what_it_is_waiting_for() {
        let delivered = issue(12, &[labels::AGENT, labels::WAITING_MERGE]);
        assert!(why_not(&delivered).contains("waiting for the merge"));
    }

    // --- the blocking report ----------------------------------------------

    #[test]
    fn the_stuck_report_names_a_gesture_for_every_open_task() {
        let mut blocked = task(12);
        blocked.blocked_by = vec![issue(11, &[labels::HUMAN])];
        let report = stuck_report(&[blocked, issue(20, &[labels::AGENT])]);
        assert!(report.contains("#12"));
        assert!(report.contains("#20"));
        assert!(report.contains(labels::READY), "say what to check");
        assert_eq!(report.lines().count(), 2);
    }

    // --- proof of delivery -----------------------------------

    #[test]
    fn closes_needs_the_keyword_on_its_own_line() {
        assert!(closes("du texte\nCloses #42\nencore", 42));
        assert!(closes("fixes #42", 42));
        assert!(closes("  resolves\t#42  ", 42));
    }

    #[test]
    fn a_mention_inside_a_sentence_does_not_count_as_closing() {
        // Stricter than GitHub, and rightly so: reading wider would close an
        // issue on a sentence mentioning it.
        assert!(!closes("this PR closes #42 among other things", 42));
        assert!(!closes("see closes #42 below", 42));
    }

    #[test]
    fn closing_another_issue_is_not_closing_this_one() {
        assert!(!closes("Closes #420", 42));
        assert!(!closes("Closes #4", 42));
    }

    #[test]
    fn the_first_pr_that_closes_the_task_is_the_proof() {
        let prs = vec![
            Issue {
                number: 1,
                body: "nothing to do with it".to_string(),
                ..Issue::default()
            },
            Issue {
                number: 2,
                body: "Closes #42".to_string(),
                ..Issue::default()
            },
        ];
        assert_eq!(first_closing(&prs, 42).expect("a PR").number, 2);
        assert!(first_closing(&prs, 99).is_none());
    }
}
