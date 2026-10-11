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
//! against it; a milestone ready to split comes next, once no refinement is
//! pending on it; then the two things
//! that can be wrong with a pull request already in flight — a red one to
//! repair before a green one to review, since reviewing a change whose CI
//! is broken reviews something about to change; then merging a reviewed,
//! green task PR into its milestone; then finishing a milestone that's
//! done; then a runnable task; then a pending refinement (business, then
//! technical); and last a file's test debt. Structural decomposition before
//! unblocking what's in flight before closing out before delivering before
//! preparing the next delivery — and paying back the tests only when nothing
//! else waits, since the debt is real but never blocks anyone.
//!
//! A parallel watch does not stop at the route: it first fills every free
//! lane, the runnable tasks before splits and refinements ([`lane_work`]),
//! so a pending refinement never keeps a task off a lane that sits free.
//!
//! `harness:ready` is the same human gate at every issue level: it means
//! "plan this" on a roadmap item, "split this" on a milestone, and "build
//! this" on a task. `harness:to-review` and `harness:pr-fix` are the same
//! kind of gate on a **pull request**, and `harness:test-debt` on the issue
//! the CI's mutants rotation opens. The router never acts on something
//! nobody has opened the tap on.

use harness_core::domain::{Issue, Pr};
use harness_core::ports::shell::github::Activity;

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
    /// Run the dev loop on this milestone's board.
    DevLoop {
        /// The milestone issue's number — so the dispatcher can derive the
        /// branch the dev loop works on (`common::branching`), rather than
        /// a fixed integration branch.
        milestone: u64,
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
    /// The current milestone, if its board has a runnable task right now —
    /// `None` either because there's no open milestone, or because the one
    /// there is has nothing runnable.
    pub dev_loop_milestone: Option<u64>,
    /// Open `harness:test-debt` issues, any order.
    pub test_debt: Vec<Issue>,
}

/// Decides the next route from a snapshot.
#[must_use]
pub fn decide(snapshot: &Snapshot) -> Route {
    if let Some(lowest) = lowest_open(&snapshot.roadmap)
        && lowest.has(labels::READY)
        && !snapshot.lowest_roadmap_has_milestone
    {
        return Route::Planner {
            roadmap: lowest.number,
        };
    }
    if let Some(ready) = snapshot
        .milestones
        .iter()
        .filter(|issue| splittable(issue))
        .min_by_key(|issue| issue.number)
    {
        return Route::Split {
            milestone: ready.number,
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
    // Delivering before preparing: a runnable task is work already decided,
    // and a pending refinement only ever prepares the next one.
    if let Some(milestone) = snapshot.dev_loop_milestone {
        return Route::DevLoop { milestone };
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneWork {
    /// The milestone whose runnable tasks take the free lanes first.
    pub tasks_of: Option<u64>,
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
        tasks_of: snapshot.dev_loop_milestone,
        preparation: snapshot.refining.iter().any(Issue::is_open)
            || snapshot.milestones.iter().any(splittable),
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

/// Whether this milestone is ready to be split now: a candidate, and every
/// milestone it is `blocked_by` delivered — closed, or `waiting-merge` with
/// its code on the integration branch.
///
/// Its tasks are cut against the code the previous milestone leaves: split
/// before that code exists, they plan for a guess. `milestone.blocked_by`
/// must have been read ([`GitHub::with_blockers`]); empty reads as nothing
/// blocking.
///
/// [`GitHub::with_blockers`]: harness_core::ports::shell::github::GitHub::with_blockers
#[must_use]
pub fn splittable(milestone: &Issue) -> bool {
    split_candidate(milestone)
        && milestone
            .blocked_by
            .iter()
            .all(|blocker| blocker.is_closed() || blocker.has(labels::WAITING_MERGE))
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
            dev_loop_milestone: None,
            test_debt: Vec::new(),
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
            milestones: vec![issue(5, &[labels::MILESTONE, labels::READY])],
            pr_to_fix: Some(pr("32", "main_agent")),
            pr_to_review: Some(pr("33", "main_agent")),
            task_to_merge: Some((40, pr("41", "milestone/3-x"))),
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            tech_refining: vec![],
            dev_loop_milestone: Some(3),
            test_debt: vec![issue(8, &[labels::TEST_DEBT])],
        };
        assert_eq!(decide(&snapshot), Route::Planner { roadmap: 4 });
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
            dev_loop_milestone: Some(3),
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
            dev_loop_milestone: Some(3),
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
            dev_loop_milestone: Some(3),
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
            dev_loop_milestone: Some(3),
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
    fn the_dev_loop_outranks_pending_refinement() {
        let snapshot = Snapshot {
            refining: vec![issue(6, &[labels::REFINEMENT])],
            tech_refining: vec![issue(7, &[labels::TECH_REFINEMENT])],
            dev_loop_milestone: Some(3),
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
            dev_loop_milestone: Some(3),
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
            dev_loop_milestone: Some(3),
            ..empty()
        };
        assert_eq!(
            lane_work(&snapshot),
            LaneWork {
                tasks_of: Some(3),
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
                tasks_of: None,
                preparation: false,
            }
        );
    }
}
