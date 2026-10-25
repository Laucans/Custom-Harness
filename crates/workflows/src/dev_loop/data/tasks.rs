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

use harness_core::domain::{Issue, Pr, Tags};

use crate::common::{labels, sections};
use harness_architecture::graph::{Graph, Kind};
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

/// Whether `pr` is this task's: open on the branch the task's body declares,
/// or titled with the task's number (`#248`, as a whole token).
///
/// The branch is what the loop writes and reads; the title is the second
/// reading, for a PR a session opened off the declared branch. A PR of
/// another task — `#2480` for #248 — names nothing.
#[must_use]
pub fn pr_names_task(pr: &Pr, task: &Issue) -> bool {
    if declared_branch(&task.body).is_some_and(|branch| pr.head == branch) {
        return true;
    }
    crate::common::delivery::title_names(&pr.title, task.number)
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
    blockers_pending_in(issue, None)
}

/// [`blockers_pending`], judged on the plan's graph too: a screen waives a
/// milestone blocker the mock-up stands in for ([`screen_stands_on`]).
///
/// The arbiter and a stop's wait add a milestone blocker for a need:
/// `ForgeDesMondes`' #219 was `blocked_by` #24 for `OralBrief`, a Micro-UI
/// the screen lays out against its fragment — held until #24 landed, when
/// its split was all it needed.
#[must_use]
pub fn blockers_pending_in<'a>(issue: &'a Issue, graph: Option<&Graph>) -> Vec<&'a Issue> {
    let on_mocks = unit_of(issue) == Some(Unit::MicroUi);
    issue
        .blocked_by
        .iter()
        .filter(|blocker| still_blocks(blocker))
        .filter(|blocker| !on_mocks || !is_agent(blocker) || mock_up_stands_in(blocker))
        .filter(|blocker| !graph.is_some_and(|graph| screen_stands_on(issue, blocker, graph)))
        .collect()
}

/// Whether a screen (a composition) stands on the mock-up for what this
/// milestone blocker produces.
///
/// It needs at least one id the milestone produces, and every one is a
/// kind the mock-up stands in for — a
/// Micro-UI it lays out, a Capability, a `DataCapability` — as [`held_by`]
/// judges. A blocker it needs nothing of (a stop's wait the graph cannot
/// explain), a table or a Concept still holds it.
#[must_use]
pub fn screen_stands_on(task: &Issue, blocker: &Issue, graph: &Graph) -> bool {
    let Some(declaration) = declaration(task).filter(|d| d.unit == Unit::Composition) else {
        return false;
    };
    let mut from_it = declaration
        .needs
        .iter()
        .filter_map(|id| graph.ids.get(id))
        .filter(|interface| interface.producer == Some(blocker.number))
        .peekable();
    from_it.peek().is_some() && from_it.all(|interface| mocked(interface.kind, true))
}

/// Whether the mock-up stands in for an id of this kind another milestone
/// builds, for a screen or not ([`mocked_only`]).
const fn mocked(kind: Kind, screen: bool) -> bool {
    match kind {
        Kind::Capability | Kind::DataCapability => true,
        // A screen lays its Micro-UIs out against the mock-up's fragments.
        Kind::MicroUi => screen,
        Kind::Aggregate | Kind::Concept => false,
    }
}

/// Whether a blocker still holds what it blocks: open, and not landed.
///
/// A task lands on its milestone's branch (`harness:waiting-merge`, then
/// closed by the merge); a milestone lands on the integration branch the
/// same way — `main_agent_merge` merges its PR, labels it, and closes it
/// later. A task waiting for a milestone (`ForgeDesMondes`' #214 on #20,
/// parked by its stop) starts again at the label, when the code it needs
/// is there, not at the close.
#[must_use]
pub fn still_blocks(blocker: &Issue) -> bool {
    blocker.is_open() && !waiting_merge(blocker)
}

/// What the task's `## Architecture` declares, if it reads.
#[must_use]
pub fn declaration(issue: &Issue) -> Option<Declaration> {
    sections::parse(&issue.body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
}

/// The unit the task's `## Architecture` declares, if any.
fn unit_of(issue: &Issue) -> Option<Unit> {
    declaration(issue).map(|declaration| declaration.unit)
}

/// Open, ready, not already delivered, and nothing blocking it.
///
/// `waiting_merge` is what prevents replaying a finished task: it stays open
/// until merge, so without this test the next round would pick it and repay
/// its three stages.
#[must_use]
pub fn runnable(issue: &Issue) -> bool {
    runnable_in(issue, None)
}

/// [`runnable`], its blockers judged on the plan's graph
/// ([`blockers_pending_in`]).
#[must_use]
pub fn runnable_in(issue: &Issue, graph: Option<&Graph>) -> bool {
    free_to_run(issue) && blockers_pending_in(issue, graph).is_empty()
}

/// Whether a lane's `--task` run may take this task.
///
/// [`runnable`], or a screen whose only pending blockers are milestones.
/// The router waived
/// those on the graph ([`screen_stands_on`]) before it started the lane,
/// which reads no graph.
#[must_use]
pub fn runnable_for_lane(issue: &Issue) -> bool {
    let screen = unit_of(issue) == Some(Unit::Composition);
    free_to_run(issue)
        && blockers_pending(issue)
            .iter()
            .all(|blocker| screen && blocker.has(labels::MILESTONE))
}

/// [`runnable`], its blockers aside.
fn free_to_run(issue: &Issue) -> bool {
    issue.is_open()
        && is_agent(issue)
        && is_ready(issue)
        && !waiting_merge(issue)
        && !review_pending(issue)
        && !awaits_arbitration(issue)
        && !awaits_decision(issue)
}

/// Whether nothing but milestones holds this task: [`runnable`] were its
/// milestone blockers landed — what a starved milestone's pull may free
/// (`routing::pulls`), never a sibling task or a person.
#[must_use]
pub fn held_only_by_milestones(issue: &Issue, graph: Option<&Graph>) -> bool {
    free_to_run(issue)
        && blockers_pending_in(issue, graph)
            .iter()
            .all(|blocker| blocker.has(labels::MILESTONE))
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
///
/// Judged on the plan's graph when there is one ([`runnable_in`]).
#[must_use]
pub fn runnable_tasks<'a>(issues: &'a [Issue], graph: Option<&Graph>) -> Vec<&'a Issue> {
    open_agent_tasks(issues)
        .into_iter()
        .filter(|issue| runnable_in(issue, graph))
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
            "{} {}: its PR is open and waits for its review and a green CI",
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
    declaration(issue).and_then(|declaration| declaration.held_aggregate().map(ToString::to_string))
}

/// What the task is, for the ledger: its unit, the profile it was cut for,
/// its side — read from its `## Architecture`. A task that declares nothing
/// is tagged empty, never guessed.
#[must_use]
pub fn tags(issue: &Issue) -> Tags {
    declaration(issue).map_or_else(Tags::default, |declaration| Tags {
        unit: declaration.unit.key().to_string(),
        profile: declaration.profile.clone().unwrap_or_default(),
        side: declaration.side().label().to_string(),
        milestone: String::new(),
    })
}

/// The tasks a milestone may run while the milestones before it in plan
/// order have not landed, judged on what each one `needs`.
///
/// What the mock-up stands in for is a **contract**: a Capability or a
/// `DataCapability` another milestone builds is mocked by its fixture until
/// built, so a task that needs only those runs ahead — a Micro-UI, but a
/// Capability too (`ForgeDesMondes`' #131 read nothing of another
/// milestone and was held a whole afternoon by its unit alone). For a
/// **screen** (a composition) the mock-up's fragment stands in for every
/// Micro-UI it lays out the same way: #219 composed five Micro-UIs of #18
/// and #24 and waited on both, and #143 waited on every producer because it
/// named rules — a rule's test stays pending while a unit it needs is not
/// built, and `rules-hold` proves it on every run after (R111's test
/// declares what it writes through since contest #298). What no fixture
/// stands in for is a **table, a Concept, or a Micro-UI another unit than a
/// screen builds on**: #118 (the `NoteImport` aggregate of #23) needed #18's
/// migrations, #216 (a Capability of #28) needed #20's `Session` and its
/// read crate — each is held, by that need, until the producer lands. A
/// need of the task's own milestone, one no milestone produces, or one the
/// graph does not know (a system's wiring) holds nothing. Without a graph
/// nothing is held: there is no producer to wait for.
///
/// `building` is every milestone before this one in plan order that has not
/// landed (`merge_train::unlanded_before`) — not only the producers the
/// graph's edges name: #28's #219 needed #24's `OralBrief` with no edge
/// from #28 to #24.
#[must_use]
pub fn mocked_only<'a>(
    tasks: &[&'a Issue],
    milestone: u64,
    graph: Option<&Graph>,
    building: &[u64],
) -> (Vec<&'a Issue>, Vec<u64>) {
    let (taken, held): (Vec<&Issue>, Vec<&Issue>) = tasks
        .iter()
        .partition(|task| runs_ahead(task, milestone, graph, building));
    (taken, held.into_iter().map(|task| task.number).collect())
}

/// Whether this task may run while `building` — the milestones before its
/// own that have not landed — still build: nothing it `needs` is an id one
/// of them produces that no mock-up stands in for ([`mocked_only`]).
#[must_use]
pub fn runs_ahead(task: &Issue, milestone: u64, graph: Option<&Graph>, building: &[u64]) -> bool {
    held_by(task, milestone, graph, building).is_empty()
}

/// The milestones of `building` this task waits on, each once, in the
/// order its needs name them: the producers of what it needs that no
/// mock-up stands in for ([`mocked_only`]). Empty when it runs ahead.
#[must_use]
pub fn held_by(task: &Issue, milestone: u64, graph: Option<&Graph>, building: &[u64]) -> Vec<u64> {
    held_needs(task, milestone, graph, building)
        .into_iter()
        .map(|(producer, _)| producer)
        .collect()
}

/// [`held_by`], each producer with the first id it holds the task on.
#[must_use]
pub fn held_needs(
    task: &Issue,
    milestone: u64,
    graph: Option<&Graph>,
    building: &[u64],
) -> Vec<(u64, String)> {
    let Some(graph) = graph else {
        return Vec::new();
    };
    let Some(declaration) = declaration(task) else {
        return Vec::new();
    };
    let screen = declaration.unit == Unit::Composition;
    let mut held: Vec<(u64, String)> = Vec::new();
    for id in &declaration.needs {
        let Some(interface) = graph.ids.get(id) else {
            continue;
        };
        if let Some(producer) = interface.producer
            && !mocked(interface.kind, screen)
            && producer != milestone
            && building.contains(&producer)
            && !held.iter().any(|(p, _)| *p == producer)
        {
            held.push((producer, id.clone()));
        }
    }
    held
}

/// Whether the mock-up stands in for what this task builds — a Micro-UI or
/// a screen, never the data layer: a blocker of this kind is not waived for
/// a Micro-UI ([`blockers_pending`]).
#[must_use]
pub fn mock_up_stands_in(task: &Issue) -> bool {
    !task.has(labels::DATA_LAYER)
        && matches!(unit_of(task), Some(Unit::MicroUi | Unit::Composition))
}

/// The runnable tasks a parallel watch may start now, given those on a lane
/// and what `may_start` admits of them, their blockers judged on `graph`
/// ([`runnable_in`]).
///
/// Two tasks that change the structure of the same aggregate run one after
/// the other — the data layer is shared by construction, and two changes of
/// one aggregate in parallel meet there. A task is held back while a running
/// task holds its aggregate, or while a startable task of a smaller number
/// does — so two free lanes do not take the same aggregate on one tick.
/// Different aggregates, and tasks holding none, run together. A task
/// `may_start` refuses holds nothing: `ForgeDesMondes`' #117 (the `Media`
/// aggregate, waiting on #18) held `Media` over #125, #128 and #130, which
/// could have run — and nothing ran.
#[must_use]
pub fn startable_tasks<'a>(
    issues: &'a [Issue],
    running: &[u64],
    graph: Option<&Graph>,
    may_start: &dyn Fn(&Issue) -> bool,
) -> Vec<&'a Issue> {
    let runnable = runnable_tasks(issues, graph);
    let mut held: Vec<String> = issues
        .iter()
        .filter(|issue| running.contains(&issue.number))
        .filter_map(held_aggregate)
        .collect();
    let mut startable = Vec::new();
    for task in runnable {
        if running.contains(&task.number) || !may_start(task) {
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
            "{pending_review} PR(s) open, waiting for their review and a green CI"
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
            runnable_tasks(&[held, task(6)], None)
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
    fn a_milestone_blocks_until_it_lands_closed_or_waiting_merge() {
        // #214 waited for #20, the milestone producing the session system:
        // pending while #20 builds, over once its PR is on the integration
        // branch — before it closes.
        let mut held = task(214);
        held.blocked_by = vec![issue(20, &[labels::MILESTONE, labels::TRIGGERED])];
        assert!(still_blocks(&held.blocked_by[0]));
        assert!(!runnable(&held));
        assert_eq!(blockers_pending(&held).len(), 1);
        held.blocked_by = vec![issue(
            20,
            &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
        )];
        assert!(!still_blocks(&held.blocked_by[0]));
        assert!(runnable(&held), "landed: the task starts again");
        held.blocked_by = vec![closed(issue(20, &[labels::MILESTONE]))];
        assert!(!still_blocks(&held.blocked_by[0]));
        assert!(runnable(&held));
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
    fn closes_needs_the_keyword_on_its_own_line_and_this_number() {
        // (body, closes #42)
        let cases = [
            ("du texte\nCloses #42\nencore", true),
            ("fixes #42", true),
            ("  resolves\t#42  ", true),
            // Stricter than GitHub, and rightly so: reading wider would close
            // an issue on a sentence mentioning it.
            ("this PR closes #42 among other things", false),
            ("see closes #42 below", false),
            // Closing another issue is not closing this one.
            ("Closes #420", false),
            ("Closes #4", false),
        ];
        for (body, expected) in cases {
            assert_eq!(closes(body, 42), expected, "{body:?}");
        }
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
    fn a_pr_names_a_task_by_its_declared_branch_or_its_number_in_the_title() {
        let mut declared = task(248);
        declared.body = "## Scope\n\nbranch: feat/battlemap-instance-aggregate\n".to_string();
        let on = |head: &str, title: &str| Pr {
            head: head.to_string(),
            title: title.to_string(),
            ..Pr::default()
        };
        assert!(pr_names_task(
            &on("feat/battlemap-instance-aggregate", "x"),
            &declared
        ));
        assert!(pr_names_task(
            &on("fix/other", "BattlemapInstance aggregate (#248)"),
            &declared
        ));
        assert!(pr_names_task(&on("fix/other", "Closes #248"), &declared));
        assert!(!pr_names_task(
            &on("fix/other", "another aggregate (#2480)"),
            &declared
        ));
        assert!(!pr_names_task(&on("fix/other", "no number"), &declared));
        assert!(
            !pr_names_task(&on("", "x"), &task(248)),
            "an empty head names an undeclared task no more"
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
            startable_tasks(&issues, running, None, &|_| true)
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
            runnable_tasks(&issues, None)
                .iter()
                .map(|t| t.number)
                .filter(|n| *n != 8)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_task_may_start_refuses_holds_no_aggregate_over_the_next() {
        let on = |number: u64, unit: &str| Issue {
            body: format!(
                "## Scope\n\nx\n\n## Architecture\n\nunit: {unit}\nsystem: media\naggregate: Media\neffect: update\ntouches: media.name"
            ),
            ..task(number)
        };
        let aggregate = on(117, "aggregate");
        let write = on(125, "data-capability");
        let other = on(128, "data-capability");
        let issues = [aggregate, write, other];
        let all: Vec<u64> = startable_tasks(&issues, &[], None, &|_| true)
            .iter()
            .map(|t| t.number)
            .collect();
        assert_eq!(
            all,
            [117],
            "#117 holds Media first; the writes wait their turn"
        );
        let without_117: Vec<u64> = startable_tasks(&issues, &[], None, &|t| t.number != 117)
            .iter()
            .map(|t| t.number)
            .collect();
        assert_eq!(
            without_117,
            [125],
            "#117 cannot start: it holds nothing, the first write takes Media"
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
        let numbers: Vec<u64> = startable_tasks(&issues, &[], None, &|_| true)
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
        let numbers: Vec<u64> = runnable_tasks(&issues, None)
            .iter()
            .map(|t| t.number)
            .collect();
        assert_eq!(numbers, [7, 8]);
    }

    #[test]
    fn ahead_work_is_what_the_mock_up_stands_in_for() {
        // #18 builds the Campaign table and campaign.listCharacters; #23
        // (the milestone judged) builds NoteImport; #24 builds OralBrief.
        let graph = Graph::parse(
            r#"{"version":1,"milestones":[
              {"number":18,"title":"campaigns","systems":["campaign"],"produces":["Campaign","campaign.listCharacters","campaign.saveEntity"],"consumes":[],"depends_on":[]},
              {"number":23,"title":"notes","systems":["ai"],"produces":["NoteImport","campaign.listImportedNotes"],"consumes":["Campaign"],"depends_on":[18]},
              {"number":24,"title":"brief","systems":["session"],"produces":["OralBrief"],"consumes":[],"depends_on":[]}
            ],"ids":{
              "Campaign":{"kind":"aggregate","producer":18,"state":"planned","version":1},
              "Character":{"kind":"aggregate","producer":18,"state":"planned","version":1},
              "campaign.listCharacters":{"kind":"capability","producer":18,"state":"mocked","version":1},
              "campaign.saveEntity":{"kind":"data-capability","producer":18,"state":"planned","version":1},
              "NoteImport":{"kind":"aggregate","producer":23,"state":"planned","version":1},
              "campaign.listImportedNotes":{"kind":"capability","producer":23,"state":"planned","version":1},
              "OralBrief":{"kind":"micro-ui","producer":24,"state":"planned","version":1},
              "Orphan":{"kind":"concept","producer":null,"state":"planned","version":1}
            }}"#,
        )
        .expect("a graph");
        let with = |number: u64, unit: &str, needs: &str| Issue {
            body: format!(
                "## Scope\n\nx\n\n## Architecture\n\nunit: {unit}\nsystem: ai\nneeds: {needs}"
            ),
            ..issue(number, &[labels::AGENT, labels::READY])
        };
        // Reads contracts of #18 and a table of its own milestone: the
        // fixtures stand in for the former, the latter is its to build.
        let ui = with(
            136,
            "micro-ui",
            "campaign.listCharacters, campaign.saveEntity, NoteImport",
        );
        let own_capability = with(131, "capability", "NoteImport, ai");
        // Reads #18's table: held until #18 lands — however it is a read.
        let reads_table = with(118, "aggregate", "Campaign, NoteImport");
        let reads_concept = with(140, "capability", "Campaign");
        // Composes a Micro-UI #24 builds: the mock-up's fragment stands in
        // for it, the screen is frozen now.
        let screen = with(219, "composition", "OralBrief, ai.listNotes");
        // A Micro-UI built on another Micro-UI #24 builds has no fragment
        // to stand in for it: held until #24 lands.
        let nested = with(150, "micro-ui", "OralBrief");
        // A screen that proves rules freezes now too: its rules stay
        // pending until their units are built, and rules-hold proves them.
        let mut proving = with(143, "composition", "SoundLibrary");
        proving
            .body
            .push_str("\n\n## Rules\n\n- R111 — business — sounds overlap");
        // A unit that is not a screen proves nothing by itself: its rules
        // line holds it no more than its needs do.
        let mut ruled = with(9, "capability", "NoteImport");
        ruled
            .body
            .push_str("\n\n## Rules\n\n- R5 — business — notes are listed");
        // An id nobody produces, or one the graph does not know, holds nothing.
        let free = with(7, "capability", "Orphan, wiring");
        let undeclared = issue(8, &[labels::AGENT, labels::READY]);
        let all = [
            &ui,
            &own_capability,
            &reads_table,
            &reads_concept,
            &screen,
            &nested,
            &proving,
            &ruled,
            &free,
            &undeclared,
        ];
        let (taken, held) = mocked_only(&all, 23, Some(&graph), &[18, 24]);
        assert_eq!(
            taken.iter().map(|t| t.number).collect::<Vec<_>>(),
            [136, 131, 219, 143, 9, 7, 8],
            "a contract is mocked, a screen's Micro-UI too; a table, a Concept or another \
             unit's Micro-UI of a building producer is not"
        );
        assert_eq!(held, [118, 140, 150]);
        assert_eq!(held_by(&reads_table, 23, Some(&graph), &[18, 24]), [18]);
        assert_eq!(held_by(&nested, 23, Some(&graph), &[18, 24]), [24]);
        let two_tables = with(161, "capability", "Campaign, Character, OralBrief");
        assert_eq!(
            held_by(&two_tables, 23, Some(&graph), &[18, 24]),
            [18, 24],
            "each producer once, in the order the needs name them"
        );
        // Once #18 has landed, the nested Micro-UI waits on #24 alone.
        let (_, held) = mocked_only(&all, 23, Some(&graph), &[24]);
        assert_eq!(held, [150]);
        // Every producer landed: nothing waits.
        let (_, held) = mocked_only(&all, 23, Some(&graph), &[]);
        assert!(held.is_empty(), "{held:?}");
        // No graph: no producer to wait for.
        let (taken, held) = mocked_only(&all, 23, None, &[18, 24]);
        assert_eq!(taken.len(), all.len());
        assert!(held.is_empty(), "held should be empty");
        let (none, nothing) = mocked_only(&[], 23, Some(&graph), &[18]);
        assert!(none.is_empty() && nothing.is_empty(), "should be empty");
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
        assert!(
            !runnable(&freeze),
            "a screen's agent blockers still hold it: only a Micro-UI's are waived"
        );
    }

    #[test]
    fn a_screen_waives_a_milestone_blocker_the_mock_up_stands_in_for() {
        // #219 composes #24's OralBrief and reads #24's brief.list; #23's
        // table NoteImport is no fragment.
        let graph = Graph::parse(
            r#"{"version":1,"milestones":[
              {"number":23,"title":"notes","systems":["ai"],"produces":["NoteImport"],"consumes":[],"depends_on":[]},
              {"number":24,"title":"brief","systems":["session"],"produces":["OralBrief","brief.list"],"consumes":[],"depends_on":[]}
            ],"ids":{
              "NoteImport":{"kind":"aggregate","producer":23,"state":"planned","version":1},
              "OralBrief":{"kind":"micro-ui","producer":24,"state":"planned","version":1},
              "brief.list":{"kind":"capability","producer":24,"state":"planned","version":1}
            }}"#,
        )
        .expect("a graph");
        let with = |number: u64, unit: &str, needs: &str, blockers: &[u64]| Issue {
            body: format!(
                "## Scope\n\nx\n\n## Architecture\n\nunit: {unit}\nsystem: session\nneeds: {needs}"
            ),
            blocked_by: blockers
                .iter()
                .map(|n| issue(*n, &[labels::MILESTONE]))
                .collect(),
            ..issue(number, &[labels::AGENT, labels::READY])
        };
        let screen = with(219, "composition", "OralBrief, brief.list", &[24]);
        assert!(!runnable(&screen), "without a graph the blocker holds");
        assert!(runnable_in(&screen, Some(&graph)), "split is all it needs");
        assert!(runnable_for_lane(&screen), "the lane trusts the router");
        assert_eq!(
            runnable_tasks(std::slice::from_ref(&screen), Some(&graph)).len(),
            1
        );
        // A table of the blocker is no fragment: it still holds.
        let table = with(220, "composition", "OralBrief, NoteImport", &[23, 24]);
        assert_eq!(
            blockers_pending_in(&table, Some(&graph))
                .iter()
                .map(|b| b.number)
                .collect::<Vec<_>>(),
            [23]
        );
        // A blocker it needs nothing of holds: no need explains it.
        let unexplained = with(221, "composition", "OralBrief", &[23]);
        assert!(!runnable_in(&unexplained, Some(&graph)));
        // A Micro-UI does not stand on another Micro-UI's fragment.
        let nested = with(222, "micro-ui", "OralBrief", &[24]);
        assert!(!runnable_in(&nested, Some(&graph)));
        assert!(!runnable_for_lane(&nested), "only a screen's milestones");
        // A screen also held by a sibling task is not the lane's to take.
        let mut sibling = with(223, "composition", "OralBrief", &[24]);
        sibling.blocked_by.push(issue(218, &[labels::AGENT]));
        assert!(!runnable_for_lane(&sibling));
        assert!(!held_only_by_milestones(&sibling, Some(&graph)));
        assert!(held_only_by_milestones(&table, Some(&graph)));
        let mut parked = table;
        parked.labels.retain(|l| l != labels::READY);
        assert!(!held_only_by_milestones(&parked, Some(&graph)));
    }
}
