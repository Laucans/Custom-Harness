//! Which workflow runs next, decided from a snapshot already read.
//!
//! Pure: no `gh` call lives here. The router (the launcher's polling loop)
//! does the reads this needs — including the facts that require more than
//! a flat issue list (`lowest_roadmap_has_milestone`, `ready_to_merge`,
//! `dev_loop_milestone`) — and hands this function the result. That is
//! what makes the decision testable without a fake adapter.
//!
//! Priority, top to bottom: a roadmap item that's `harness:ready` and has no
//! milestone yet outranks everything, because nothing else can be planned
//! against it; then a `harness:ready` rules' task, then a decision asked
//! (`harness:needs-decision`, the `decide` workflow), since its tests are what
//! every development conforms to — and **while a rules' task is open, ready
//! or not, no development task starts** ([`rules_first`]); a milestone ready
//! to split comes next, once no refinement is
//! pending on it; then the two things
//! that can be wrong with a pull request already in flight — a red one to
//! repair before a green one to review, since reviewing a change whose CI
//! is broken reviews something about to change; then merging a reviewed,
//! green task PR into its milestone; then finishing a milestone that's
//! done; then a task whose graph is in question, for the arbiter; then a
//! runnable task; then a pending refinement (business, then technical);
//! and last a file's test debt. Structural decomposition before
//! unblocking what's in flight before closing out before delivering before
//! preparing the next delivery — and paying back the tests only when nothing
//! else waits, since the debt is real but never blocks anyone.
//!
//! A parallel watch does not stop at the route: it first fills every free
//! lane, the runnable tasks before splits and refinements ([`lane_work`]),
//! so a pending refinement never keeps a task off a lane that sits free.
//!
//! `harness:ready` is the same tap at every issue level: it means "plan
//! this" on a roadmap item, "split this" on a milestone, and "build this" on
//! a task. The factory's Initialize opens it on the roadmap item, the planner
//! on its milestones, split on its agent tasks: **the harness opens the tap,
//! a human closes it** by taking the label off — after the fact, told by the
//! notifications. `harness:to-review` and `harness:pr-fix` are the same kind
//! of tap on a **pull request**, `harness:test-debt` on the issue the CI's
//! mutants rotation opens, `harness:needs-decision` on any issue a decision
//! is asked on. The router never acts on something no tap is open on.

use harness_core::domain::{Issue, Outcome, Pr};
use harness_core::ports::shell::github::{Activity, GitHub};
use harness_core::traces::{Event, Logbook};

pub use harness_architecture::graph::Graph;

use crate::common::labels;

/// What the router decides to run next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Open the milestones of this roadmap item — `planner`.
    Planner {
        /// The roadmap issue's number.
        roadmap: u64,
    },
    /// Open the tasks of this milestone — `split`.
    Split {
        /// The milestone issue's number.
        milestone: u64,
    },
    /// Attempt one repair on this red PR — `pr_fix`.
    PrFix {
        /// The PR, as the ports name one.
        pr: String,
    },
    /// Review this PR — `pr_review`.
    PrReview {
        /// The PR, as the ports name one.
        pr: String,
        /// The branch it targets. Carried so the review judges its own skip
        /// rules against the branch this PR actually has, rather than
        /// against a fixed integration branch a task PR never targets.
        base: String,
    },
    /// Merge this task's reviewed, green PR into its milestone branch —
    /// `milestone_merge`.
    MergeIntoMilestone {
        /// The task's number.
        task: u64,
        /// The PR, as the ports name one.
        pr: String,
        /// The branch it targets — the milestone's.
        base: String,
    },
    /// Open or merge this milestone's PR into the base branch —
    /// `main_agent_merge`.
    MergeMainAgent {
        /// The milestone issue's number.
        milestone: u64,
    },
    /// Rewrite this issue's body — `refinement`.
    Refinement {
        /// The issue's number.
        issue: u64,
    },
    /// Write the technical sections of this issue's body — `refinement`,
    /// technical phase.
    TechRefinement {
        /// The issue's number.
        issue: u64,
    },
    /// Settle this task's graph question, or hand it to a human — `arbiter`.
    Arbiter {
        /// The task's number.
        issue: u64,
    },
    /// Run the dev loop on this milestone's board.
    DevLoop {
        /// The milestone issue's number — so the dispatcher can derive the
        /// branch the dev loop works on (`common::branching`), rather than
        /// a fixed integration branch.
        milestone: u64,
    },
    /// Take the decision this issue asks for — `decide`.
    Decide {
        /// The issue's number.
        issue: u64,
    },
    /// Evolve the contract a consumer needs changed — `contract_manager`.
    ContractChange {
        /// The `harness:contract-change` issue's number.
        issue: u64,
    },
    /// Write the rules and their tests, or settle a contest — `rules`.
    Rules {
        /// The rules' issue number.
        issue: u64,
    },
    /// Write the tests this issue's file owes — `test_debt`.
    TestDebt {
        /// The test-debt issue's number.
        issue: u64,
    },
    /// Nothing is ready to run.
    Nothing,
}

/// Everything [`decide`] needs, already read.
pub struct Snapshot {
    /// Open `harness:roadmap` issues, any order.
    pub roadmap: Vec<Issue>,
    /// Whether the lowest-numbered open roadmap issue already has at least
    /// one milestone under it. Meaningless, and never read, if `roadmap`
    /// is empty or that issue isn't `harness:ready` yet.
    pub lowest_roadmap_has_milestone: bool,
    /// Open `harness:milestone` issues, any order.
    pub milestones: Vec<Issue>,
    /// The open PR carrying `harness:pr-fix` whose CI has actually broken,
    /// if any — the router pairs the label with a failing-check read, which
    /// a label list cannot answer alone.
    pub pr_to_fix: Option<Pr>,
    /// The open PR carrying `harness:to-review` that the review would not
    /// skip, if any — the router applies the review's own skip rules
    /// (`pr_review::data::skip_rules`) so a PR that would skip never costs a
    /// mounted checkout per poll.
    pub pr_to_review: Option<Pr>,
    /// The task whose PR is open on its milestone, reviewed and green, with
    /// that PR — what `milestone_merge` merges.
    pub task_to_merge: Option<(u64, Pr)>,
    /// The lowest-numbered open milestone whose tasks are all closed and
    /// that isn't `harness:waiting-merge` yet, if any — computed by the
    /// router against each milestone's own sub-issues, which a flat list
    /// cannot answer alone.
    pub ready_to_merge: Option<u64>,
    /// Open `harness:refinement` issues, any order.
    pub refining: Vec<Issue>,
    /// Open `harness:tech-refinement` issues, any order.
    pub tech_refining: Vec<Issue>,
    /// Open `harness:needs-ai-decision` issues, any order.
    pub arbitrating: Vec<Issue>,
    /// The milestones building now whose board has a runnable task, the
    /// critical path's first ([`critical_path`]) — empty when no open
    /// milestone has one.
    pub dev_loop_milestones: Vec<u64>,
    /// The plan's graph (contract R), read off the integration branch, when
    /// the plan wrote one.
    pub graph: Option<Graph>,
    /// Open `harness:contract-change` issues, any order.
    pub contract_changes: Vec<Issue>,
    /// Open `harness:test-debt` issues, any order.
    pub test_debt: Vec<Issue>,
    /// Open `harness:rules` issues, any order.
    pub rules: Vec<Issue>,
    /// Open `harness:needs-decision` issues, any order.
    pub deciding: Vec<Issue>,
}

/// The rules' task development waits on: the lowest open `harness:rules`
/// issue, ready or not — a failed attempt hands it to a human, and the
/// tests still come first.
#[must_use]
pub fn rules_first(snapshot: &Snapshot) -> Option<u64> {
    lowest_open(&snapshot.rules).map(|issue| issue.number)
}

/// Decides the next route from a snapshot.
#[must_use]
pub fn decide(snapshot: &Snapshot) -> Route {
    if let Some(lowest) = plannable_roadmap(&snapshot.roadmap)
        && lowest.has(labels::READY)
        && !snapshot.lowest_roadmap_has_milestone
    {
        return Route::Planner {
            roadmap: lowest.number,
        };
    }
    if let Some(issue) = lowest_open(&snapshot.rules).filter(|issue| issue.has(labels::READY)) {
        return Route::Rules {
            issue: issue.number,
        };
    }
    // A decision is cheap, and it unblocks a task, a PR or a split: before
    // any of them.
    if let Some(issue) = lowest_open(&snapshot.deciding) {
        return Route::Decide {
            issue: issue.number,
        };
    }
    // A contract a consumer waits on blocks a task already cut: evolved
    // before anything new is cut against the old version.
    if let Some(issue) = lowest_open(&snapshot.contract_changes) {
        return Route::ContractChange {
            issue: issue.number,
        };
    }
    if let Some(milestone) = splits(&snapshot.milestones, snapshot.graph.as_ref()).first() {
        return Route::Split {
            milestone: *milestone,
        };
    }
    if let Some(pr) = &snapshot.pr_to_fix {
        return Route::PrFix { pr: pr.num.clone() };
    }
    if let Some(pr) = &snapshot.pr_to_review {
        return Route::PrReview {
            pr: pr.num.clone(),
            base: pr.base.clone(),
        };
    }
    if let Some((task, pr)) = &snapshot.task_to_merge {
        return Route::MergeIntoMilestone {
            task: *task,
            pr: pr.num.clone(),
            base: pr.base.clone(),
        };
    }
    if let Some(milestone) = snapshot.ready_to_merge {
        return Route::MergeMainAgent { milestone };
    }
    // A graph in question before a task is picked: the arbitration is
    // cheap, and the task it settles — or the one that waits on it — may
    // be the next one the loop would otherwise start on incomplete blockers.
    if let Some(issue) = lowest_open(&snapshot.arbitrating) {
        return Route::Arbiter {
            issue: issue.number,
        };
    }
    // Delivering before preparing: a runnable task is work already decided,
    // and a pending refinement only ever prepares the next one.
    if let Some(milestone) = snapshot.dev_loop_milestones.first()
        && rules_first(snapshot).is_none()
    {
        return Route::DevLoop {
            milestone: *milestone,
        };
    }
    if let Some(issue) = lowest_open(&snapshot.refining) {
        return Route::Refinement {
            issue: issue.number,
        };
    }
    if let Some(issue) = lowest_open(&snapshot.tech_refining) {
        return Route::TechRefinement {
            issue: issue.number,
        };
    }
    if let Some(issue) = lowest_open(&snapshot.test_debt) {
        return Route::TestDebt {
            issue: issue.number,
        };
    }
    Route::Nothing
}

/// What a parallel watch hands its free lanes, in the order it hands it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneWork {
    /// The milestones whose runnable tasks take the free lanes first, in
    /// the order they are served: the critical path's, then the rest.
    pub tasks_of: Vec<u64>,
    /// Whether a milestone waits on its split or an issue on its business
    /// refinement — what takes the lanes the tasks left.
    pub preparation: bool,
}

/// What a parallel watch fills its free lanes with, whatever [`decide`]
/// chose: the runnable tasks first, then splits and refinements.
///
/// [`decide`] is one route per tick, and a lane route used to fill the lanes
/// with its own kind only: while one refinement was pending, no task reached
/// a lane, however many lanes sat free.
#[must_use]
pub fn lane_work(snapshot: &Snapshot) -> LaneWork {
    LaneWork {
        tasks_of: if rules_first(snapshot).is_none() {
            snapshot.dev_loop_milestones.clone()
        } else {
            Vec::new()
        },
        preparation: snapshot.refining.iter().any(Issue::is_open)
            || !splits(&snapshot.milestones, snapshot.graph.as_ref()).is_empty(),
    }
}

/// How many quiet ticks a poll lets pass before it reads the board in full
/// anyway — the valve for a change the activity read cannot see.
pub const FULL_READ_EVERY: u32 = 10;

/// What a poll keeps between two ticks to know whether to read the board.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pulse {
    /// The tag of the last activity read, handed back as `If-None-Match`.
    pub etag: Option<String>,
    /// Whether the last full read saw a pull request the board waits on —
    /// a check run or a review changes nothing on an issue, so those are
    /// read in full every tick while one is open.
    pub watching_ci: bool,
    /// Ticks since the last full read.
    pub quiet_ticks: u32,
    /// The rest after a `QUOTA` stop.
    pub rest: Rest,
    /// How many times this watch marked each issue `harness:ready` itself
    /// (`--no-human-gate`).
    pub taps: std::collections::BTreeMap<u64, u32>,
}

/// The first rest after a `QUOTA` stop, in ticks.
pub const FIRST_REST: u32 = 4;

/// The longest rest, in ticks: each stop in a row doubles the rest up to
/// here. At the default interval that is about half an hour.
pub const LONGEST_REST: u32 = 64;

/// How long the watch rests after a `QUOTA` stop, in ticks.
///
/// A subscription window that is exhausted does not come back on the next
/// tick: trying again every thirty seconds buys nothing and leaves one
/// `QUOTA` row per tick in the ledgers. The rest doubles with every stop in a
/// row — the window may take hours — and starts again from
/// [`FIRST_REST`] once a dispatch went through.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Rest {
    /// Ticks still to wait before anything is dispatched again.
    pub ticks_left: u32,
    /// `QUOTA` stops in a row.
    pub strikes: u32,
}

impl Rest {
    /// A `QUOTA` stop: rests twice as long as the last one, up to
    /// [`LONGEST_REST`]. Returns the ticks to wait.
    pub fn struck(&mut self) -> u32 {
        self.strikes = self.strikes.saturating_add(1);
        let doubled = 2u32.saturating_pow(self.strikes.saturating_sub(1).min(16));
        self.ticks_left = FIRST_REST.saturating_mul(doubled).min(LONGEST_REST);
        self.ticks_left
    }

    /// A tick passes: whether the rest is over — true when there was none.
    pub const fn passes(&mut self) -> bool {
        if self.ticks_left == 0 {
            return true;
        }
        self.ticks_left -= 1;
        false
    }

    /// A dispatch went through: the next rest starts again from the first.
    pub const fn cleared(&mut self) {
        self.strikes = 0;
        self.ticks_left = 0;
    }

    /// Whether the watch is resting.
    #[must_use]
    pub const fn resting(&self) -> bool {
        self.ticks_left > 0
    }
}

impl Pulse {
    /// What the pulse remembers after a full read.
    pub fn read_in_full(&mut self, etag: Option<String>, watching_ci: bool) {
        self.etag = etag;
        self.watching_ci = watching_ci;
        self.quiet_ticks = 0;
    }

    /// What the pulse remembers after a tick that read nothing.
    pub const fn stayed_quiet(&mut self) {
        self.quiet_ticks = self.quiet_ticks.saturating_add(1);
    }

    /// Whether this watch may mark `issue` ready once more, counting it if
    /// so: [`TAP_REOPENS`] times at most, so an issue whose attempt keeps
    /// failing is not paid for again and again.
    pub fn may_open(&mut self, issue: u64) -> bool {
        let opened = self.taps.entry(issue).or_insert(0);
        if *opened >= TAP_REOPENS {
            return false;
        }
        *opened += 1;
        true
    }
}

/// How many times one watch marks an issue `harness:ready` itself, at most.
pub const TAP_REOPENS: u32 = 2;

/// What a watch with no human gate marks `harness:ready` itself.
///
/// Every open roadmap item, milestone and rules' task without the label,
/// and what it is, for the journal. Never an issue a person must act on
/// (`harness:human`), one a decision is pending on, nor a milestone already
/// split (`harness:triggered`), which would be split again.
#[must_use]
pub fn taps_to_open(snapshot: &Snapshot) -> Vec<(u64, &'static str)> {
    let closed_to_the_watch = |issue: &Issue| {
        !issue.is_open()
            || issue.has(labels::READY)
            || issue.has(labels::HUMAN)
            || issue.has(labels::NEEDS_DECISION)
    };
    let mut taps: Vec<(u64, &'static str)> = Vec::new();
    for (issues, what) in [
        (&snapshot.roadmap, "roadmap item"),
        (&snapshot.milestones, "milestone"),
        (&snapshot.rules, "rules' task"),
    ] {
        for issue in issues {
            if closed_to_the_watch(issue) || (what == "milestone" && issue.has(labels::TRIGGERED)) {
                continue;
            }
            if !taps.iter().any(|(n, _)| *n == issue.number) {
                taps.push((issue.number, what));
            }
        }
    }
    taps.sort_unstable_by_key(|(n, _)| *n);
    taps
}

/// No human gate: what waits for `harness:ready` is marked so on the board
/// and in `found`, before the route is decided, so this very tick takes it.
///
/// # Errors
///
/// GitHub refused a label: what was marked before it stays marked.
pub async fn open_taps(
    gh: &dyn GitHub,
    found: &mut Snapshot,
    pulse: &mut Pulse,
    log: &Logbook,
) -> Outcome<()> {
    for (issue, what) in taps_to_open(found) {
        if !pulse.may_open(issue) {
            continue;
        }
        gh.add_label(issue, labels::READY).await?;
        mark_ready(found, issue);
        log.event(&Event::TapOpened {
            issue,
            what: what.to_string(),
        });
    }
    Ok(())
}

/// The snapshot as it reads once `number` is marked `harness:ready`.
pub fn mark_ready(snapshot: &mut Snapshot, number: u64) {
    for issue in snapshot
        .roadmap
        .iter_mut()
        .chain(snapshot.milestones.iter_mut())
        .chain(snapshot.rules.iter_mut())
        .filter(|issue| issue.number == number && !issue.has(labels::READY))
    {
        issue.labels.push(labels::READY.to_string());
    }
}

/// Whether this tick may skip the full read of the board.
///
/// Only when the one conditional read said nothing moved, no pull request's
/// checks or review are being waited on, no lane was just freed — a lane
/// freed with the board unchanged is a crashed lane whose work is still
/// due — and the valve has not run out.
#[must_use]
pub const fn may_skip(pulse: &Pulse, activity: &Activity, freed_lanes: usize) -> bool {
    matches!(activity, Activity::Unchanged)
        && !pulse.watching_ci
        && freed_lanes == 0
        && pulse.quiet_ticks < FULL_READ_EVERY
}

/// Whether this milestone is a candidate for a split, its blockers aside:
/// open, `harness:ready`, and no refinement pending.
///
/// A milestone still waiting on its refinement is split once that round is
/// written: tasks cut from the planner's draft would miss it.
#[must_use]
pub fn split_candidate(milestone: &Issue) -> bool {
    milestone.is_open() && milestone.has(labels::READY) && !milestone.has(labels::REFINEMENT)
}

/// Whether this milestone is ready to be split now.
///
/// A candidate, every milestone it is `blocked_by` delivered — closed, or
/// `waiting-merge` with its code on the integration branch — and what the
/// plan says it waits on:
///
/// - a milestone the plan's graph knows (contract R) waits on the producers
///   of what it consumes, until each is **split** (`harness:triggered`) or
///   delivered: from there the contract it reads is in place, mocked until
///   built, and the two build side by side;
/// - one the graph does not know waits on every milestone before it (a
///   lower number) delivered, the order of a plan written without a graph
///   being the dependency it does not write.
///
/// `milestone.blocked_by` must have been read ([`GitHub::with_blockers`]);
/// empty reads as nothing blocking.
///
/// [`GitHub::with_blockers`]: harness_core::ports::shell::github::GitHub::with_blockers
#[must_use]
pub fn splittable(milestone: &Issue, milestones: &[Issue], graph: Option<&Graph>) -> bool {
    let delivered = |other: &Issue| other.is_closed() || other.has(labels::WAITING_MERGE);
    if !split_candidate(milestone) || !milestone.blocked_by.iter().all(delivered) {
        return false;
    }
    graph
        .and_then(|graph| graph.node(milestone.number))
        .map_or_else(
            || {
                milestones
                    .iter()
                    .filter(|other| other.number < milestone.number)
                    .all(delivered)
            },
            |node| {
                graph
                    .map(|graph| graph.waits_on(node.number))
                    .unwrap_or_default()
                    .iter()
                    .all(|producer| {
                        milestones
                            .iter()
                            .find(|other| other.number == *producer)
                            .is_none_or(|other| delivered(other) || other.has(labels::TRIGGERED))
                    })
            },
        )
}

/// How many milestones build at once, at most, beyond the critical path's:
/// the ahead work is taken on the resources left, never instead of what the
/// path waits on.
pub const MAX_CURRENT: usize = 3;

/// The milestones to split now, in the order to split them.
///
/// The critical path's first ([`critical_path`]), then the rest by number —
/// those only while fewer than [`MAX_CURRENT`] milestones build
/// (`harness:triggered`, not delivered).
#[must_use]
pub fn splits(milestones: &[Issue], graph: Option<&Graph>) -> Vec<u64> {
    let building = milestones
        .iter()
        .filter(|m| m.is_open() && m.has(labels::TRIGGERED) && !m.has(labels::WAITING_MERGE))
        .count();
    let path = critical_path(milestones, graph);
    let mut found: Vec<u64> = milestones
        .iter()
        .filter(|m| splittable(m, milestones, graph))
        .map(|m| m.number)
        .collect();
    by_critical_path(&mut found, &path);
    let mut room = MAX_CURRENT.saturating_sub(building);
    found.retain(|number| {
        if path.contains(number) {
            return true;
        }
        let kept = room > 0;
        room = room.saturating_sub(1);
        kept
    });
    found
}

/// The critical path of the plan among what is left to build.
///
/// The chain of milestones, each producing what the next consumes, that
/// weighs the most — a delivered milestone weighs nothing, an open one what
/// it produces. What a free lane serves first; empty without a graph.
#[must_use]
pub fn critical_path(milestones: &[Issue], graph: Option<&Graph>) -> Vec<u64> {
    let Some(graph) = graph else {
        return Vec::new();
    };
    let delivered = |number: u64| {
        milestones
            .iter()
            .find(|m| m.number == number)
            .is_none_or(|m| m.is_closed() || m.has(labels::WAITING_MERGE))
    };
    graph.critical_path(&|number| {
        if delivered(number) {
            0
        } else {
            graph.node(number).map_or(1, |node| {
                u64::try_from(node.produces.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(1)
            })
        }
    })
}

/// Sorts milestone numbers: the critical path's first, in the path's order,
/// then the rest by number.
pub fn by_critical_path(numbers: &mut [u64], path: &[u64]) {
    numbers.sort_by_key(|number| {
        (
            path.iter()
                .position(|on_path| on_path == number)
                .unwrap_or(usize::MAX),
            *number,
        )
    });
}

/// Where the plan's graph is read from, on the integration branch.
pub const GRAPH_PATH: &str = harness_architecture::graph::PATH;

/// The graph a `plan/graph.json` text holds, if it reads; `None` for no
/// file or one that does not parse — a plan without a graph.
#[must_use]
pub fn parse_graph(text: Option<&str>) -> Option<Graph> {
    text.and_then(|text| Graph::parse(text).ok())
}

/// The roadmap item the planner works on: the lowest open one not already
/// delivered — `harness:waiting-merge`, every milestone landed, waiting for
/// a human to release it and close it.
#[must_use]
pub fn plannable_roadmap(roadmap: &[Issue]) -> Option<&Issue> {
    roadmap
        .iter()
        .filter(|issue| issue.is_open() && !issue.has(labels::WAITING_MERGE))
        .min_by_key(|issue| issue.number)
}

/// The lowest-numbered open issue, if any.
fn lowest_open(issues: &[Issue]) -> Option<&Issue> {
    issues
        .iter()
        .filter(|issue| issue.is_open())
        .min_by_key(|issue| issue.number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_human_gate_every_waiting_tap_opens_but_what_a_person_or_a_decision_holds() {
        let mut snapshot = empty();
        snapshot.roadmap = vec![
            issue(4, &[labels::ROADMAP]),
            issue(1, &[labels::ROADMAP, labels::READY]),
        ];
        snapshot.milestones = vec![
            issue(6, &[labels::MILESTONE, labels::SPEC_WRITTEN]),
            issue(5, &[labels::MILESTONE]),
            issue(7, &[labels::MILESTONE, labels::TRIGGERED]),
            issue(8, &[labels::MILESTONE, labels::NEEDS_DECISION]),
            Issue {
                state: "closed".to_string(),
                ..issue(9, &[labels::MILESTONE])
            },
        ];
        snapshot.rules = vec![
            issue(2, &[labels::RULES]),
            issue(3, &[labels::RULES, labels::HUMAN]),
        ];
        assert_eq!(
            taps_to_open(&snapshot),
            vec![
                (2, "rules' task"),
                (4, "roadmap item"),
                (5, "milestone"),
                (6, "milestone")
            ]
        );
        mark_ready(&mut snapshot, 5);
        mark_ready(&mut snapshot, 5);
        assert_eq!(
            snapshot.milestones[1].labels,
            vec![labels::MILESTONE.to_string(), labels::READY.to_string()],
            "marked once"
        );
        assert_eq!(taps_to_open(&snapshot).len(), 3);
    }

    #[tokio::test]
    async fn the_taps_are_opened_on_the_board_and_in_the_snapshot_twice_at_most() {
        use crate::common::fake_github::{FakeGitHub, Wrote};
        let gh = FakeGitHub::default();
        let mut found = empty();
        found.milestones = vec![issue(5, &[labels::MILESTONE])];
        let mut pulse = Pulse::default();
        open_taps(&gh, &mut found, &mut pulse, &Logbook::null())
            .await
            .expect("opened");
        assert_eq!(
            gh.writes(),
            vec![Wrote::Label(5, labels::READY.to_string())]
        );
        assert!(found.milestones[0].has(labels::READY), "this tick takes it");
        for _ in 0..3 {
            found.milestones = vec![issue(5, &[labels::MILESTONE])];
            open_taps(&gh, &mut found, &mut pulse, &Logbook::null())
                .await
                .expect("opened");
        }
        assert_eq!(gh.writes().len(), 2, "twice at most: {:?}", gh.writes());
        gh.refuse_label(labels::READY);
        found.milestones = vec![issue(6, &[labels::MILESTONE])];
        assert!(
            open_taps(&gh, &mut found, &mut pulse, &Logbook::null())
                .await
                .is_err()
        );
        assert!(
            !found.milestones[0].has(labels::READY),
            "not marked when refused"
        );
    }

    #[test]
    fn a_milestone_waits_for_the_ones_before_it_to_land_before_its_split() {
        // ForgeDesMondes: the skeleton (#16) blocked nobody, and #17, #18 were
        // cut against a repository it had not built yet.
        let skeleton = issue(16, &[labels::MILESTONE, labels::TRIGGERED]);
        let next = issue(17, &[labels::MILESTONE, labels::READY]);
        let mut snapshot = empty();
        snapshot.milestones = vec![skeleton, next.clone()];
        assert!(
            !splittable(&next, &snapshot.milestones, None),
            "#16 still open"
        );
        assert_ne!(decide(&snapshot), Route::Split { milestone: 17 });
        let landed = |labels: &[&str], state: &str| Issue {
            state: state.to_string(),
            ..issue(16, labels)
        };
        for done in [
            landed(&[labels::MILESTONE, labels::WAITING_MERGE], "open"),
            landed(&[labels::MILESTONE], "closed"),
        ] {
            snapshot.milestones = vec![done, next.clone()];
            assert!(
                splittable(&next, &snapshot.milestones, None),
                "#16 delivered"
            );
            assert_eq!(decide(&snapshot), Route::Split { milestone: 17 });
        }
        assert!(
            splittable(
                &issue(16, &[labels::MILESTONE, labels::READY]),
                &[next],
                None
            ),
            "a later one holds nobody"
        );
    }

    #[test]
    fn a_watch_opens_one_issue_s_tap_twice_at_most() {
        let mut pulse = Pulse::default();
        assert!(pulse.may_open(2));
        assert!(pulse.may_open(2));
        assert!(!pulse.may_open(2), "a third attempt waits on a person");
        assert!(pulse.may_open(5), "each issue its own count");
    }

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn empty() -> Snapshot {
        Snapshot {
            roadmap: Vec::new(),
            lowest_roadmap_has_milestone: false,
            milestones: Vec::new(),
            pr_to_fix: None,
            pr_to_review: None,
            task_to_merge: None,
            ready_to_merge: None,
            refining: Vec::new(),
            tech_refining: Vec::new(),
            arbitrating: Vec::new(),
            dev_loop_milestones: Vec::new(),
            graph: None,
            contract_changes: Vec::new(),
            test_debt: Vec::new(),
            rules: Vec::new(),
            deciding: Vec::new(),
        }
    }

    fn pr(num: &str, base: &str) -> Pr {
        Pr {
            num: num.to_string(),
            base: base.to_string(),
            ..Pr::default()
        }
    }

    #[test]
    fn a_green_task_pr_is_merged_into_its_milestone_before_the_milestone_moves_up() {
        let snapshot = Snapshot {
            task_to_merge: Some((64, pr("68", "milestone/52-socle"))),
            ready_to_merge: Some(52),
            refining: vec![issue(70, &[labels::REFINEMENT])],
            ..empty()
        };
        assert_eq!(
            decide(&snapshot),
            Route::MergeIntoMilestone {
                task: 64,
                pr: "68".to_string(),
                base: "milestone/52-socle".to_string(),
            }
        );
    }

    #[test]
    fn a_ready_roadmap_item_with_no_milestone_outranks_everything() {
        let snapshot = Snapshot {
            roadmap: vec![
                issue(9, &[labels::ROADMAP, labels::READY]),
                issue(4, &[labels::ROADMAP, labels::READY]),
            ],
            lowest_roadmap_has_milestone: false,
            graph: None,
            contract_changes: Vec::new(),
            milestones: vec![issue(5, &[labels::MILESTONE, labels::READY])],
            pr_to_fix: Some(pr("32", "main_agent")),
            pr_to_review: Some(pr("33", "main_agent")),
            task_to_merge: Some((40, pr("41", "milestone/3-x"))),
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            tech_refining: vec![],
            arbitrating: Vec::new(),
            dev_loop_milestones: vec![3],
            test_debt: vec![issue(8, &[labels::TEST_DEBT])],
            rules: vec![issue(2, &[labels::RULES, labels::READY])],
            deciding: vec![issue(3, &[labels::NEEDS_DECISION])],
        };
        assert_eq!(decide(&snapshot), Route::Planner { roadmap: 4 });
    }

    #[test]
    fn a_ready_rules_task_comes_next_and_an_open_one_holds_every_development() {
        let mut settled = issue(1, &[labels::RULES, labels::READY]);
        settled.state = "closed".to_string();
        let ready = Snapshot {
            rules: vec![settled.clone(), issue(5, &[labels::RULES, labels::READY])],
            milestones: vec![issue(9, &[labels::MILESTONE, labels::READY])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(decide(&ready), Route::Rules { issue: 5 });
        // Taken, failed, waiting on a human: the development still waits.
        let waiting = Snapshot {
            rules: vec![issue(5, &[labels::RULES, labels::HUMAN])],
            dev_loop_milestones: vec![3],
            refining: vec![issue(6, &[labels::REFINEMENT])],
            ..empty()
        };
        assert_eq!(rules_first(&waiting), Some(5));
        assert_eq!(decide(&waiting), Route::Refinement { issue: 6 });
        assert!(lane_work(&waiting).tasks_of.is_empty());
        // Settled: the development goes on.
        let settled = Snapshot {
            rules: vec![settled],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(rules_first(&settled), None);
        assert_eq!(decide(&settled), Route::DevLoop { milestone: 3 });
        assert_eq!(lane_work(&settled).tasks_of, [3]);
    }

    #[test]
    fn test_debt_waits_until_nothing_else_does_then_takes_the_lowest_open() {
        let mut closed = issue(3, &[labels::TEST_DEBT]);
        closed.state = "closed".to_string();
        let debt = vec![
            issue(12, &[labels::TEST_DEBT]),
            closed,
            issue(9, &[labels::TEST_DEBT]),
        ];
        let waiting = Snapshot {
            tech_refining: vec![issue(6, &[labels::TECH_REFINEMENT])],
            arbitrating: Vec::new(),
            test_debt: debt.clone(),
            ..empty()
        };
        assert_eq!(decide(&waiting), Route::TechRefinement { issue: 6 });
        let alone = Snapshot {
            test_debt: debt,
            ..empty()
        };
        assert_eq!(decide(&alone), Route::TestDebt { issue: 9 });
        assert_eq!(decide(&empty()), Route::Nothing);
    }

    #[test]
    fn a_roadmap_item_that_already_has_a_milestone_is_not_replanned() {
        let snapshot = Snapshot {
            roadmap: vec![issue(4, &[labels::ROADMAP, labels::READY])],
            lowest_roadmap_has_milestone: true,
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Nothing);
    }

    #[test]
    fn a_decision_comes_after_the_rules_and_before_any_work_it_unblocks() {
        let mut decided = issue(2, &[labels::NEEDS_DECISION]);
        decided.state = "closed".to_string();
        let snapshot = Snapshot {
            deciding: vec![decided, issue(8, &[labels::NEEDS_DECISION, labels::AGENT])],
            milestones: vec![issue(9, &[labels::MILESTONE, labels::READY])],
            pr_to_fix: Some(pr("32", "milestone/3-x")),
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Decide { issue: 8 });
        let rules_first = Snapshot {
            rules: vec![issue(5, &[labels::RULES, labels::READY])],
            ..snapshot
        };
        assert_eq!(decide(&rules_first), Route::Rules { issue: 5 });
    }

    #[test]
    fn a_delivered_roadmap_item_waiting_for_its_release_does_not_hold_the_next() {
        let delivered = issue(4, &[labels::ROADMAP, labels::READY, labels::WAITING_MERGE]);
        let next = issue(9, &[labels::ROADMAP, labels::READY]);
        let snapshot = Snapshot {
            roadmap: vec![delivered.clone(), next],
            lowest_roadmap_has_milestone: false,
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Planner { roadmap: 9 });
        assert_eq!(plannable_roadmap(&[delivered]), None);
    }

    #[test]
    fn a_roadmap_item_without_ready_is_not_planned() {
        let snapshot = Snapshot {
            roadmap: vec![issue(4, &[labels::ROADMAP])],
            lowest_roadmap_has_milestone: false,
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Nothing);
    }

    #[test]
    fn a_ready_milestone_outranks_merging_refinement_and_the_dev_loop() {
        let snapshot = Snapshot {
            milestones: vec![issue(9, &[labels::MILESTONE, labels::READY])],
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Split { milestone: 9 });
    }

    #[test]
    fn a_milestone_is_split_only_once_the_one_before_is_delivered() {
        let mut next = issue(9, &[labels::MILESTONE, labels::READY]);
        next.blocked_by = vec![issue(8, &[labels::MILESTONE])];
        let snapshot = Snapshot {
            milestones: vec![next.clone()],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Nothing);

        next.blocked_by = vec![issue(8, &[labels::MILESTONE, labels::WAITING_MERGE])];
        let snapshot = Snapshot {
            milestones: vec![next],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Split { milestone: 9 });
    }

    #[test]
    fn a_ready_milestone_is_refined_before_it_is_split() {
        let pending = issue(9, &[labels::MILESTONE, labels::READY, labels::REFINEMENT]);
        let snapshot = Snapshot {
            milestones: vec![pending.clone()],
            refining: vec![pending],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Refinement { issue: 9 });
    }

    #[test]
    fn a_milestone_ready_to_merge_outranks_refinement_and_the_dev_loop() {
        let snapshot = Snapshot {
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::MergeMainAgent { milestone: 7 });
    }

    #[test]
    fn a_red_pr_outranks_reviewing_merging_refinement_and_the_dev_loop() {
        // What is already in flight and broken comes before anything new:
        // the milestone it belongs to cannot close over a red PR anyway.
        let snapshot = Snapshot {
            pr_to_fix: Some(pr("32", "milestone/4-territory")),
            pr_to_review: Some(pr("33", "milestone/4-territory")),
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(
            decide(&snapshot),
            Route::PrFix {
                pr: "32".to_string()
            }
        );
    }

    #[test]
    fn a_pr_to_review_outranks_merging_refinement_and_the_dev_loop() {
        let snapshot = Snapshot {
            pr_to_review: Some(pr("33", "milestone/4-territory")),
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(
            decide(&snapshot),
            Route::PrReview {
                pr: "33".to_string(),
                base: "milestone/4-territory".to_string(),
            }
        );
    }

    #[test]
    fn the_review_carries_the_branch_the_pr_actually_targets() {
        // A task PR targets its milestone's branch, never the integration
        // branch — a review judging against a fixed base would skip it as
        // "targets the wrong branch".
        let snapshot = Snapshot {
            pr_to_review: Some(pr("33", "milestone/9-cities")),
            ..empty()
        };
        let Route::PrReview { base, .. } = decide(&snapshot) else {
            panic!("a review");
        };
        assert_eq!(base, "milestone/9-cities");
    }

    #[test]
    fn a_roadmap_item_to_plan_still_outranks_a_broken_pr() {
        // Decomposition stays at the top: without a plan there is no task
        // whose PR could be broken in the first place.
        let snapshot = Snapshot {
            roadmap: vec![issue(4, &[labels::ROADMAP, labels::READY])],
            pr_to_fix: Some(pr("32", "main_agent")),
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Planner { roadmap: 4 });
    }

    #[test]
    fn a_milestone_without_ready_is_not_split() {
        let snapshot = Snapshot {
            milestones: vec![issue(9, &[labels::MILESTONE])],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Nothing);
    }

    #[test]
    fn a_closed_ready_milestone_is_not_split_again() {
        let mut closed = issue(9, &[labels::MILESTONE, labels::READY]);
        closed.state = "closed".to_string();
        let snapshot = Snapshot {
            milestones: vec![closed],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Nothing);
    }

    #[test]
    fn a_graph_in_question_outranks_the_dev_loop_and_refinement_but_not_a_merge() {
        let mut snapshot = empty();
        snapshot.arbitrating = vec![issue(9, &[labels::AGENT, labels::NEEDS_AI_DECISION])];
        snapshot.dev_loop_milestones = vec![3];
        snapshot.refining = vec![issue(6, &[labels::REFINEMENT])];
        assert_eq!(decide(&snapshot), Route::Arbiter { issue: 9 });
        snapshot.ready_to_merge = Some(7);
        assert_eq!(decide(&snapshot), Route::MergeMainAgent { milestone: 7 });
    }

    #[test]
    fn the_dev_loop_outranks_pending_refinement() {
        let snapshot = Snapshot {
            refining: vec![issue(6, &[labels::REFINEMENT])],
            tech_refining: vec![issue(7, &[labels::TECH_REFINEMENT])],
            arbitrating: Vec::new(),
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::DevLoop { milestone: 3 });
    }

    #[test]
    fn the_lowest_numbered_refinement_issue_goes_first() {
        let snapshot = Snapshot {
            refining: vec![
                issue(9, &[labels::REFINEMENT]),
                issue(6, &[labels::REFINEMENT]),
            ],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Refinement { issue: 6 });
    }

    #[test]
    fn business_refinement_goes_before_technical() {
        let snapshot = Snapshot {
            refining: vec![issue(9, &[labels::REFINEMENT])],
            tech_refining: vec![issue(6, &[labels::TECH_REFINEMENT])],
            arbitrating: Vec::new(),
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Refinement { issue: 9 });
        let snapshot = Snapshot {
            refining: Vec::new(),
            ..snapshot
        };
        assert_eq!(decide(&snapshot), Route::TechRefinement { issue: 6 });
    }

    #[test]
    fn a_runnable_task_runs_the_dev_loop_when_nothing_outranks_it() {
        let snapshot = Snapshot {
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::DevLoop { milestone: 3 });
    }

    #[test]
    fn an_entirely_quiet_repo_routes_to_nothing() {
        assert_eq!(decide(&empty()), Route::Nothing);
    }

    fn quiet() -> Pulse {
        Pulse {
            etag: Some("W/\"abc\"".to_string()),
            watching_ci: false,
            quiet_ticks: 0,
            rest: Rest::default(),
            taps: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn a_quota_stop_rests_longer_each_time_up_to_the_cap() {
        let mut rest = Rest::default();
        assert!(!rest.resting());
        assert!(rest.passes(), "no rest: the tick goes on");
        assert_eq!(rest.struck(), FIRST_REST);
        assert_eq!(rest.struck(), FIRST_REST * 2);
        assert_eq!(rest.struck(), FIRST_REST * 4);
        for _ in 0..20 {
            rest.struck();
        }
        assert_eq!(rest.ticks_left, LONGEST_REST, "capped, never overflowing");
    }

    #[test]
    fn a_rest_passes_tick_by_tick_and_a_dispatch_clears_it() {
        let mut rest = Rest::default();
        rest.struck();
        for _ in 0..FIRST_REST {
            assert!(rest.resting());
            assert!(!rest.passes());
        }
        assert!(!rest.resting());
        assert!(rest.passes(), "the rest is over");
        rest.struck();
        rest.cleared();
        assert_eq!(rest, Rest::default());
        assert_eq!(rest.struck(), FIRST_REST, "cleared: back to the first rest");
    }

    #[test]
    fn an_unchanged_board_skips_the_full_read() {
        assert!(may_skip(&quiet(), &Activity::Unchanged, 0));
    }

    #[test]
    fn a_changed_board_is_read_in_full() {
        let changed = Activity::Changed {
            etag: None,
            moved_at: None,
        };
        assert!(!may_skip(&quiet(), &changed, 0));
    }

    #[test]
    fn a_waited_on_pull_request_keeps_the_full_read_going() {
        let pulse = Pulse {
            watching_ci: true,
            ..quiet()
        };
        assert!(!may_skip(&pulse, &Activity::Unchanged, 0));
    }

    #[test]
    fn a_freed_lane_reads_the_board_again() {
        assert!(!may_skip(&quiet(), &Activity::Unchanged, 1));
    }

    #[test]
    fn the_valve_forces_a_full_read_after_enough_quiet_ticks() {
        let mut pulse = quiet();
        for _ in 0..FULL_READ_EVERY {
            assert!(may_skip(&pulse, &Activity::Unchanged, 0));
            pulse.stayed_quiet();
        }
        assert!(!may_skip(&pulse, &Activity::Unchanged, 0));
        pulse.read_in_full(Some("W/\"def\"".to_string()), false);
        assert_eq!(pulse.quiet_ticks, 0);
        assert!(may_skip(&pulse, &Activity::Unchanged, 0));
    }

    #[test]
    fn lanes_take_the_tasks_and_the_preparation_together() {
        let snapshot = Snapshot {
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(
            lane_work(&snapshot),
            LaneWork {
                tasks_of: vec![3],
                preparation: true,
            }
        );
    }

    #[test]
    fn a_ready_milestone_is_lane_preparation_but_a_closed_refinement_is_not() {
        let closed = Issue {
            state: "closed".to_string(),
            ..issue(6, &[labels::REFINEMENT])
        };
        let snapshot = Snapshot {
            refining: vec![closed],
            ..empty()
        };
        assert!(!lane_work(&snapshot).preparation);
        let snapshot = Snapshot {
            milestones: vec![issue(4, &[labels::MILESTONE, labels::READY])],
            ..empty()
        };
        assert!(lane_work(&snapshot).preparation);
    }

    #[test]
    fn a_quiet_board_gives_the_lanes_nothing() {
        assert_eq!(
            lane_work(&empty()),
            LaneWork {
                tasks_of: Vec::new(),
                preparation: false,
            }
        );
    }

    const GRAPH: &str = r#"{"version":1,"milestones":[
      {"number":18,"title":"Campaigns","systems":["campaign"],"produces":["campaign.listCampaigns"],"consumes":[],"depends_on":[]},
      {"number":20,"title":"Sessions","systems":["session"],"produces":["session.listScenes","session.addScene"],"consumes":["campaign.listCampaigns"],"depends_on":[18]},
      {"number":19,"title":"Media","systems":["media"],"produces":["media.listMedia"],"consumes":["session.listScenes"],"depends_on":[20]},
      {"number":21,"title":"Sound","systems":["sound"],"produces":["sound.play"],"consumes":[],"depends_on":[]},
      {"number":22,"title":"Battle","systems":["battle"],"produces":["battle.start"],"consumes":["session.listScenes"],"depends_on":[20]},
      {"number":30,"title":"Home","systems":["home"],"produces":["home.show"],"consumes":[],"depends_on":[]},
      {"number":31,"title":"Archive","systems":["archive"],"produces":["archive.export"],"consumes":[],"depends_on":[]}
    ],"ids":{}}"#;

    fn graph() -> Graph {
        parse_graph(Some(GRAPH)).expect("a graph")
    }

    #[test]
    fn with_a_graph_a_milestone_splits_once_its_producers_are_split_whatever_its_number() {
        let graph = graph();
        let ready = |n: u64| issue(n, &[labels::MILESTONE, labels::READY]);
        let building = |n: u64| issue(n, &[labels::MILESTONE, labels::TRIGGERED]);
        // #18 builds: #20 reads it — its contract is mocked, #20 may split;
        // #21 reads nothing and splits; #19 and #22 wait on #20's split.
        let milestones = vec![building(18), ready(19), ready(20), ready(21), ready(22)];
        assert!(splittable(&milestones[2], &milestones, Some(&graph)), "#20");
        assert!(splittable(&milestones[3], &milestones, Some(&graph)), "#21");
        assert!(
            !splittable(&milestones[1], &milestones, Some(&graph)),
            "#19 waits on #20"
        );
        assert!(
            !splittable(&milestones[4], &milestones, Some(&graph)),
            "#22 waits on #20"
        );
        // Without the graph, the old rule: #18 open holds every later one.
        assert!(!splittable(&milestones[3], &milestones, None));
        // A producer split suffices — its contract is mocked — one not yet
        // split does not; one absent from the open list is closed: delivered.
        let waiting = vec![ready(19), ready(20)];
        assert!(
            !splittable(&waiting[0], &waiting, Some(&graph)),
            "#20 not split"
        );
        let later = vec![ready(19), building(20)];
        assert!(splittable(&later[0], &later, Some(&graph)), "#20 building");
        let landed = vec![
            ready(19),
            issue(
                20,
                &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
            ),
        ];
        assert!(splittable(&landed[0], &landed, Some(&graph)));
        assert!(
            splittable(&ready(22), &[ready(22)], Some(&graph)),
            "#20 gone: closed"
        );
        // A milestone the graph does not know keeps the old rule.
        let unknown = vec![building(18), ready(40)];
        assert!(!splittable(&unknown[1], &unknown, Some(&graph)));
    }

    #[test]
    fn the_critical_path_goes_first_and_the_ahead_work_fills_up_to_the_cap() {
        let graph = graph();
        let ready = |n: u64| issue(n, &[labels::MILESTONE, labels::READY]);
        let building = |n: u64| issue(n, &[labels::MILESTONE, labels::TRIGGERED]);
        // The chain 18 -> 20 -> 19/22 weighs the most: the path, #19 first
        // in plan order on the tie.
        let milestones = vec![ready(18), ready(19), ready(20), ready(21), ready(22)];
        assert_eq!(critical_path(&milestones, Some(&graph)), [18, 20, 19]);
        assert!(critical_path(&milestones, None).is_empty());
        // Nothing builds: #18 (the path) and #21 split; #20 waits on #18.
        assert_eq!(splits(&milestones, Some(&graph)), [18, 21]);
        // Three build already: the path still goes, the ahead work waits.
        let crowded = vec![
            building(18),
            building(21),
            building(30),
            ready(20),
            ready(31),
        ];
        assert_eq!(
            splits(&crowded, Some(&graph)),
            [20],
            "#31 is ahead work past the cap"
        );
        // Two build: one ahead slot left, the path first.
        let two = vec![building(18), building(21), ready(20), ready(30), ready(31)];
        assert_eq!(splits(&two, Some(&graph)), [20, 30]);
        let mut numbers = vec![22, 19, 21, 20, 18];
        by_critical_path(&mut numbers, &[18, 20, 19]);
        assert_eq!(numbers, [18, 20, 19, 21, 22]);
    }

    #[test]
    fn a_delivered_milestone_weighs_nothing_on_the_path() {
        let graph = graph();
        let landed = vec![
            issue(
                18,
                &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
            ),
            issue(20, &[labels::MILESTONE, labels::TRIGGERED]),
            issue(19, &[labels::MILESTONE, labels::READY]),
            issue(22, &[labels::MILESTONE, labels::READY]),
        ];
        let path = critical_path(&landed, Some(&graph));
        assert!(path.ends_with(&[20, 19]), "{path:?}");
    }

    #[test]
    fn a_graph_that_does_not_read_is_no_graph() {
        assert!(parse_graph(None).is_none());
        assert!(parse_graph(Some("{")).is_none());
        assert_eq!(
            parse_graph(Some(GRAPH)).map(|g| g.milestones.len()),
            Some(7)
        );
        assert_eq!(GRAPH_PATH, "plan/graph.json");
    }

    #[test]
    fn a_contract_change_outranks_a_split_and_the_lanes_take_every_building_milestone() {
        let snapshot = Snapshot {
            contract_changes: vec![issue(70, &[labels::CONTRACT_CHANGE])],
            milestones: vec![issue(5, &[labels::MILESTONE, labels::READY])],
            dev_loop_milestones: vec![20, 18],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::ContractChange { issue: 70 });
        assert_eq!(lane_work(&snapshot).tasks_of, [20, 18]);
        let closed = Snapshot {
            contract_changes: vec![Issue {
                state: "closed".to_string(),
                ..issue(70, &[labels::CONTRACT_CHANGE])
            }],
            ..snapshot
        };
        assert_eq!(closed.dev_loop_milestones, [20, 18]);
        assert_eq!(decide(&closed), Route::Split { milestone: 5 });
        let building = Snapshot {
            milestones: Vec::new(),
            ..closed
        };
        assert_eq!(decide(&building), Route::DevLoop { milestone: 20 });
    }
}
