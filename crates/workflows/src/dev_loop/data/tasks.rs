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
//! 4. **`harness:ready` commands everything.** `split` opens every agent
//!    task with it; a task without it — a human took it off, or opened the
//!    task by hand — stops the run; it especially does not read as "nothing
//!    left", which would wrongly say the milestone is finished.
//!
//! Pure business logic: no I/O, no subprocess.

use harness_core::domain::{Issue, Tags};

use crate::common::{labels, sections};
use harness_architecture::vocabulary::{Declaration, Side, Unit};

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

/// True if the SPEC is already written in the body: `split` wrote its goal
/// and acceptance criteria, or a refinement marked it `harness:spec-written`.
#[must_use]
pub fn spec_written(issue: &Issue) -> bool {
    if issue.has(labels::SPEC_WRITTEN) {
        return true;
    }
    let found = sections::parse(&issue.body);
    ["business-goal", "acceptance-criteria"]
        .iter()
        .all(|key| found.get(*key).is_some_and(|text| !text.trim().is_empty()))
}

/// True if the task is delivered and waiting for a merge.
#[must_use]
pub fn waiting_merge(issue: &Issue) -> bool {
    issue.has(labels::WAITING_MERGE)
}

/// True if the task's PR is open and waits for its review and a green CI —
/// the write side's own waiting state, before `waiting-merge`.
#[must_use]
pub fn review_pending(issue: &Issue) -> bool {
    issue.has(labels::REVIEW_PENDING)
}

/// True if the task is on the write side of the architecture.
///
/// The label decides first; a task opened by hand, without one, is read from
/// its `## Architecture` section; a task that declares nothing is read-side —
/// the behaviour every task had before the sides existed, and the harmless
/// reading when wrong here: its PR merges on its own, as they all did.
#[must_use]
pub fn is_write_side(issue: &Issue) -> bool {
    if issue.has(labels::WRITE_SIDE) {
        return true;
    }
    if issue.has(labels::READ_SIDE) {
        return false;
    }
    sections::parse(&issue.body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
        .is_some_and(|declaration| declaration.side() == Side::Write)
}

/// The branch the task's body declares — the `branch: <type>/<slug>` line
/// `split` writes under Scope — if any.
#[must_use]
pub fn declared_branch(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("branch:"))
        .map(|branch| branch.trim().trim_matches('`').to_string())
        .filter(|branch| !branch.is_empty())
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
///
/// A Micro-UI does not wait on the crates it reads or writes through: it is
/// built against the mocked contracts and their fixtures, so the mock stands
/// in for its capabilities (`ForgeDesMondes`' #141 waited on four of them,
/// and no Micro-UI of the seven milestones ahead could start). It still
/// waits on another Micro-UI, a screen, and on anything not an agent task:
/// a human, or the contract request its own stop opened (#167 on #220).
#[must_use]
pub fn blockers_pending(issue: &Issue) -> Vec<&Issue> {
    let on_mocks = unit_of(issue) == Some(Unit::MicroUi);
    issue
        .blocked_by
        .iter()
        .filter(|blocker| blocker.is_open() && !waiting_merge(blocker))
        .filter(|blocker| !on_mocks || !is_agent(blocker) || mock_up_stands_in(blocker))
        .collect()
}

/// The unit the task's `## Architecture` declares, if any.
fn unit_of(issue: &Issue) -> Option<Unit> {
    sections::parse(&issue.body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
        .map(|declaration| declaration.unit)
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
        && !review_pending(issue)
        && !awaits_arbitration(issue)
        && !awaits_decision(issue)
        && blockers_pending(issue).is_empty()
}

/// A decision is asked on the task — a session stopped on an ambiguity, the
/// breaker refused, its PR's repairs are used up: `decide` takes it at the
/// next tick, and the task does not start before.
#[must_use]
pub fn awaits_decision(issue: &Issue) -> bool {
    issue.has(labels::NEEDS_DECISION)
}

/// The task's graph is in question: its refinement raised what `needs`
/// does not name, and the arbiter has not settled it. Its blockers may be
/// incomplete, so it does not start.
#[must_use]
pub fn awaits_arbitration(issue: &Issue) -> bool {
    issue.has(labels::NEEDS_AI_DECISION)
}

/// Every runnable open task, in number order — what a parallel watch
/// spreads over its lanes, where a single loop takes only the first.
#[must_use]
pub fn runnable_tasks(issues: &[Issue]) -> Vec<&Issue> {
    open_agent_tasks(issues)
        .into_iter()
        .filter(|issue| runnable(issue))
        .collect()
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
    if review_pending(issue) {
        return format!(
            "{} {}: on the write side — its PR is open and waits for its review and a green CI",
            issue.reference(),
            issue.title
        );
    }
    let mut reasons = Vec::new();
    if !is_ready(issue) {
        reasons.push(format!("no {} label", labels::READY));
    }
    if awaits_arbitration(issue) {
        reasons.push(format!(
            "{}: its graph is in question until the arbiter settles it",
            labels::NEEDS_AI_DECISION
        ));
    }
    if awaits_decision(issue) {
        reasons.push(format!(
            "{}: a decision is asked — the harness takes it at the next tick",
            labels::NEEDS_DECISION
        ));
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

/// The aggregate a task holds while it runs.
///
/// The `aggregate:` line of its `## Architecture`, for a unit that changes
/// the aggregate's structure ([`Declaration::held_aggregate`]). A task that
/// declares nothing holds nothing: the behaviour every task had before, and
/// the harmless reading.
#[must_use]
pub fn held_aggregate(issue: &Issue) -> Option<String> {
    sections::parse(&issue.body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
        .and_then(|declaration| declaration.held_aggregate().map(ToString::to_string))
}

/// What the task is, for the ledger: its unit, the profile it was cut for,
/// its side — read from its `## Architecture`. A task that declares nothing
/// is tagged empty, never guessed.
#[must_use]
pub fn tags(issue: &Issue) -> Tags {
    sections::parse(&issue.body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
        .map_or_else(Tags::default, |declaration| Tags {
            unit: declaration.unit.key().to_string(),
            profile: declaration.profile.clone().unwrap_or_default(),
            side: declaration.side().label().to_string(),
        })
}

/// The tasks a milestone may run while the producers of what it reads have
/// not landed: what the mock-up stands in for.
///
/// The Micro-UIs, built against the mocked contracts — and nothing else. A
/// contract is mocked until built; a table is not (`ForgeDesMondes`' #118,
/// the `NoteImport` aggregate of #23, needed #18's migrations), nor is a
/// crate (#216 of #28 needed `crates/session/`, #20's), nor a screen: it
/// composes Micro-UIs other milestones build and proves its rules end to end
/// (#219 of #28 found five of its six Micro-UIs missing). Everything else
/// waits for the producers to land.
#[must_use]
pub fn mocked_only<'a>(tasks: &[&'a Issue]) -> (Vec<&'a Issue>, Vec<u64>) {
    let (taken, held): (Vec<&Issue>, Vec<&Issue>) = tasks.iter().partition(|task| runs_ahead(task));
    (taken, held.into_iter().map(|task| task.number).collect())
}

/// Whether this task may run ahead of its producers: a Micro-UI outside the
/// data layer, the one unit the mock-up fully stands in for.
#[must_use]
pub fn runs_ahead(task: &Issue) -> bool {
    !task.has(labels::DATA_LAYER) && unit_of(task) == Some(Unit::MicroUi)
}

/// Whether the mock-up stands in for what this task builds — a Micro-UI or
/// a screen, never the data layer: a blocker of this kind is not waived for
/// a Micro-UI ([`blockers_pending`]).
#[must_use]
pub fn mock_up_stands_in(task: &Issue) -> bool {
    !task.has(labels::DATA_LAYER)
        && matches!(unit_of(task), Some(Unit::MicroUi | Unit::Composition))
}

/// The runnable tasks a parallel watch may start now, given those on a lane.
///
/// Two tasks that change the structure of the same aggregate run one after
/// the other — the data layer is shared by construction, and two changes of
/// one aggregate in parallel meet there. A task is held back while a running
/// task holds its aggregate, or while a runnable task of a smaller number
/// does — so two free lanes do not take the same aggregate on one tick.
/// Different aggregates, and tasks holding none, run together.
#[must_use]
pub fn startable_tasks<'a>(issues: &'a [Issue], running: &[u64]) -> Vec<&'a Issue> {
    let runnable = runnable_tasks(issues);
    let mut held: Vec<String> = issues
        .iter()
        .filter(|issue| running.contains(&issue.number))
        .filter_map(held_aggregate)
        .collect();
    let mut startable = Vec::new();
    for task in runnable {
        if running.contains(&task.number) {
            continue;
        }
        match held_aggregate(task) {
            Some(aggregate) if held.contains(&aggregate) => {}
            Some(aggregate) => {
                held.push(aggregate);
                startable.push(task);
            }
            None => startable.push(task),
        }
    }
    startable
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

/// One line on why a milestone with open tasks offers nothing to run.
///
/// What an idle tick says instead of a bare "nothing", so the human sees
/// the gesture it waits for: "#17 offers nothing to run: 6 delivered,
/// waiting for the merge · human action open: #67".
#[must_use]
pub fn idle_reason(milestone: u64, issues: &[Issue]) -> String {
    let open = open_agent_tasks(issues);
    let delivered = open.iter().filter(|task| waiting_merge(task)).count();
    let pending_review = open.iter().filter(|task| review_pending(task)).count();
    let not_ready = open
        .iter()
        .filter(|task| !is_ready(task) && !waiting_merge(task) && !review_pending(task))
        .count();
    let blocked = open
        .iter()
        .filter(|task| is_ready(task) && !blockers_pending(task).is_empty())
        .count();
    let mut humans: Vec<String> = issues
        .iter()
        .filter(|issue| issue.is_open() && is_human(issue))
        .map(Issue::reference)
        .collect();
    humans.sort();
    let mut parts = Vec::new();
    if delivered > 0 {
        parts.push(format!("{delivered} delivered, waiting for the merge"));
    }
    if pending_review > 0 {
        parts.push(format!(
            "{pending_review} write-side PR(s) waiting for their review and a green CI"
        ));
    }
    if blocked > 0 {
        parts.push(format!("{blocked} blocked by an open task"));
    }
    if not_ready > 0 {
        parts.push(format!("{not_ready} without {}", labels::READY));
    }
    if !humans.is_empty() {
        parts.push(format!("human action open: {}", humans.join(", ")));
    }
    if parts.is_empty() {
        parts.push("no open task".to_string());
    }
    format!("#{milestone} offers nothing to run: {}", parts.join(" · "))
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

    #[test]
    fn the_tags_are_the_declaration_s_unit_profile_and_side() {
        let declared = Issue {
            body: "## Architecture\n\nunit: capability\nsystem: credit\nprofile: small\n"
                .to_string(),
            ..issue(7, &[labels::AGENT])
        };
        let got = tags(&declared);
        assert_eq!(got.unit, "capability");
        assert_eq!(got.profile, "small");
        assert_eq!(got.side, labels::READ_SIDE);
        let mutating = Issue {
            body: "## Architecture\n\nunit: data-capability\nsystem: credit\neffect: update\n"
                .to_string(),
            ..issue(8, &[labels::AGENT])
        };
        let got = tags(&mutating);
        assert_eq!(got.unit, "data-capability");
        assert_eq!(got.profile, "", "no profile line, no profile invented");
        assert_eq!(got.side, labels::WRITE_SIDE);
    }

    #[test]
    fn a_task_that_declares_nothing_is_tagged_empty_not_guessed() {
        assert_eq!(tags(&issue(9, &[labels::AGENT])), Tags::default());
    }

    fn task(number: u64) -> Issue {
        issue(number, &[labels::AGENT, labels::READY])
    }

    fn closed(mut issue: Issue) -> Issue {
        issue.state = "closed".to_string();
        issue
    }

    #[test]
    fn a_spec_is_written_by_its_label_or_by_the_sections_split_wrote() {
        let mut issue = Issue {
            body: "## Business Goal\n\nthe DM sees the PJs\n\n## Acceptance Criteria\n\n- given, when, then"
                .to_string(),
            ..Issue::default()
        };
        assert!(spec_written(&issue), "split's sections");
        issue.body =
            "## Business Goal\n\nthe DM sees the PJs\n\n## Acceptance Criteria\n\n  ".to_string();
        assert!(!spec_written(&issue), "no criterion, no spec");
        issue.labels = vec![labels::SPEC_WRITTEN.to_string()];
        assert!(spec_written(&issue), "a refinement's label");
    }

    #[test]
    fn a_task_whose_graph_awaits_the_arbiter_does_not_run_and_says_why() {
        let mut held = task(5);
        held.labels.push(labels::NEEDS_AI_DECISION.to_string());
        assert!(runnable(&task(5)));
        assert!(!runnable(&held));
        assert!(
            why_not(&held).contains(labels::NEEDS_AI_DECISION),
            "{}",
            why_not(&held)
        );
        assert!(
            runnable_tasks(&[held, task(6)])
                .iter()
                .all(|t| t.number == 6)
        );
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

    #[test]
    fn the_idle_reason_names_the_merge_and_the_human_action_a_milestone_waits_for() {
        let delivered = issue(61, &[labels::AGENT, labels::READY, labels::WAITING_MERGE]);
        let human = issue(67, &[labels::HUMAN]);
        let unready = issue(70, &[labels::AGENT]);
        let reason = idle_reason(17, &[delivered, human, unready]);
        assert_eq!(
            reason,
            "#17 offers nothing to run: 1 delivered, waiting for the merge · 1 without \
             harness:ready · human action open: #67"
        );
        assert_eq!(
            idle_reason(18, &[]),
            "#18 offers nothing to run: no open task"
        );
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

    // --- the write side ----------------------------------------------------

    #[test]
    fn a_task_whose_pr_waits_for_a_human_is_not_runnable_and_still_blocks_the_next() {
        let waiting = issue(11, &[labels::AGENT, labels::READY, labels::REVIEW_PENDING]);
        assert!(!runnable(&waiting));
        assert!(why_not(&waiting).contains("waits for its review and a green CI"));
        let mut next = task(12);
        next.blocked_by = vec![waiting];
        assert!(!runnable(&next), "its code is not on the branch yet");
    }

    #[test]
    fn the_side_is_read_from_the_label_first_then_from_the_body() {
        assert!(is_write_side(&issue(
            1,
            &[labels::AGENT, labels::WRITE_SIDE]
        )));
        assert!(!is_write_side(&issue(
            2,
            &[labels::AGENT, labels::READ_SIDE]
        )));
        let mut by_body = issue(3, &[labels::AGENT]);
        by_body.body =
            "## Scope\n\nbranch: feat/x\n\n## Architecture\n\nunit: migration\nsystem: credit\n"
                .to_string();
        assert!(is_write_side(&by_body));
        by_body.body = "## Architecture\n\nunit: capability\n".to_string();
        assert!(!is_write_side(&by_body));
        assert!(
            !is_write_side(&issue(4, &[labels::AGENT])),
            "nothing declared: read side"
        );
    }

    #[test]
    fn the_declared_branch_is_the_scope_s_branch_line() {
        assert_eq!(
            declared_branch("## Scope\n\nbranch: `feat/risk-badge`\n\ncovers A").as_deref(),
            Some("feat/risk-badge")
        );
        assert_eq!(declared_branch("no branch here"), None);
        assert_eq!(declared_branch("branch:   "), None);
    }

    fn holding(number: u64, aggregate: &str, unit: &str) -> Issue {
        let mut issue = task(number);
        issue.body = format!(
            "## Architecture\n\nunit: {unit}\nsystem: campagne\neffect: update\ntouches: {aggregate}.name\naggregate: {aggregate}\n"
        );
        issue
    }

    #[test]
    fn two_tasks_changing_one_aggregate_start_one_after_the_other() {
        // #7 and #8 both change PJ; #9 changes Campagne; #10 declares nothing.
        let issues = [
            holding(7, "PJ", "data-capability"),
            holding(8, "PJ", "migration"),
            holding(9, "Campagne", "invariant"),
            task(10),
        ];
        let numbers = |running: &[u64]| -> Vec<u64> {
            startable_tasks(&issues, running)
                .iter()
                .map(|t| t.number)
                .collect()
        };
        assert_eq!(
            numbers(&[]),
            [7, 9, 10],
            "one task per aggregate on a tick, the smallest first"
        );
        assert_eq!(numbers(&[7]), [9, 10], "#8 waits while #7 holds PJ");
        assert_eq!(numbers(&[7, 9]), [10]);
        assert_eq!(
            numbers(&[]),
            runnable_tasks(&issues)
                .iter()
                .map(|t| t.number)
                .filter(|n| *n != 8)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_pure_insert_and_a_reader_hold_no_aggregate() {
        let mut insert = holding(7, "PJ", "data-capability");
        insert.body = "## Architecture\n\nunit: data-capability\nsystem: campagne\neffect: insert\ntouches: -\naggregate: PJ\n".to_string();
        let mut reader = holding(8, "PJ", "capability");
        reader.body =
            "## Architecture\n\nunit: capability\nsystem: campagne\naggregate: PJ\n".to_string();
        let changer = holding(9, "PJ", "migration");
        let issues = [insert, reader, changer];
        let numbers: Vec<u64> = startable_tasks(&issues, &[])
            .iter()
            .map(|t| t.number)
            .collect();
        assert_eq!(
            numbers,
            [7, 8, 9],
            "only a structure change holds the aggregate"
        );
        assert_eq!(held_aggregate(&issues[0]), None);
        assert_eq!(held_aggregate(&issues[2]).as_deref(), Some("PJ"));
    }

    #[test]
    fn every_runnable_task_is_listed_in_number_order() {
        let mut blocked = task(9);
        blocked.blocked_by = vec![task(7)];
        let issues = [task(8), blocked, task(7), issue(6, &[labels::AGENT])];
        let numbers: Vec<u64> = runnable_tasks(&issues).iter().map(|t| t.number).collect();
        assert_eq!(numbers, [7, 8]);
    }

    #[test]
    fn ahead_work_is_what_the_mock_up_stands_in_for() {
        let with = |number: u64, labels: &[&str], unit: &str| Issue {
            body: format!("## Scope\n\nx\n\n## Architecture\n\nunit: {unit}\nsystem: session"),
            ..issue(number, labels)
        };
        let ui = with(
            60,
            &[labels::AGENT, labels::READY, labels::READ_SIDE],
            "micro-ui",
        );
        let screen = with(
            61,
            &[labels::AGENT, labels::READY, labels::READ_SIDE],
            "composition",
        );
        let capability = with(
            216,
            &[labels::AGENT, labels::READY, labels::READ_SIDE],
            "capability",
        );
        let data = with(
            118,
            &[
                labels::AGENT,
                labels::READY,
                labels::WRITE_SIDE,
                labels::DATA_LAYER,
            ],
            "aggregate",
        );
        let write = with(
            119,
            &[labels::AGENT, labels::READY, labels::WRITE_SIDE],
            "data-capability",
        );
        let undeclared = issue(7, &[labels::AGENT, labels::READY]);
        let (taken, held) = mocked_only(&[&ui, &screen, &capability, &data, &write, &undeclared]);
        assert_eq!(
            taken.iter().map(|t| t.number).collect::<Vec<_>>(),
            [60],
            "the mock server stands in for a Micro-UI's reads and writes; a crate needs its producers' code, a screen their Micro-UIs"
        );
        assert_eq!(held, [61, 216, 118, 119, 7]);
        let (none, nothing) = mocked_only(&[]);
        assert!(none.is_empty() && nothing.is_empty());
    }

    #[test]
    fn a_micro_ui_does_not_wait_on_the_crates_the_mock_stands_in_for() {
        let with = |number: u64, labels: &[&str], unit: &str| Issue {
            body: format!("## Scope\n\nx\n\n## Architecture\n\nunit: {unit}\nsystem: soundbox"),
            ..issue(number, labels)
        };
        let capability = with(140, &[labels::AGENT, labels::READY], "capability");
        let data = with(
            116,
            &[labels::AGENT, labels::READY, labels::DATA_LAYER],
            "micro-ui",
        );
        let other_ui = with(138, &[labels::AGENT, labels::READY], "micro-ui");
        let screen = with(143, &[labels::AGENT, labels::READY], "composition");
        let human = issue(99, &[labels::HUMAN]);
        let contract = issue(220, &[labels::CONTRACT_CHANGE]);
        let mut ui = with(141, &[labels::AGENT, labels::READY], "micro-ui");
        ui.blocked_by = vec![capability.clone(), data];
        assert!(runnable(&ui), "the mock stands in for #140 and #116");
        ui.blocked_by = vec![capability.clone(), other_ui, screen, human, contract];
        assert_eq!(
            blockers_pending(&ui)
                .iter()
                .map(|b| b.number)
                .collect::<Vec<_>>(),
            [138, 143, 99, 220]
        );
        let mut freeze = with(143, &[labels::AGENT, labels::READY], "composition");
        freeze.blocked_by = vec![capability];
        assert!(!runnable(&freeze), "a screen proves end to end: it waits");
    }
}
