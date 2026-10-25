//! Which work runs next, decided from a snapshot already read.
//!
//! Pure: no `gh` call lives here. The router (the launcher's polling loop)
//! does the reads this needs — including the facts that require more than
//! a flat issue list (`lowest_roadmap_has_milestone`, `ready_to_merge`, the
//! building milestones' boards) — and hands this module the result. That is
//! what makes the decision testable without a fake adapter.
//!
//! **One scheduler for every route** ([`schedule`]): every piece of work the
//! snapshot shows becomes a [`Job`], weighted, and the watch starts them in
//! weight order — the free ones (the catch-ups, a settled contest's
//! follow-through and the two merges) in the
//! tick, each paid one
//! on a free lane, as many as there are lanes. Nothing paid runs in the
//! tick under `--parallel`, so a long session never keeps the lanes from
//! being refilled; nothing waits a tick because another job of its kind
//! was picked first.
//!
//! The weight is the route's base ([`base_weight`]) — free work first: a
//! milestone branch's catch-up with the integration branch's green head
//! (`common::catch_up`), then a task's merge into its milestone before the
//! milestone's into the integration branch, since they unblock work at no
//! cost; then planning, the rules' tests and
//! the decisions, which unblock others; then repairing before reviewing a
//! PR in flight; then cutting and arbitrating; then building; then
//! preparing (business, then technical refinement); and last a file's test
//! debt — plus a bonus for a milestone on the critical path
//! ([`critical_path`]), so its PRs, decisions and tasks pass before the
//! ahead work, and a bonus for every tick a job has waited
//! ([`Pulse::waited`]), capped, so the low ones are never starved. **While a
//! rules' task is open, ready or not, no development task starts**
//! ([`rules_first`]) — a contest aside: it holds only the PR that contests,
//! which waits ([`Snapshot::prs_waiting`], `common::contest`) with no
//! repair, decision or merge until the contest settles; the settled contest
//! is then followed through for free ([`Route::Resume`]). A rules' task
//! whose PR is faithful and still checking is neither tapped nor attempted
//! again ([`Snapshot::rules_on_pr`]). [`decide`] is the schedule's first
//! job, for a watch that runs one route at a time.
//!
//! **While the integration branch is red** ([`base_red`]), the jobs that
//! would build on it or merge into it — a task's start, since its milestone
//! branch takes the integration branch first; a merge into a milestone; a
//! merge into the base — are not scheduled; the reviews, the repairs, the
//! rules, the test debt and the decisions go on. The red is read by the
//! sentinel (`common::sentinel`), which also opens the one issue that says
//! it.
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

use crate::common::catch_up::CatchUps;
use crate::common::close_merged::Delivered;
use crate::common::contest::{ContractLanded, PrWait, Settled};
use crate::common::labels;
use crate::test_debt::data::landing::Landing;

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
    /// `milestone_merge`. The rules' task's PR, decided merged by the
    /// harness, goes the same way into the integration branch.
    MergeIntoMilestone {
        /// The task's number.
        task: u64,
        /// The PR, as the ports name one.
        pr: String,
        /// The branch it targets — the milestone's, or the integration
        /// branch for the rules' task.
        base: String,
    },
    /// Bring this milestone's branch up to the integration branch's head —
    /// `common::catch_up`: GitHub's merge, else the harness's own with the
    /// generated files regenerated. Free, run in the tick.
    CatchUp {
        /// The milestone issue's number.
        milestone: u64,
        /// Its branch.
        branch: String,
    },
    /// Follow a settled rules contest through — `common::contest::resume`:
    /// `CONTEST.md` deleted on the branch it was written for, and the PR
    /// that waited on it marked resumed, its repairs counted anew. Free,
    /// run in the tick.
    Resume {
        /// The contest issue, closed.
        contest: u64,
        /// The PR that waited on it, if one did.
        pr: Option<String>,
        /// The branch to delete `CONTEST.md` on, if any.
        lift: Option<String>,
    },
    /// Close a task whose PR was merged into its milestone outside the
    /// merge train — `common::close_merged`. Free, run in the tick.
    CloseMerged {
        /// The task's number.
        task: u64,
        /// The merged PR's number.
        pr: u64,
        /// The milestone branch it was merged into.
        branch: String,
    },
    /// Resume a PR whose contract change landed — `common::contest::
    /// resume_contract`: the resumed marker, what the change said, one
    /// repair asked. Free, run in the tick.
    ContractLanded {
        /// The `harness:contract-change` issue, closed.
        request: u64,
        /// The PR that waited on it.
        pr: String,
    },
    /// Merge a green test-debt PR into the integration branch and close its
    /// issue — `milestone_merge::run`, at most one per
    /// [`DEBT_MERGE_EVERY`]. Free, run in the tick.
    MergeDebt {
        /// The test-debt issue.
        issue: u64,
        /// The PR.
        pr: String,
    },
    /// Close a test-debt PR that cannot land — red, conflicting, or touching
    /// what a test-only PR may not — with the reason; its issue stays open
    /// for the next rotation. Free, run in the tick.
    DropDebtPr {
        /// The test-debt issue.
        issue: u64,
        /// The PR.
        pr: String,
        /// Why, in a sentence.
        why: String,
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
    /// Build this task of this milestone on a lane — the dev loop's
    /// `--task` child. What a parallel watch starts; a sequential one runs
    /// [`Route::DevLoop`], which picks its task itself.
    DevTask {
        /// The milestone issue's number.
        milestone: u64,
        /// The task's number.
        task: u64,
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
    /// The open PRs to repair, in number order: `harness:pr-fix` with a
    /// check that actually broke, or a blocking review or a conflict — the
    /// router pairs the label with a failing-check read, which a label list
    /// cannot answer alone.
    pub prs_to_fix: Vec<Pr>,
    /// The open PRs carrying `harness:to-review` that the review would not
    /// skip, in number order — the router applies the review's own skip
    /// rules (`pr_review::data::skip_rules`) so a PR that would skip never
    /// costs a mounted checkout per poll.
    pub prs_to_review: Vec<Pr>,
    /// The tasks whose PR is open on their milestone, reviewed and green,
    /// with that PR — what `milestone_merge` merges, one per board — and the
    /// rules' task whose PR the harness decided to merge as it is, once
    /// green (`milestone_merge::rules_prs`, `RulesPr::Mergeable`).
    pub tasks_to_merge: Vec<(u64, Pr)>,
    /// Every open PR carrying `harness:to-review` or `harness:pr-fix`, as read.
    ///
    /// On a lane or not, worth reviewing or not, unlike the two lists above.
    /// A task one of them names ([`pr_cycle_owns`]) belongs to that PR's own
    /// cycle — review, repair, merge — until the PR is merged or closed, and
    /// no second session is sent onto its branch.
    pub task_prs: Vec<Pr>,
    /// The open milestones whose tasks are all delivered, not
    /// `harness:waiting-merge` yet, whose producers have landed — computed
    /// by the router against each milestone's own sub-issues, which a flat
    /// list cannot answer alone.
    pub ready_to_merge: Vec<u64>,
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
    /// The boards read this tick, in the same order — what the lanes take
    /// their tasks from, read once: a second read per lane was what spent
    /// the GitHub rate limit on eight building milestones.
    pub boards: Vec<crate::dev_loop::data::board::Board>,
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
    /// The integration branch's own checks, as the sentinel
    /// (`common::sentinel::watch`) answers them from
    /// [`GitHub::branch_checks`](harness_core::ports::shell::github::GitHub::branch_checks):
    /// `Some(false)` — red, or not green again yet after a red — holds
    /// everything that would build on the base or merge into it
    /// ([`base_red`]); `None` and `Some(true)` hold nothing.
    pub base_checks: Option<bool>,
    /// The product's database does not answer (its `DATABASE_URL`): a
    /// code session or a repair started now would run its tests without it
    /// — those that need it skip in silence, then fail in CI
    /// (`ForgeDesMondes`, 2026-10-19/20: Docker down all night, 7 of 20 red
    /// `rust` checks). Both wait; the rest goes on.
    pub database_down: bool,
    /// As many task PRs are open as there are lanes: no new task starts
    /// until reviews, repairs and merges bring the count down — finishing
    /// before starting (`ForgeDesMondes`, 2026-10-20: the open PRs grew to
    /// ten while every lane took new work).
    pub review_backlog_full: bool,
    /// The milestone branches behind the integration branch's green head,
    /// with their branch — the catch-ups due ([`CatchUps::due`]). Their
    /// tasks wait for the catch-up, and so does a repair of their landing
    /// PR.
    pub catch_up: Vec<(u64, String)>,
    /// The milestones whose tasks wait on their branch
    /// ([`CatchUps::holding`]): it could not take the base — a conflict no
    /// script solves, a failure parked — or it lags the integration
    /// branch's head, its catch-up not made yet. Not a stuck branch: one
    /// that lags is held from every head's move to its catch-up, a few
    /// ticks; what reads this as "stuck" must tell the two apart.
    pub catch_up_held: Vec<u64>,
    /// The PRs of [`Snapshot::task_prs`] that wait — on their open contest,
    /// or on an issue a decision named (`common::contest::waiting`): no
    /// repair, no decision on their task, no merge. The review still runs.
    pub prs_waiting: Vec<(Pr, PrWait)>,
    /// The open rules' tasks whose PR is judged faithful and mergeable or
    /// still checking (`milestone_merge::RulesPr`): neither tapped nor
    /// attempted again while it is.
    pub rules_on_pr: Vec<u64>,
    /// The settled contests to follow through ([`Route::Resume`]).
    pub settled: Vec<Settled>,
    /// The tasks of the building milestones whose PR was merged outside the
    /// merge train, still open ([`Route::CloseMerged`]).
    pub tasks_merged: Vec<Delivered>,
    /// The PRs whose contract change landed — closed, their base holding it
    /// ([`Route::ContractLanded`]).
    pub contracts_landed: Vec<ContractLanded>,
    /// The test-debt PRs that land now ([`Route::MergeDebt`],
    /// [`Route::DropDebtPr`]) — the merges dropped while one was merged less
    /// than [`DEBT_MERGE_EVERY`] ago ([`hold_debt_merges`]).
    pub debt_landings: Vec<Landing>,
}

/// Whether the integration branch is red: its newest head has a failed
/// workflow run.
///
/// A milestone branch takes the integration branch before a task starts, so
/// a task started now would build on the red; a merge into a milestone or
/// into the base would spread it or bury it. Both wait; the reviews, the
/// repairs, the rules, the test debt and the decisions do not touch the base
/// and go on.
#[must_use]
pub const fn base_red(snapshot: &Snapshot) -> bool {
    matches!(snapshot.base_checks, Some(false))
}

/// The rules' task development waits on: the lowest open `harness:rules`
/// issue, ready or not — a failed attempt hands it to a human, and the
/// tests still come first.
///
/// A contest (`Rules contested: …`) holds no development: the PR that
/// contests waits on it ([`Snapshot::prs_waiting`]) and nothing else does —
/// one test of one rule is in flux, not the tests.
#[must_use]
pub fn rules_first(snapshot: &Snapshot) -> Option<u64> {
    let holding: Vec<Issue> = snapshot
        .rules
        .iter()
        .filter(|issue| {
            !issue
                .title
                .trim_start()
                .starts_with(crate::rules::data::request::CONTESTED)
        })
        .cloned()
        .collect();
    lowest_open(&holding).map(|issue| issue.number)
}

/// The first job of the [`schedule`], for a watch that runs one route at a
/// time: a task to build is its milestone's dev loop, which picks it.
#[must_use]
pub fn decide(snapshot: &Snapshot) -> Route {
    schedule(snapshot, &[], &|_| 0)
        .into_iter()
        .next()
        .map_or(Route::Nothing, |job| match job.route {
            Route::DevTask { milestone, .. } => Route::DevLoop { milestone },
            route => route,
        })
}

/// What a job costs: a paid one opens a session and waits out a quota
/// rest; a free one is a few GitHub calls, run in the tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cost {
    /// No session: the catch-ups, a settled contest's or a landed
    /// contract's follow-through, a merged task's close, the merges, a
    /// test-debt PR's landing.
    Free,
    /// One session or more.
    Paid,
}

/// One piece of work the snapshot shows, weighted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// What to run.
    pub route: Route,
    /// Its weight: the higher starts first ([`base_weight`], the critical
    /// path's bonus, the ticks it waited).
    pub weight: u32,
    /// Free or paid.
    pub cost: Cost,
    /// The issue or PR it works on: one job per key at a time.
    pub key: u64,
    /// The jobs that may never run side by side share a group: the
    /// technical refinement reads code a task may be changing, the test
    /// debt has one writable checkout.
    pub exclusive: Option<&'static str>,
}

/// A route's own weight, before the critical path's and the waiting's.
#[must_use]
pub const fn base_weight(route: &Route) -> u32 {
    match route {
        Route::CatchUp { .. } => 1010,
        Route::Resume { .. } => 1005,
        Route::CloseMerged { .. } | Route::ContractLanded { .. } => 1003,
        Route::MergeIntoMilestone { .. } => 1000,
        Route::DropDebtPr { .. } => 995,
        Route::MergeMainAgent { .. } => 990,
        Route::MergeDebt { .. } => 985,
        Route::Planner { .. } => 900,
        Route::Rules { .. } => 880,
        Route::Decide { .. } => 860,
        Route::ContractChange { .. } => 850,
        Route::PrFix { .. } => 800,
        Route::PrReview { .. } => 780,
        Route::Split { .. } => 700,
        Route::Arbiter { .. } => 680,
        Route::DevLoop { .. } | Route::DevTask { .. } => 600,
        Route::Refinement { .. } => 500,
        Route::TechRefinement { .. } => 450,
        Route::TestDebt { .. } => 200,
        Route::Nothing => 0,
    }
}

/// The bonus of the critical path's first milestone; each next one gets ten
/// less, and a milestone off the path none.
pub const CRITICAL_BONUS: u32 = 100;

/// The most a job's waiting adds to its weight: one a tick, up to here —
/// a refinement waited long enough passes the tasks, a test debt never
/// passes a decision.
pub const AGE_CAP: u32 = 150;

/// Every job the snapshot shows, heaviest first, given the issues lanes
/// are on (`running`) and how many ticks each route has waited.
///
/// Ties go to the heavier base, then to the order listed. The launcher
/// keeps one job per key and per exclusive group, and skips a paid one
/// during a quota rest ([`plan`]).
#[must_use]
pub fn schedule(snapshot: &Snapshot, running: &[u64], waited: &dyn Fn(&Route) -> u32) -> Vec<Job> {
    let path = critical_path(&snapshot.milestones, snapshot.graph.as_ref());
    let bonus = |milestone: Option<u64>| {
        milestone
            .and_then(|m| path.iter().position(|on| *on == m))
            .map_or(0, |at| {
                CRITICAL_BONUS
                    .saturating_sub(u32::try_from(at).unwrap_or(u32::MAX).saturating_mul(10))
            })
    };
    let mut found = in_flight(snapshot);
    found.extend(to_build(snapshot, running));
    let mut jobs: Vec<Job> = found
        .into_iter()
        .map(|(route, key, milestone)| Job {
            cost: if matches!(
                route,
                Route::CatchUp { .. }
                    | Route::Resume { .. }
                    | Route::CloseMerged { .. }
                    | Route::ContractLanded { .. }
                    | Route::MergeDebt { .. }
                    | Route::DropDebtPr { .. }
                    | Route::MergeMainAgent { .. }
                    | Route::MergeIntoMilestone { .. }
            ) {
                Cost::Free
            } else {
                Cost::Paid
            },
            exclusive: match route {
                Route::TechRefinement { .. } => Some("tech-refinement"),
                Route::TestDebt { .. } => Some("test-debt"),
                Route::Planner { .. } => Some("planner"),
                _ => None,
            },
            weight: base_weight(&route)
                .saturating_add(bonus(milestone))
                .saturating_add(waited(&route).min(AGE_CAP)),
            route,
            key,
        })
        .collect();
    // Stable: within a weight, the order the jobs were listed in — number
    // order within a kind, the building order (critical path first) among
    // the boards.
    jobs.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then(base_weight(&b.route).cmp(&base_weight(&a.route)))
    });
    jobs
}

/// A route found, the issue or PR it works on, and its milestone if known.
type Found = (Route, u64, Option<u64>);

/// The milestone an issue belongs to, when a board read this tick holds it.
fn milestone_of_issue(snapshot: &Snapshot, number: u64) -> Option<u64> {
    snapshot
        .boards
        .iter()
        .find(|b| b.tasks.iter().any(|t| t.number == number))
        .map(|b| b.milestone.number)
}

/// The routes of an issue list, in number order, each with its milestone.
fn of_issues(snapshot: &Snapshot, issues: &[Issue], route: fn(u64) -> Route) -> Vec<Found> {
    open(issues)
        .map(|issue| {
            (
                route(issue.number),
                issue.number,
                milestone_of_issue(snapshot, issue.number),
            )
        })
        .collect()
}

/// Whether `pr` is the landing PR of a milestone whose catch-up is due:
/// its repair waits for the catch-up, which may resolve it.
fn landing_caught_up(snapshot: &Snapshot, pr: &Pr) -> bool {
    crate::common::branching::milestone_number(&pr.head)
        .is_some_and(|m| snapshot.catch_up.iter().any(|(due, _)| *due == m))
}

/// Whether `pr` waits ([`Snapshot::prs_waiting`]).
fn waits(snapshot: &Snapshot, pr: &str) -> bool {
    snapshot.prs_waiting.iter().any(|(held, _)| held.num == pr)
}

/// Whether `issue` is the task of a PR that waits: no decision is paid on
/// it until the wait settles.
fn task_waits(snapshot: &Snapshot, issue: &Issue) -> bool {
    snapshot
        .prs_waiting
        .iter()
        .any(|(pr, _)| crate::dev_loop::data::tasks::pr_names_task(pr, issue))
}

/// The issue or PR a [`Route::Resume`] works on, and its milestone: the
/// PR's number, else the milestone of the branch it lifts on.
fn resume_found(settled: &Settled) -> Found {
    let milestone = settled
        .lift
        .as_deref()
        .and_then(crate::common::branching::milestone_number);
    let key = settled
        .pr
        .as_deref()
        .and_then(|pr| pr.parse::<u64>().ok())
        .or(milestone)
        .unwrap_or(settled.contest);
    (
        Route::Resume {
            contest: settled.contest,
            pr: settled.pr.clone(),
            lift: settled.lift.clone(),
        },
        key,
        milestone,
    )
}

/// What is followed through for free whatever the base, since it merges
/// nothing: the settled contests (a file lifted, a PR resumed), the tasks
/// merged outside the train (closed), the PRs whose contract landed
/// (resumed), the test-debt PRs that cannot land (closed).
fn followed_through(snapshot: &Snapshot) -> Vec<Found> {
    let mut found: Vec<Found> = snapshot.settled.iter().map(resume_found).collect();
    found.extend(snapshot.tasks_merged.iter().map(|done| {
        (
            Route::CloseMerged {
                task: done.task,
                pr: done.pr,
                branch: done.branch.clone(),
            },
            done.task,
            crate::common::branching::milestone_number(&done.branch),
        )
    }));
    found.extend(snapshot.contracts_landed.iter().filter_map(|landed| {
        landed.pr.parse::<u64>().ok().map(|number| {
            (
                Route::ContractLanded {
                    request: landed.request,
                    pr: landed.pr.clone(),
                },
                number,
                None,
            )
        })
    }));
    // A debt PR that cannot land is closed whatever the base — it merges
    // nothing — except one dropped for its checks while the base is red:
    // a red integration branch fails every debt PR's CI alike, and closing
    // them wastes sessions whose tests may be fine.
    let base_red = snapshot.base_checks == Some(false);
    found.extend(snapshot.debt_landings.iter().filter_map(|landing| {
        let Landing::Drop {
            issue,
            pr,
            why,
            red,
        } = landing
        else {
            return None;
        };
        if *red && base_red {
            return None;
        }
        pr.num.parse::<u64>().ok().map(|number| {
            (
                Route::DropDebtPr {
                    issue: *issue,
                    pr: pr.num.clone(),
                    why: why.clone(),
                },
                number,
                None,
            )
        })
    }));
    found
}

/// The test-debt merge due: the first green one. A test-debt PR moves the
/// integration branch, and every building milestone takes it — a catch-up,
/// its PRs' checks run again — so one at a time, paced by
/// [`hold_debt_merges`].
fn debt_merge(snapshot: &Snapshot) -> Option<Found> {
    snapshot
        .debt_landings
        .iter()
        .find_map(|landing| match landing {
            Landing::Merge { issue, pr } => pr.num.parse::<u64>().ok().map(|n| {
                (
                    Route::MergeDebt {
                        issue: *issue,
                        pr: pr.num.clone(),
                    },
                    n,
                    None,
                )
            }),
            Landing::Drop { .. } => None,
        })
}

/// What is in flight: the catch-ups, the merges, the plan, the rules'
/// tests, the decisions and contracts asked, the PRs to repair or review.
fn in_flight(snapshot: &Snapshot) -> Vec<Found> {
    let mut found: Vec<Found> = followed_through(snapshot);
    // Nothing merges on a red base: a merge into a milestone would spread
    // it, a merge into the base would bury it. A catch-up whose landing PR
    // a repair works on waits for it (`catch_up::catch_up`, which reads the
    // PR): two writers on a branch.
    if !base_red(snapshot) {
        found.extend(snapshot.catch_up.iter().map(|(m, branch)| {
            (
                Route::CatchUp {
                    milestone: *m,
                    branch: branch.clone(),
                },
                *m,
                Some(*m),
            )
        }));
        found.extend(
            snapshot
                .ready_to_merge
                .iter()
                .map(|m| (Route::MergeMainAgent { milestone: *m }, *m, Some(*m))),
        );
        found.extend(
            snapshot
                .tasks_to_merge
                .iter()
                .filter(|(_, pr)| !waits(snapshot, &pr.num))
                .map(|(task, pr)| {
                    (
                        Route::MergeIntoMilestone {
                            task: *task,
                            pr: pr.num.clone(),
                            base: pr.base.clone(),
                        },
                        *task,
                        crate::common::branching::milestone_number(&pr.base),
                    )
                }),
        );
        found.extend(debt_merge(snapshot));
    }
    if let Some(lowest) = plannable_roadmap(&snapshot.roadmap)
        && lowest.has(labels::READY)
        && !snapshot.lowest_roadmap_has_milestone
    {
        found.push((
            Route::Planner {
                roadmap: lowest.number,
            },
            lowest.number,
            None,
        ));
    }
    // A rules' task whose faithful PR is still checking is not attempted
    // again: the attempt would judge the same PR faithful again
    // (ForgeDesMondes' #517, attempted three times while PR #518 waited on
    // `rust`).
    let ready_rules: Vec<Issue> = snapshot
        .rules
        .iter()
        .filter(|issue| issue.has(labels::READY) && !snapshot.rules_on_pr.contains(&issue.number))
        .cloned()
        .collect();
    found.extend(of_issues(snapshot, &ready_rules, |issue| Route::Rules {
        issue,
    }));
    // A decision on the task of a PR that waits is not paid: whatever it
    // decides, the PR waits first (ForgeDesMondes' #486: two decisions
    // "wait on #517" while the contest was open).
    let deciding: Vec<Issue> = snapshot
        .deciding
        .iter()
        .filter(|issue| !task_waits(snapshot, issue))
        .cloned()
        .collect();
    found.extend(of_issues(snapshot, &deciding, |issue| Route::Decide {
        issue,
    }));
    found.extend(of_issues(snapshot, &snapshot.contract_changes, |issue| {
        Route::ContractChange { issue }
    }));
    let of_pr = |pr: &Pr, route: Route| {
        pr.num.parse::<u64>().ok().map(|number| {
            (
                route,
                number,
                crate::common::branching::milestone_number(&pr.base),
            )
        })
    };
    if !snapshot.database_down {
        found.extend(
            snapshot
                .prs_to_fix
                .iter()
                .filter(|pr| !landing_caught_up(snapshot, pr) && !waits(snapshot, &pr.num))
                .filter_map(|pr| of_pr(pr, Route::PrFix { pr: pr.num.clone() })),
        );
    }
    found.extend(snapshot.prs_to_review.iter().filter_map(|pr| {
        of_pr(
            pr,
            Route::PrReview {
                pr: pr.num.clone(),
                base: pr.base.clone(),
            },
        )
    }));
    found
}

/// What builds and prepares: the splits, the graph questions, the tasks of
/// the building milestones — none while a rules' task is open, none on a
/// red base, which the task's milestone branch would take first — the
/// refinements and the test debt.
fn to_build(snapshot: &Snapshot, running: &[u64]) -> Vec<Found> {
    let mut found: Vec<Found> = splits(&snapshot.milestones, snapshot.graph.as_ref())
        .into_iter()
        .map(|milestone| (Route::Split { milestone }, milestone, Some(milestone)))
        .collect();
    found.extend(of_issues(snapshot, &snapshot.arbitrating, |issue| {
        Route::Arbiter { issue }
    }));
    if rules_first(snapshot).is_none()
        && !base_red(snapshot)
        && !snapshot.database_down
        && !snapshot.review_backlog_full
    {
        for milestone in &snapshot.dev_loop_milestones {
            // A branch behind the base, or one that could not take it, is
            // no base to build on: its tasks wait for the catch-up.
            if snapshot.catch_up.iter().any(|(m, _)| m == milestone)
                || snapshot.catch_up_held.contains(milestone)
            {
                continue;
            }
            let Some(board) = snapshot
                .boards
                .iter()
                .find(|b| b.milestone.number == *milestone)
            else {
                // No board read for it: its dev loop picks its own task.
                found.push((
                    Route::DevLoop {
                        milestone: *milestone,
                    },
                    *milestone,
                    Some(*milestone),
                ));
                continue;
            };
            found.extend(startable(snapshot, board, running).into_iter().map(|task| {
                (
                    Route::DevTask {
                        milestone: *milestone,
                        task: task.number,
                    },
                    task.number,
                    Some(*milestone),
                )
            }));
        }
    }
    found.extend(of_issues(snapshot, &snapshot.refining, |issue| {
        Route::Refinement { issue }
    }));
    found.extend(of_issues(snapshot, &snapshot.tech_refining, |issue| {
        Route::TechRefinement { issue }
    }));
    found.extend(open(&snapshot.test_debt).map(|issue| {
        (
            Route::TestDebt {
                issue: issue.number,
            },
            issue.number,
            None,
        )
    }));
    found
}

/// What a tick starts of a [`schedule`], and what it leaves to wait.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// The free jobs, run in the tick in this order.
    pub free: Vec<Job>,
    /// The paid jobs, one per free lane, heaviest first.
    pub paid: Vec<Job>,
    /// What waits a tick more ([`Pulse::age`]).
    pub left: Vec<Route>,
}

/// Picks from `jobs` (heaviest first) what this tick starts.
///
/// Every free job, and paid ones for the `slots` free lanes — none when
/// `paid` is false, a quota rest. One job per issue or PR (`running` and
/// those taken now), one per exclusive group (`held` and those taken now);
/// a job on a lane already is not waiting, one held back by a group or a
/// slot is.
#[must_use]
pub fn plan(
    jobs: Vec<Job>,
    running: &[u64],
    held: &[&'static str],
    slots: usize,
    paid: bool,
) -> Plan {
    let mut taken: Vec<u64> = running.to_vec();
    let mut groups: Vec<&'static str> = held.to_vec();
    let mut picked = Plan::default();
    for job in jobs {
        if taken.contains(&job.key) {
            continue;
        }
        if job.exclusive.is_some_and(|group| groups.contains(&group)) {
            picked.left.push(job.route);
            continue;
        }
        if job.cost == Cost::Paid && (!paid || picked.paid.len() >= slots) {
            picked.left.push(job.route);
            continue;
        }
        taken.push(job.key);
        groups.extend(job.exclusive);
        match job.cost {
            Cost::Free => picked.free.push(job),
            Cost::Paid => picked.paid.push(job),
        }
    }
    picked
}

/// What may start of a building milestone's tasks.
///
/// Runnable, not on a lane, no aggregate another holds, not held by its own
/// pull request ([`pr_cycle_owns`]) — and, while the producers of what the
/// milestone reads still build, only what runs ahead of them
/// ([`crate::dev_loop::data::tasks::runs_ahead`]).
#[must_use]
pub fn startable<'a>(
    snapshot: &Snapshot,
    board: &'a crate::dev_loop::data::board::Board,
    running: &[u64],
) -> Vec<&'a Issue> {
    let milestone = board.milestone.number;
    let waiting_on = crate::main_agent_merge::data::merge_train::producers_landed(
        snapshot.graph.as_ref(),
        milestone,
        &snapshot.milestones,
    );
    let may_start = |task: &Issue| {
        !pr_cycle_owns(snapshot, task)
            && (waiting_on.is_empty()
                || crate::dev_loop::data::tasks::runs_ahead(
                    task,
                    milestone,
                    snapshot.graph.as_ref(),
                    &waiting_on,
                ))
    };
    crate::dev_loop::data::tasks::startable_tasks(&board.tasks, running, &may_start)
}

/// Whether a task's branch is held by its pull request's own cycle.
///
/// An open PR of [`Snapshot::task_prs`] names it
/// ([`crate::dev_loop::data::tasks::pr_names_task`]), or
/// [`Snapshot::tasks_to_merge`] lists it. Never two writers on one branch. `ForgeDesMondes`, 2026-10-18: three
/// write-side sessions were killed waiting on their PR's CI, their rounds
/// filed `FAILED` and their tasks left unmarked; the next tick sent a second
/// session onto #248's branch while a repair of its PR #264 was pushing to
/// it, and the restarted watch did it again with #264 under review and
/// #246's green PR a tick from its merge. The PR's cycle — review, repair,
/// merge — owns the branch until the PR is merged (the train closes the
/// task) or closed (the task runs again).
#[must_use]
pub fn pr_cycle_owns(snapshot: &Snapshot, task: &Issue) -> bool {
    snapshot
        .tasks_to_merge
        .iter()
        .any(|(number, _)| *number == task.number)
        || snapshot
            .task_prs
            .iter()
            .any(|pr| pr.is_open() && crate::dev_loop::data::tasks::pr_names_task(pr, task))
}

/// The open issues of a list, in number order.
fn open(issues: &[Issue]) -> impl Iterator<Item = &Issue> {
    let mut found: Vec<&Issue> = issues.iter().filter(|issue| issue.is_open()).collect();
    found.sort_by_key(|issue| issue.number);
    found.into_iter()
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
    /// read in full every tick while one is open. A red integration branch
    /// is watched the same way: the push that turns it green moves no issue.
    pub watching_ci: bool,
    /// Ticks since the last full read.
    pub quiet_ticks: u32,
    /// The rest after a `QUOTA` stop.
    pub rest: Rest,
    /// How many times this watch marked each issue `harness:ready` itself
    /// (`--no-human-gate`).
    pub taps: std::collections::BTreeMap<u64, u32>,
    /// How many ticks each route has waited, the job a tick left behind —
    /// what keeps a low one from being starved ([`schedule`]).
    pub waiting: std::collections::BTreeMap<String, u32>,
    /// The integration branch's head and what each milestone branch holds
    /// of it ([`crate::common::catch_up`]).
    pub catch_ups: CatchUps,
    /// How many full reads each settled contest has kept its PR waiting
    /// ([`PrWait::Settling`]) — what tells a stalled one ([`tell_settling`]).
    pub settling: std::collections::BTreeMap<u64, u32>,
    /// How many full reads each rules' task has been held on its faithful
    /// PR ([`Snapshot::rules_on_pr`]) — what bounds the hold
    /// ([`bound_rules_on_pr`]).
    pub rules_held: std::collections::BTreeMap<u64, u32>,
    /// When this watch last attempted a test-debt merge that did not fail,
    /// in seconds ([`hold_debt_merges`]).
    pub debt_merged_at: Option<u64>,
}

/// The least time between two test-debt merges, in seconds.
///
/// Each moves the integration branch, and every building milestone then takes it (a
/// catch-up, its task PRs' checks run again) — a debt PR is never worth
/// that more than once every few hours.
pub const DEBT_MERGE_EVERY: u64 = 3 * 3600;

/// Holds the test-debt merges while one was merged less than
/// [`DEBT_MERGE_EVERY`] ago (`now`, in seconds); the drops are never held.
pub fn hold_debt_merges(snapshot: &mut Snapshot, pulse: &Pulse, now: u64) {
    let held = pulse
        .debt_merged_at
        .is_some_and(|at| now.saturating_sub(at) < DEBT_MERGE_EVERY);
    if held {
        snapshot
            .debt_landings
            .retain(|landing| matches!(landing, Landing::Drop { .. }));
    }
}

/// How many full reads a PR waits on a settled contest before a human is
/// told ([`tell_settling`]): a catch-up takes minutes, one held takes a
/// person.
pub const SETTLING_TELL: u32 = 60;

/// How many full reads a rules' task is held on its faithful PR, at most
/// ([`bound_rules_on_pr`]): a PR with no check at all, or one that never
/// ends, reads as "checking" forever.
pub const RULES_HELD_READS: u32 = 30;

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
    /// How many ticks `route` has waited to start.
    #[must_use]
    pub fn waited(&self, route: &Route) -> u32 {
        self.waiting
            .get(&format!("{route:?}"))
            .copied()
            .unwrap_or(0)
    }

    /// A tick passed: every route of `left` waited one tick more, and a
    /// route the tick started, or no longer shows, waits no more.
    pub fn age(&mut self, left: &[Route]) {
        let mut next = std::collections::BTreeMap::new();
        for route in left {
            let key = format!("{route:?}");
            let waited = self.waiting.get(&key).copied().unwrap_or(0);
            next.insert(key, waited.saturating_add(1));
        }
        self.waiting = next;
    }

    /// A full read: each of `now` counted once more, what is no longer
    /// there forgotten. Returns the new counts, in `now`'s order.
    fn count_reads(
        counts: &mut std::collections::BTreeMap<u64, u32>,
        now: &[u64],
    ) -> Vec<(u64, u32)> {
        let mut next = std::collections::BTreeMap::new();
        let mut found = Vec::new();
        for key in now {
            let reads = counts.get(key).copied().unwrap_or(0).saturating_add(1);
            next.insert(*key, reads);
            found.push((*key, reads));
        }
        *counts = next;
        found
    }

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

    /// A `QUOTA` stop: the rest starts, and every tap this watch opened is
    /// forgotten — a session the quota refused made no attempt, and the
    /// rules' task #107 was left waiting on a person after two refusals.
    /// Returns the ticks to rest.
    pub fn quota_struck(&mut self) -> u32 {
        self.taps.clear();
        self.rest.struck()
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
///
/// Counted per watch process ([`Pulse::taps`]), so a restart or a quota
/// stop ([`Pulse::quota_struck`]) counts again from zero: a rules' task
/// whose attempt keeps failing is attempted twice per watch, not once in
/// all.
pub const TAP_REOPENS: u32 = 2;

/// What a watch with no human gate marks `harness:ready` itself.
///
/// Every open roadmap item, milestone and rules' task without the label,
/// and what it is, for the journal. Never an issue a person must act on
/// (`harness:human`), one a decision is pending on, a milestone already
/// split (`harness:triggered`), which would be split again, nor a rules'
/// task whose faithful PR is still checking ([`Snapshot::rules_on_pr`]) —
/// the merge train waits on it, as on a task's PR.
#[must_use]
pub fn taps_to_open(snapshot: &Snapshot) -> Vec<(u64, &'static str)> {
    let closed_to_the_watch = |issue: &Issue| {
        !issue.is_open()
            || issue.has(labels::READY)
            || issue.has(labels::HUMAN)
            || issue.has(labels::NEEDS_DECISION)
            || snapshot.rules_on_pr.contains(&issue.number)
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

/// Counts a full read for each PR that waits on a settled contest.
///
/// Kept in [`Pulse::settling`]; the one whose wait reaches
/// [`SETTLING_TELL`] is told — once per watch, a warning a human hears
/// (`Event::SettlingStalled`). Each other wait is said in debug.
pub fn tell_settling(snapshot: &Snapshot, pulse: &mut Pulse, log: &Logbook) {
    let settling: Vec<(u64, &Pr)> = snapshot
        .prs_waiting
        .iter()
        .filter_map(|(pr, wait)| match wait {
            PrWait::Settling { issue } => Some((*issue, pr)),
            _ => None,
        })
        .collect();
    let contests: Vec<u64> = settling.iter().map(|(contest, _)| *contest).collect();
    for (contest, reads) in Pulse::count_reads(&mut pulse.settling, &contests) {
        let Some((_, pr)) = settling.iter().find(|(c, _)| *c == contest) else {
            continue;
        };
        if reads == SETTLING_TELL {
            log.event(&Event::SettlingStalled {
                contest,
                pr: pr.num.clone(),
                reads,
            });
        } else {
            log.debug(&format!(
                "pr #{} waits on contest #{contest}, settled, for its base to take the rules \
                 ({reads} read(s))",
                pr.num
            ));
        }
    }
}

/// Bounds how long a rules' task is held on its faithful PR.
///
/// A full read counts once more each task held ([`Pulse::rules_held`]);
/// one held past [`RULES_HELD_READS`] is released from
/// [`Snapshot::rules_on_pr`] — tapped and attempted again, within
/// [`TAP_REOPENS`] — and said once, a warning: its PR reads as checking
/// with no end (no check at all, or one that never concludes).
pub fn bound_rules_on_pr(snapshot: &mut Snapshot, pulse: &mut Pulse, log: &Logbook) {
    let held = snapshot.rules_on_pr.clone();
    for (task, reads) in Pulse::count_reads(&mut pulse.rules_held, &held) {
        if reads <= RULES_HELD_READS {
            continue;
        }
        snapshot.rules_on_pr.retain(|on| *on != task);
        if reads == RULES_HELD_READS.saturating_add(1) {
            log.warn(&format!(
                "watch: rules' task #{task} — its faithful PR still reads as checking after \
                 {RULES_HELD_READS} reads (no check at all, or one that never ends): attempted \
                 again"
            ));
        }
    }
}

/// Fills `snapshot`'s catch-ups from what the pulse remembers, at `now`.
///
/// [`CatchUps::read`]: those due — behind the integration branch's head,
/// once that very commit's checks are green — and the milestones whose
/// tasks wait: their branch could not take the head, or lags it. The
/// verdict is the head's own (`catch_up::read_head`), never the branch's
/// newest run, which a push just made still leaves to the head before. A
/// head whose checks have not concluded
/// [`catch_up::LAG_HOLD_SECONDS`](crate::common::catch_up::LAG_HOLD_SECONDS)
/// after its commit releases the tasks of the branches lagging it: said
/// once, as a warning.
pub fn read_catch_ups(snapshot: &mut Snapshot, pulse: &mut Pulse, now: u64, log: &Logbook) {
    (snapshot.catch_up, snapshot.catch_up_held) = pulse.catch_ups.read(&snapshot.milestones, now);
    if let Some((head, released)) = pulse.catch_ups.lag_expiry(&snapshot.milestones, now) {
        let milestones: Vec<String> = released.iter().map(|number| format!("#{number}")).collect();
        log.warn(&format!(
            "watch: the integration branch's head {} has no verdict {} min after its commit — \
             the tasks of milestone(s) {} start on their branch as it is",
            head.get(..7).unwrap_or(&head),
            crate::common::catch_up::LAG_HOLD_SECONDS / 60,
            milestones.join(", ")
        ));
    }
}

/// Whether this tick may skip the full read of the board.
///
/// Only when the one conditional read said nothing moved, the catch-ups
/// ask for no read either (`catching_up`, `catch_up::read_head`: the
/// integration branch's head moved — a push that updates no issue, an
/// upgrade, a human's, is what a catch-up follows — its verdict came in, or
/// a catch-up due is unsettled), no pull request's checks or review are
/// being waited on, no lane was just freed — a lane freed with the board
/// unchanged is a crashed lane whose work is still due — and the valve has
/// not run out.
#[must_use]
pub const fn may_skip(
    pulse: &Pulse,
    activity: &Activity,
    freed_lanes: usize,
    catching_up: bool,
) -> bool {
    matches!(activity, Activity::Unchanged)
        && !catching_up
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
/// A candidate, and what it waits on:
///
/// - a milestone the plan's graph knows (contract R) waits on the producers
///   of what it consumes, until each is **split** (`harness:triggered`) or
///   delivered — closed, or `waiting-merge` with its code on the integration
///   branch: from there the contract it reads is in place, mocked until
///   built, and the two build side by side. A `blocked_by` on another
///   milestone the graph knows is the planner's own chaining, which the
///   graph supersedes; a blocker the graph does not know — a person's task —
///   must be delivered;
/// - one the graph does not know waits on every milestone it is `blocked_by`
///   and every milestone before it (a lower number) delivered, the order of a
///   plan written without a graph being the dependency it does not write.
///
/// `milestone.blocked_by` must have been read ([`GitHub::with_blockers`]);
/// empty reads as nothing blocking.
///
/// [`GitHub::with_blockers`]: harness_core::ports::shell::github::GitHub::with_blockers
#[must_use]
pub fn splittable(milestone: &Issue, milestones: &[Issue], graph: Option<&Graph>) -> bool {
    let delivered = |other: &Issue| other.is_closed() || other.has(labels::WAITING_MERGE);
    if !split_candidate(milestone) {
        return false;
    }
    let Some(graph) = graph.filter(|graph| graph.node(milestone.number).is_some()) else {
        return milestone.blocked_by.iter().all(delivered)
            && milestones
                .iter()
                .filter(|other| other.number < milestone.number)
                .all(delivered);
    };
    milestone
        .blocked_by
        .iter()
        .filter(|blocker| graph.node(blocker.number).is_none())
        .all(delivered)
        && graph.waits_on(milestone.number).iter().all(|producer| {
            milestones
                .iter()
                .find(|other| other.number == *producer)
                .is_none_or(|other| delivered(other) || other.has(labels::TRIGGERED))
        })
}

/// How many milestones build at once, at most, beyond the critical path's:
/// the ahead work is taken on the resources left, never instead of what the
/// path waits on.
pub const MAX_CURRENT: usize = 3;

/// The milestones to split now, in the order to split them.
///
/// The critical path's **next step** first ([`critical_path`]) — the one
/// path milestone not yet building, whatever the cap: the path never waits
/// on the ahead work — then the rest by number, those only while fewer than
/// [`MAX_CURRENT`] milestones build (`harness:triggered`, not delivered).
/// Every path member passing the cap at once put eight milestones in
/// construction on `ForgeDesMondes`.
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
    let next_on_path = path.iter().copied().find(|number| {
        milestones
            .iter()
            .find(|m| m.number == *number)
            .is_some_and(|m| m.is_open() && !m.has(labels::TRIGGERED))
    });
    let mut room = MAX_CURRENT.saturating_sub(building);
    found.retain(|number| {
        if Some(*number) == next_on_path {
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

/// The milestones whose boards a tick reads, in the order their tasks are
/// served.
///
/// Every one building (`harness:triggered`, open, not delivered), the
/// critical path's first; the lowest open milestone when none is split yet,
/// as the single-milestone watch read it.
#[must_use]
pub fn building(milestones: &[Issue], graph: Option<&Graph>) -> Vec<u64> {
    let mut found: Vec<u64> = milestones
        .iter()
        .filter(|m| m.is_open() && m.has(labels::TRIGGERED) && !m.has(labels::WAITING_MERGE))
        .map(|m| m.number)
        .collect();
    by_critical_path(&mut found, &critical_path(milestones, graph));
    if found.is_empty()
        && let Some(lowest) = milestones
            .iter()
            .filter(|m| m.is_open() && m.has(labels::MILESTONE))
            .min_by_key(|m| m.number)
    {
        found.push(lowest.number);
    }
    found
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
        assert_eq!(pulse.quota_struck(), FIRST_REST);
        assert!(pulse.may_open(2), "a refusal on the quota was no attempt");
        assert_eq!(pulse.rest.strikes, 1);
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
            prs_to_fix: Vec::new(),
            prs_to_review: Vec::new(),
            tasks_to_merge: Vec::new(),
            task_prs: Vec::new(),
            ready_to_merge: Vec::new(),
            refining: Vec::new(),
            tech_refining: Vec::new(),
            arbitrating: Vec::new(),
            dev_loop_milestones: Vec::new(),
            boards: Vec::new(),
            graph: None,
            contract_changes: Vec::new(),
            test_debt: Vec::new(),
            rules: Vec::new(),
            deciding: Vec::new(),
            base_checks: None,
            database_down: false,
            review_backlog_full: false,
            catch_up: Vec::new(),
            catch_up_held: Vec::new(),
            prs_waiting: Vec::new(),
            rules_on_pr: Vec::new(),
            settled: Vec::new(),
            tasks_merged: Vec::new(),
            contracts_landed: Vec::new(),
            debt_landings: Vec::new(),
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
            tasks_to_merge: vec![(64, pr("68", "milestone/52-socle"))],
            ready_to_merge: vec![52],
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
    fn a_ready_roadmap_item_with_no_milestone_outranks_every_paid_route() {
        let snapshot = Snapshot {
            roadmap: vec![
                issue(9, &[labels::ROADMAP, labels::READY]),
                issue(4, &[labels::ROADMAP, labels::READY]),
            ],
            lowest_roadmap_has_milestone: false,
            graph: None,
            boards: Vec::new(),
            contract_changes: Vec::new(),
            milestones: vec![issue(5, &[labels::MILESTONE, labels::READY])],
            prs_to_fix: vec![pr("32", "main_agent")],
            prs_to_review: vec![pr("33", "main_agent")],
            tasks_to_merge: vec![(40, pr("41", "milestone/3-x"))],
            task_prs: Vec::new(),
            ready_to_merge: vec![7],
            refining: vec![issue(6, &[labels::REFINEMENT])],
            tech_refining: vec![],
            arbitrating: Vec::new(),
            dev_loop_milestones: vec![3],
            test_debt: vec![issue(8, &[labels::TEST_DEBT])],
            rules: vec![issue(2, &[labels::RULES, labels::READY])],
            deciding: vec![issue(3, &[labels::NEEDS_DECISION])],
            base_checks: Some(true),
            database_down: false,
            review_backlog_full: false,
            catch_up: Vec::new(),
            catch_up_held: Vec::new(),
            prs_waiting: Vec::new(),
            rules_on_pr: Vec::new(),
            settled: Vec::new(),
            tasks_merged: Vec::new(),
            contracts_landed: Vec::new(),
            debt_landings: Vec::new(),
        };
        // The free merges go first, in the tick; then the plan.
        assert_eq!(
            routes(&snapshot)[..3],
            [
                Route::MergeIntoMilestone {
                    task: 40,
                    pr: "41".to_string(),
                    base: "milestone/3-x".to_string(),
                },
                Route::MergeMainAgent { milestone: 7 },
                Route::Planner { roadmap: 4 },
            ]
        );
    }

    #[test]
    fn a_full_review_backlog_holds_new_tasks_and_keeps_reviews_repairs_and_merges() {
        let plant = |full: bool| Snapshot {
            tasks_to_merge: vec![(64, pr("68", "milestone/52-socle"))],
            dev_loop_milestones: vec![52],
            boards: vec![board(52, vec![issue(60, &[labels::AGENT, labels::READY])])],
            prs_to_fix: vec![pr("32", "milestone/52-socle")],
            prs_to_review: vec![pr("33", "milestone/52-socle")],
            review_backlog_full: full,
            ..empty()
        };
        let full = routes(&plant(true));
        assert!(
            !full
                .iter()
                .any(|r| matches!(r, Route::DevLoop { .. } | Route::DevTask { .. })),
            "{full:?}"
        );
        for kept in [
            |r: &Route| matches!(r, Route::PrFix { .. }),
            |r: &Route| matches!(r, Route::PrReview { .. }),
            |r: &Route| matches!(r, Route::MergeIntoMilestone { .. }),
        ] {
            assert!(full.iter().any(kept), "{full:?}");
        }
        let room = routes(&plant(false));
        assert!(
            room.iter()
                .any(|r| matches!(r, Route::DevLoop { .. } | Route::DevTask { .. })),
            "{room:?}"
        );
    }

    #[test]
    fn a_database_down_holds_the_code_and_the_repairs_and_nothing_else() {
        // ForgeDesMondes, 2026-10-19/20: Docker down all night, every code
        // session ran its tests without a database, red in CI after.
        let plant = |down: bool| Snapshot {
            tasks_to_merge: vec![(64, pr("68", "milestone/52-socle"))],
            dev_loop_milestones: vec![52],
            boards: vec![board(52, vec![issue(60, &[labels::AGENT, labels::READY])])],
            prs_to_fix: vec![pr("32", "milestone/52-socle")],
            prs_to_review: vec![pr("33", "milestone/52-socle")],
            database_down: down,
            review_backlog_full: false,
            ..empty()
        };
        let down = routes(&plant(true));
        assert!(
            !down.iter().any(|r| matches!(
                r,
                Route::PrFix { .. } | Route::DevLoop { .. } | Route::DevTask { .. }
            )),
            "{down:?}"
        );
        assert!(
            down.iter()
                .any(|r| matches!(r, Route::MergeIntoMilestone { .. })),
            "merges go on: {down:?}"
        );
        assert!(
            down.iter().any(|r| matches!(r, Route::PrReview { .. })),
            "reviews go on: {down:?}"
        );
        let up = routes(&plant(false));
        assert!(
            up.iter().any(|r| matches!(r, Route::PrFix { .. })),
            "{up:?}"
        );
        assert!(
            up.iter()
                .any(|r| matches!(r, Route::DevLoop { .. } | Route::DevTask { .. })),
            "{up:?}"
        );
    }

    #[test]
    fn a_red_base_holds_the_merges_and_the_tasks_and_nothing_else() {
        // The live failure: main_agent red for two hours while the watch kept
        // starting tasks on it and opening PRs that inherited the red.
        let plant = |checks: Option<bool>| Snapshot {
            tasks_to_merge: vec![(64, pr("68", "milestone/52-socle"))],
            ready_to_merge: vec![52],
            dev_loop_milestones: vec![52],
            boards: vec![board(52, vec![issue(60, &[labels::AGENT, labels::READY])])],
            prs_to_fix: vec![pr("32", "milestone/52-socle")],
            prs_to_review: vec![pr("33", "milestone/52-socle")],
            deciding: vec![issue(3, &[labels::NEEDS_DECISION])],
            test_debt: vec![issue(8, &[labels::TEST_DEBT])],
            base_checks: checks,
            database_down: false,
            review_backlog_full: false,
            ..empty()
        };
        let red = plant(Some(false));
        assert!(base_red(&red));
        assert_eq!(
            routes(&red),
            [
                Route::Decide { issue: 3 },
                Route::PrFix {
                    pr: "32".to_string()
                },
                Route::PrReview {
                    pr: "33".to_string(),
                    base: "milestone/52-socle".to_string(),
                },
                Route::TestDebt { issue: 8 },
            ],
            "no merge, no task: what does not touch the base goes on"
        );
        assert_eq!(decide(&red), Route::Decide { issue: 3 });
        // The rules' tests go on too; a milestone whose board was not read
        // does not run its loop either.
        let rules = Snapshot {
            rules: vec![issue(5, &[labels::RULES, labels::READY])],
            boards: Vec::new(),
            ..plant(Some(false))
        };
        assert_eq!(decide(&rules), Route::Rules { issue: 5 });
        assert!(
            !routes(&Snapshot {
                boards: Vec::new(),
                ..plant(Some(false))
            })
            .contains(&Route::DevLoop { milestone: 52 }),
            "no board read: its loop would pick a task on the red"
        );
        // Green again, or not known yet: the merges and the task are back,
        // the merges first.
        for checks in [Some(true), None] {
            let back = plant(checks);
            assert!(!base_red(&back));
            assert_eq!(
                routes(&back),
                [
                    Route::MergeIntoMilestone {
                        task: 64,
                        pr: "68".to_string(),
                        base: "milestone/52-socle".to_string(),
                    },
                    Route::MergeMainAgent { milestone: 52 },
                    Route::Decide { issue: 3 },
                    Route::PrFix {
                        pr: "32".to_string()
                    },
                    Route::PrReview {
                        pr: "33".to_string(),
                        base: "milestone/52-socle".to_string(),
                    },
                    Route::DevTask {
                        milestone: 52,
                        task: 60,
                    },
                    Route::TestDebt { issue: 8 },
                ],
                "{checks:?}"
            );
            assert!(
                routes(&Snapshot {
                    boards: Vec::new(),
                    ..plant(checks)
                })
                .contains(&Route::DevLoop { milestone: 52 })
            );
        }
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
        assert!(
            !routes(&waiting)
                .iter()
                .any(|r| matches!(r, Route::DevLoop { .. }))
        );
        // Settled: the development goes on.
        let settled = Snapshot {
            rules: vec![settled],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(rules_first(&settled), None);
        assert_eq!(decide(&settled), Route::DevLoop { milestone: 3 });
        assert!(routes(&settled).contains(&Route::DevLoop { milestone: 3 }));
    }

    #[test]
    fn a_contest_holds_only_the_pr_that_contests() {
        let mut contest = issue(517, &[labels::RULES, labels::READY]);
        contest.title = "Rules contested: PR #516".to_string();
        let snapshot = Snapshot {
            rules: vec![contest],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(rules_first(&snapshot), None, "no development waits on it");
        let found = routes(&snapshot);
        assert!(found.contains(&Route::Rules { issue: 517 }), "{found:?}");
        assert!(
            found.contains(&Route::DevLoop { milestone: 3 }),
            "{found:?}"
        );
        // The first rules' task still holds everything.
        let mut first = issue(5, &[labels::RULES]);
        first.title = crate::rules::data::request::FIRST_TITLE.to_string();
        let both = Snapshot {
            rules: vec![first, snapshot.rules[0].clone()],
            ..snapshot
        };
        assert_eq!(rules_first(&both), Some(5));
    }

    /// PR #516 into milestone/27, of task #486, waiting on contest #517.
    fn waiting_on_its_contest() -> (Pr, Issue) {
        let mut task = issue(486, &[labels::AGENT, labels::NEEDS_DECISION]);
        task.body = "## Scope\n\nbranch: feat/scene\n".to_string();
        let pr = Pr {
            head: "feat/scene".to_string(),
            ..pr("516", "milestone/27-x")
        };
        (pr, task)
    }

    #[test]
    fn a_pr_waiting_on_its_contest_is_neither_repaired_nor_decided_nor_merged() {
        let (waiting, task) = waiting_on_its_contest();
        let other = pr("520", "milestone/27-x");
        let snapshot = Snapshot {
            prs_to_fix: vec![waiting.clone(), other],
            prs_to_review: vec![waiting.clone()],
            tasks_to_merge: vec![(486, waiting.clone())],
            deciding: vec![task, issue(600, &[labels::NEEDS_DECISION])],
            prs_waiting: vec![(waiting, PrWait::Contest { issue: 517 })],
            ..empty()
        };
        let found = routes(&snapshot);
        assert!(
            !found.contains(&Route::PrFix {
                pr: "516".to_string()
            }),
            "{found:?}"
        );
        assert!(
            found.contains(&Route::PrFix {
                pr: "520".to_string()
            }),
            "{found:?}"
        );
        assert!(!found.contains(&Route::Decide { issue: 486 }), "{found:?}");
        assert!(found.contains(&Route::Decide { issue: 600 }), "{found:?}");
        assert!(
            !found
                .iter()
                .any(|r| matches!(r, Route::MergeIntoMilestone { .. })),
            "{found:?}"
        );
        assert!(
            found.contains(&Route::PrReview {
                pr: "516".to_string(),
                base: "milestone/27-x".to_string()
            }),
            "the review does not depend on the contest: {found:?}"
        );
    }

    #[test]
    fn a_red_pr_fix_pr_past_its_repairs_waits_too() {
        // A decided wait holds the `to_fix` branch of the repair route, even
        // within its repairs.
        let (waiting, _) = waiting_on_its_contest();
        let snapshot = Snapshot {
            prs_to_fix: vec![waiting.clone()],
            prs_waiting: vec![(waiting, PrWait::Decided { issue: 20 })],
            ..empty()
        };
        assert!(routes(&snapshot).is_empty(), "{:?}", routes(&snapshot));
    }

    #[test]
    fn a_pr_settling_on_a_closed_contest_is_neither_repaired_nor_decided_nor_merged() {
        // The window between the contest closing and the resume: the
        // catch-up and the resume are due, never a paid repair beside them.
        let (waiting, task) = waiting_on_its_contest();
        let snapshot = Snapshot {
            prs_to_fix: vec![waiting.clone()],
            prs_to_review: vec![waiting.clone()],
            tasks_to_merge: vec![(486, waiting.clone())],
            deciding: vec![task],
            prs_waiting: vec![(waiting, PrWait::Settling { issue: 517 })],
            ..empty()
        };
        let found = routes(&snapshot);
        assert!(
            !found.iter().any(|r| matches!(
                r,
                Route::PrFix { .. } | Route::Decide { .. } | Route::MergeIntoMilestone { .. }
            )),
            "{found:?}"
        );
    }

    /// A sink that keeps the lines and the events told.
    #[derive(Default)]
    struct Told(std::cell::RefCell<(Vec<String>, Vec<Event>)>);

    impl harness_core::traces::Sink for Told {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().0.push(line.to_string());
        }
        fn keep(&self, _line: &str) {}
        fn record(&self, event: &Event) {
            self.0.borrow_mut().1.push(event.clone());
        }
    }

    fn told() -> (std::rc::Rc<Told>, Logbook) {
        let sink = std::rc::Rc::new(Told::default());
        let log = Logbook::new(
            std::rc::Rc::clone(&sink) as std::rc::Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Verbose,
        );
        (sink, log)
    }

    #[test]
    fn a_settling_wait_that_stalls_is_told_once() {
        let (waiting, _) = waiting_on_its_contest();
        let snapshot = Snapshot {
            prs_waiting: vec![
                (waiting, PrWait::Settling { issue: 517 }),
                (pr("40", "milestone/27-x"), PrWait::Decided { issue: 20 }),
            ],
            ..empty()
        };
        let (sink, log) = told();
        let mut pulse = Pulse::default();
        for _ in 0..SETTLING_TELL.saturating_add(3) {
            tell_settling(&snapshot, &mut pulse, &log);
        }
        let events = sink.0.borrow().1.clone();
        assert_eq!(
            events,
            [Event::SettlingStalled {
                contest: 517,
                pr: "516".to_string(),
                reads: SETTLING_TELL,
            }],
            "told once, the decided wait never"
        );
        assert_eq!(
            pulse.settling.get(&517).copied(),
            Some(SETTLING_TELL.saturating_add(3))
        );
        assert!(
            sink.0.borrow().0.iter().any(|line| line
                .contains("pr #516 waits on contest #517, settled")
                && line.contains("(1 read(s))")),
            "{:?}",
            sink.0.borrow().0
        );
        // Resumed: forgotten, so a later wait counts from zero.
        tell_settling(&empty(), &mut pulse, &log);
        assert!(pulse.settling.is_empty());
    }

    #[test]
    fn a_rules_task_held_on_its_pr_past_the_bound_is_released_and_said_once() {
        // A faithful PR with no check at all reads as checking forever.
        let (sink, log) = told();
        let mut pulse = Pulse::default();
        let held = || Snapshot {
            rules: vec![issue(517, &[labels::RULES]), issue(518, &[labels::RULES])],
            rules_on_pr: vec![517],
            ..empty()
        };
        for _ in 0..RULES_HELD_READS {
            let mut found = held();
            bound_rules_on_pr(&mut found, &mut pulse, &log);
            assert_eq!(found.rules_on_pr, [517], "held within the bound");
            assert!(taps_to_open(&found).iter().all(|(n, _)| *n != 517));
        }
        for _ in 0..3 {
            let mut found = held();
            bound_rules_on_pr(&mut found, &mut pulse, &log);
            assert!(found.rules_on_pr.is_empty(), "released past the bound");
            assert!(taps_to_open(&found).contains(&(517, "rules' task")));
        }
        let warned: Vec<String> = sink
            .0
            .borrow()
            .0
            .iter()
            .filter(|line| line.contains("rules' task #517"))
            .cloned()
            .collect();
        assert_eq!(warned.len(), 1, "said once: {warned:?}");
        // Off its PR: forgotten, the next hold counts from zero.
        bound_rules_on_pr(&mut empty(), &mut pulse, &log);
        assert!(pulse.rules_held.is_empty());
        let mut found = held();
        bound_rules_on_pr(&mut found, &mut pulse, &log);
        assert_eq!(found.rules_on_pr, [517]);
    }

    #[test]
    fn a_rules_task_whose_pr_is_checking_is_neither_tapped_nor_routed() {
        let snapshot = Snapshot {
            rules: vec![
                issue(517, &[labels::RULES, labels::READY]),
                issue(518, &[labels::RULES]),
            ],
            rules_on_pr: vec![517, 518],
            ..empty()
        };
        assert!(
            !routes(&snapshot).contains(&Route::Rules { issue: 517 }),
            "{:?}",
            routes(&snapshot)
        );
        assert!(
            taps_to_open(&snapshot).is_empty(),
            "{:?}",
            taps_to_open(&snapshot)
        );
    }

    #[test]
    fn a_rules_task_whose_faithful_pr_went_red_is_attempted_again() {
        // Red is not in `rules_on_pr`: the task is tapped and routed.
        let snapshot = Snapshot {
            rules: vec![issue(517, &[labels::RULES])],
            ..empty()
        };
        assert_eq!(taps_to_open(&snapshot), [(517, "rules' task")]);
        let tapped = Snapshot {
            rules: vec![issue(517, &[labels::RULES, labels::READY])],
            ..empty()
        };
        assert!(routes(&tapped).contains(&Route::Rules { issue: 517 }));
    }

    #[test]
    fn a_settled_contest_is_a_free_resume_before_the_merges() {
        let snapshot = Snapshot {
            settled: vec![
                Settled {
                    contest: 517,
                    pr: Some("516".to_string()),
                    lift: Some("feat/scene".to_string()),
                },
                Settled {
                    contest: 459,
                    pr: None,
                    lift: Some("milestone/28-x".to_string()),
                },
            ],
            tasks_to_merge: vec![(64, pr("68", "milestone/52-socle"))],
            catch_up: vec![(28, "milestone/28-x".to_string())],
            base_checks: None,
            ..empty()
        };
        let jobs = schedule(&snapshot, &[], &|_| 0);
        let order: Vec<(&Route, u64, Cost)> =
            jobs.iter().map(|j| (&j.route, j.key, j.cost)).collect();
        assert!(
            matches!(order[0], (Route::CatchUp { .. }, 28, Cost::Free)),
            "{order:?}"
        );
        assert!(
            matches!(
                order[1],
                (Route::Resume { contest: 517, .. }, 516, Cost::Free)
            ),
            "{order:?}"
        );
        assert!(
            matches!(
                order[2],
                (Route::Resume { contest: 459, .. }, 28, Cost::Free)
            ),
            "keyed on its milestone: the catch-up of the branch goes first, {order:?}"
        );
        assert!(
            matches!(order[3], (Route::MergeIntoMilestone { .. }, _, Cost::Free)),
            "{order:?}"
        );
        // One job per key: the lift of milestone/28 waits for its catch-up.
        let picked = plan(jobs, &[], &[], 0, false);
        assert!(
            picked
                .free
                .iter()
                .all(|j| !matches!(j.route, Route::Resume { contest: 459, .. }))
        );
        // Followed through on a red base too: it merges nothing.
        let red = Snapshot {
            base_checks: Some(false),
            ..snapshot
        };
        assert!(
            routes(&red)
                .iter()
                .any(|r| matches!(r, Route::Resume { contest: 517, .. }))
        );
        // A lane on the PR holds it.
        let running = schedule(&red, &[516], &|_| 0);
        let picked = plan(running, &[516], &[], 0, false);
        assert!(
            !picked
                .free
                .iter()
                .any(|j| matches!(j.route, Route::Resume { contest: 517, .. }))
        );
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
            prs_to_fix: vec![pr("32", "milestone/3-x")],
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
    fn a_ready_milestone_outranks_refinement_and_the_dev_loop_after_the_merge() {
        let snapshot = Snapshot {
            milestones: vec![issue(9, &[labels::MILESTONE, labels::READY])],
            ready_to_merge: vec![7],
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(
            routes(&snapshot),
            [
                Route::MergeMainAgent { milestone: 7 },
                Route::Split { milestone: 9 },
                Route::DevLoop { milestone: 3 },
                Route::Refinement { issue: 6 },
            ]
        );
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
            ready_to_merge: vec![7],
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::MergeMainAgent { milestone: 7 });
    }

    #[test]
    fn a_green_task_pr_merges_before_a_repair_or_a_review_is_paid() {
        let snapshot = Snapshot {
            tasks_to_merge: vec![(31, pr("34", "milestone/4-territory"))],
            prs_to_fix: vec![pr("32", "milestone/4-territory")],
            prs_to_review: vec![pr("33", "milestone/4-territory")],
            ready_to_merge: vec![7],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(
            decide(&snapshot),
            Route::MergeIntoMilestone {
                task: 31,
                pr: "34".to_string(),
                base: "milestone/4-territory".to_string(),
            }
        );
    }

    #[test]
    fn a_red_pr_outranks_reviewing_refinement_and_the_dev_loop_after_the_merge() {
        // What is already in flight and broken comes before anything new:
        // the milestone it belongs to cannot close over a red PR anyway.
        let snapshot = Snapshot {
            prs_to_fix: vec![pr("32", "milestone/4-territory")],
            prs_to_review: vec![pr("33", "milestone/4-territory")],
            ready_to_merge: vec![7],
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(
            routes(&snapshot),
            [
                Route::MergeMainAgent { milestone: 7 },
                Route::PrFix {
                    pr: "32".to_string()
                },
                Route::PrReview {
                    pr: "33".to_string(),
                    base: "milestone/4-territory".to_string(),
                },
                Route::DevLoop { milestone: 3 },
                Route::Refinement { issue: 6 },
            ]
        );
    }

    #[test]
    fn a_pr_to_review_outranks_refinement_and_the_dev_loop_after_the_merge() {
        let snapshot = Snapshot {
            prs_to_review: vec![pr("33", "milestone/4-territory")],
            ready_to_merge: vec![7],
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        assert_eq!(
            routes(&snapshot),
            [
                Route::MergeMainAgent { milestone: 7 },
                Route::PrReview {
                    pr: "33".to_string(),
                    base: "milestone/4-territory".to_string(),
                },
                Route::DevLoop { milestone: 3 },
                Route::Refinement { issue: 6 },
            ]
        );
    }

    #[test]
    fn the_review_carries_the_branch_the_pr_actually_targets() {
        // A task PR targets its milestone's branch, never the integration
        // branch — a review judging against a fixed base would skip it as
        // "targets the wrong branch".
        let snapshot = Snapshot {
            prs_to_review: vec![pr("33", "milestone/9-cities")],
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
            prs_to_fix: vec![pr("32", "main_agent")],
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
        snapshot.ready_to_merge = vec![7];
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

    #[test]
    fn a_merged_task_is_closed_and_a_landed_contract_resumed_in_the_tick_for_free() {
        let snapshot = Snapshot {
            tasks_merged: vec![Delivered {
                task: 464,
                pr: 469,
                branch: "milestone/27-x".to_string(),
            }],
            contracts_landed: vec![ContractLanded {
                request: 503,
                pr: "500".to_string(),
            }],
            // A red base holds neither: they merge nothing.
            base_checks: Some(false),
            ..empty()
        };
        let jobs = schedule(&snapshot, &[], &|_| 0);
        let free: Vec<&Route> = jobs
            .iter()
            .filter(|job| job.cost == Cost::Free)
            .map(|job| &job.route)
            .collect();
        assert_eq!(
            free,
            [
                &Route::CloseMerged {
                    task: 464,
                    pr: 469,
                    branch: "milestone/27-x".to_string(),
                },
                &Route::ContractLanded {
                    request: 503,
                    pr: "500".to_string(),
                },
            ]
        );
        assert_eq!(jobs.len(), 2, "nothing paid: {jobs:?}");
    }

    fn debt(num: &str, issue: u64) -> Pr {
        Pr {
            num: num.to_string(),
            head: format!("test/debt-{issue}-1"),
            base: "main_agent".to_string(),
            ..Pr::default()
        }
    }

    #[test]
    fn a_green_debt_pr_merges_one_at_a_time_and_a_dropped_one_is_closed() {
        let landings = vec![
            Landing::Merge {
                issue: 7,
                pr: debt("40", 7),
            },
            Landing::Merge {
                issue: 8,
                pr: debt("41", 8),
            },
            Landing::Drop {
                issue: 9,
                pr: debt("42", 9),
                why: "its checks failed: rust".to_string(),
                red: true,
            },
        ];
        let snapshot = Snapshot {
            debt_landings: landings.clone(),
            ..empty()
        };
        let routes: Vec<Route> = schedule(&snapshot, &[], &|_| 0)
            .into_iter()
            .map(|job| job.route)
            .collect();
        assert_eq!(
            routes,
            [
                Route::DropDebtPr {
                    issue: 9,
                    pr: "42".to_string(),
                    why: "its checks failed: rust".to_string(),
                },
                Route::MergeDebt {
                    issue: 7,
                    pr: "40".to_string(),
                },
            ],
            "one merge a time, the drop whatever"
        );
        // Merged an hour ago: the next merge waits; a red base holds it too.
        let mut held = Snapshot {
            debt_landings: landings.clone(),
            ..empty()
        };
        let pulse = Pulse {
            debt_merged_at: Some(10_000),
            ..quiet()
        };
        hold_debt_merges(&mut held, &pulse, 10_000 + 3600);
        let routes_held: Vec<Route> = schedule(&held, &[], &|_| 0)
            .into_iter()
            .map(|job| job.route)
            .collect();
        assert!(
            !routes_held
                .iter()
                .any(|route| matches!(route, Route::MergeDebt { .. })),
            "{routes_held:?}"
        );
        assert!(
            routes_held
                .iter()
                .any(|route| matches!(route, Route::DropDebtPr { .. })),
            "a drop is never held"
        );
        let mut later = Snapshot {
            debt_landings: landings.clone(),
            ..empty()
        };
        hold_debt_merges(&mut later, &pulse, 10_000 + DEBT_MERGE_EVERY);
        assert_eq!(
            later.debt_landings.len(),
            3,
            "past the spacing it merges again"
        );
        let red = Snapshot {
            debt_landings: landings,
            base_checks: Some(false),
            ..empty()
        };
        assert!(
            !schedule(&red, &[], &|_| 0)
                .iter()
                .any(|job| matches!(job.route, Route::MergeDebt { .. }))
        );
    }

    #[test]
    fn a_red_base_holds_the_drops_for_failing_checks_only() {
        let landings = vec![
            Landing::Drop {
                issue: 9,
                pr: debt("42", 9),
                why: "its checks failed: rust".to_string(),
                red: true,
            },
            Landing::Drop {
                issue: 10,
                pr: debt("43", 10),
                why: "it conflicts with `main_agent`".to_string(),
                red: false,
            },
        ];
        let dropped = |base_checks: Option<bool>| -> Vec<u64> {
            let snapshot = Snapshot {
                debt_landings: landings.clone(),
                base_checks,
                ..empty()
            };
            schedule(&snapshot, &[], &|_| 0)
                .into_iter()
                .filter_map(|job| match job.route {
                    Route::DropDebtPr { issue, .. } => Some(issue),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(dropped(Some(false)), [10], "the red one waits for the base");
        assert_eq!(dropped(Some(true)), [9, 10]);
        assert_eq!(dropped(None), [9, 10], "an unread base holds nothing");
    }

    fn quiet() -> Pulse {
        Pulse {
            etag: Some("W/\"abc\"".to_string()),
            watching_ci: false,
            quiet_ticks: 0,
            rest: Rest::default(),
            taps: std::collections::BTreeMap::new(),
            waiting: std::collections::BTreeMap::new(),
            catch_ups: CatchUps::default(),
            settling: std::collections::BTreeMap::new(),
            rules_held: std::collections::BTreeMap::new(),
            debt_merged_at: None,
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
        assert!(may_skip(&quiet(), &Activity::Unchanged, 0, false));
    }

    #[test]
    fn a_changed_board_is_read_in_full() {
        let changed = Activity::Changed {
            etag: None,
            moved_at: None,
        };
        assert!(!may_skip(&quiet(), &changed, 0, false));
    }

    #[test]
    fn a_waited_on_pull_request_keeps_the_full_read_going() {
        let pulse = Pulse {
            watching_ci: true,
            ..quiet()
        };
        assert!(!may_skip(&pulse, &Activity::Unchanged, 0, false));
    }

    #[test]
    fn a_moved_integration_head_reads_the_board_again() {
        assert!(!may_skip(&quiet(), &Activity::Unchanged, 0, true));
    }

    #[test]
    fn a_freed_lane_reads_the_board_again() {
        assert!(!may_skip(&quiet(), &Activity::Unchanged, 1, false));
    }

    #[test]
    fn the_valve_forces_a_full_read_after_enough_quiet_ticks() {
        let mut pulse = quiet();
        for _ in 0..FULL_READ_EVERY {
            assert!(may_skip(&pulse, &Activity::Unchanged, 0, false));
            pulse.stayed_quiet();
        }
        assert!(!may_skip(&pulse, &Activity::Unchanged, 0, false));
        pulse.read_in_full(Some("W/\"def\"".to_string()), false);
        assert_eq!(pulse.quiet_ticks, 0);
        assert!(may_skip(&pulse, &Activity::Unchanged, 0, false));
    }

    /// The routes of a snapshot's schedule, heaviest first.
    fn routes(snapshot: &Snapshot) -> Vec<Route> {
        schedule(snapshot, &[], &|_| 0)
            .into_iter()
            .map(|job| job.route)
            .collect()
    }

    #[test]
    fn every_kind_of_work_is_one_job_and_all_of_a_kind_are_offered_at_once() {
        // ForgeDesMondes: four reviews ran one at a time on five lanes, and
        // one PR to merge waited behind the tick's single route.
        let snapshot = Snapshot {
            tasks_to_merge: vec![
                (31, pr("34", "milestone/4-x")),
                (32, pr("35", "milestone/4-x")),
            ],
            prs_to_review: vec![pr("40", "milestone/4-x"), pr("41", "milestone/4-x")],
            prs_to_fix: vec![pr("42", "milestone/4-x")],
            deciding: vec![
                issue(50, &[labels::NEEDS_DECISION]),
                issue(51, &[labels::NEEDS_DECISION]),
            ],
            refining: vec![issue(6, &[labels::REFINEMENT])],
            tech_refining: vec![issue(7, &[labels::TECH_REFINEMENT])],
            test_debt: vec![issue(8, &[labels::TEST_DEBT])],
            arbitrating: vec![issue(9, &[labels::NEEDS_AI_DECISION])],
            milestones: vec![issue(4, &[labels::MILESTONE, labels::READY])],
            dev_loop_milestones: vec![3],
            ready_to_merge: vec![2],
            ..empty()
        };
        let jobs = schedule(&snapshot, &[], &|_| 0);
        let keys: Vec<(u64, Cost)> = jobs.iter().map(|j| (j.key, j.cost)).collect();
        assert_eq!(
            keys,
            [
                (31, Cost::Free),
                (32, Cost::Free),
                (2, Cost::Free),
                (50, Cost::Paid),
                (51, Cost::Paid),
                (42, Cost::Paid),
                (40, Cost::Paid),
                (41, Cost::Paid),
                (4, Cost::Paid),
                (9, Cost::Paid),
                (3, Cost::Paid),
                (6, Cost::Paid),
                (7, Cost::Paid),
                (8, Cost::Paid),
            ],
            "merges, decisions, repairs, reviews, splits, arbitration, building, preparing, debt"
        );
        assert_eq!(
            jobs.iter().filter_map(|j| j.exclusive).collect::<Vec<_>>(),
            ["tech-refinement", "test-debt"]
        );
        assert!(
            schedule(&empty(), &[], &|_| 0).is_empty(),
            "schedule(&empty(), &[], &|_| 0) should be empty"
        );
        assert_eq!(decide(&empty()), Route::Nothing);
    }

    #[test]
    fn the_critical_path_s_work_passes_the_ahead_work_of_its_kind() {
        let graph = graph();
        let ready = |n: u64| issue(n, &[labels::MILESTONE, labels::READY]);
        // The path is 18 -> 20 -> 19: a review of #19's passes #21's, #18's
        // passes both, each ten less down the path.
        let snapshot = Snapshot {
            milestones: vec![ready(18), ready(19), ready(20), ready(21), ready(22)],
            graph: Some(graph),
            prs_to_review: vec![
                pr("60", "milestone/21-sound"),
                pr("61", "milestone/19-media"),
                pr("62", "milestone/18-campaigns"),
            ],
            ..empty()
        };
        let jobs: Vec<(u64, u32)> = schedule(&snapshot, &[], &|_| 0)
            .into_iter()
            .filter(|j| matches!(j.route, Route::PrReview { .. }))
            .map(|j| (j.key, j.weight))
            .collect();
        assert_eq!(jobs, [(62, 880), (61, 860), (60, 780)]);
    }

    #[test]
    fn a_job_that_waits_gains_a_point_a_tick_up_to_the_cap() {
        let snapshot = Snapshot {
            test_debt: vec![issue(8, &[labels::TEST_DEBT])],
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestones: vec![3],
            ..empty()
        };
        let mut pulse = Pulse::default();
        assert_eq!(routes(&snapshot)[0], Route::DevLoop { milestone: 3 });
        for _ in 0..120 {
            pulse.age(&[Route::Refinement { issue: 6 }, Route::TestDebt { issue: 8 }]);
        }
        assert_eq!(pulse.waited(&Route::Refinement { issue: 6 }), 120);
        let aged = schedule(&snapshot, &[], &|route| pulse.waited(route));
        assert_eq!(
            aged.iter().map(|j| j.route.clone()).collect::<Vec<_>>(),
            [
                Route::Refinement { issue: 6 },
                Route::DevLoop { milestone: 3 },
                Route::TestDebt { issue: 8 },
            ],
            "a refinement waited long enough passes the build; a debt never passes it"
        );
        assert_eq!(aged[0].weight, 500 + 120);
        assert_eq!(aged[2].weight, 200 + 120);
        for _ in 0..100 {
            pulse.age(&[Route::TestDebt { issue: 8 }]);
        }
        assert_eq!(
            pulse.waited(&Route::Refinement { issue: 6 }),
            0,
            "it no longer waits"
        );
        let capped = schedule(&snapshot, &[], &|route| pulse.waited(route));
        assert_eq!(capped.last().map(|j| j.weight), Some(200 + AGE_CAP));
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
        // ForgeDesMondes: the planner chained #21 behind #18, #19 and #20
        // on GitHub; the graph supersedes that chaining. A person's task
        // among the blockers still holds.
        let chained = Issue {
            blocked_by: vec![building(18), ready(19), ready(20)],
            ..ready(21)
        };
        assert!(splittable(&chained, &milestones, Some(&graph)));
        let held = Issue {
            blocked_by: vec![issue(3, &[labels::HUMAN])],
            ..ready(21)
        };
        assert!(
            !splittable(&held, &milestones, Some(&graph)),
            "a person's task holds"
        );
        assert!(
            !splittable(&chained, &milestones, None),
            "without the graph the chaining is the rule"
        );
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
        assert!(
            critical_path(&milestones, None).is_empty(),
            "critical_path(&milestones, None) should be empty"
        );
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
        // Only the path's next step passes over the cap: #19 and #22 both
        // read #20, built; three build already — #19 (on the path) goes, #22
        // waits for a slot.
        let chain = vec![
            building(18),
            building(20),
            building(30),
            ready(19),
            ready(22),
        ];
        assert_eq!(splits(&chain, Some(&graph)), [19]);
        let landed = vec![
            issue(
                18,
                &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
            ),
            building(20),
            building(30),
            building(31),
            ready(19),
            ready(22),
        ];
        assert_eq!(
            splits(&landed, Some(&graph)),
            [19],
            "a landed path member is no next step"
        );
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
        assert_eq!(
            routes(&snapshot)
                .into_iter()
                .filter(|r| matches!(r, Route::DevLoop { .. }))
                .collect::<Vec<_>>(),
            [
                Route::DevLoop { milestone: 20 },
                Route::DevLoop { milestone: 18 }
            ]
        );
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

    #[test]
    fn a_closed_producer_weighs_nothing_and_the_other_chain_becomes_the_path() {
        let graph = parse_graph(Some(
            r#"{"version":1,"milestones":[
              {"number":30,"title":"a","systems":[],"produces":["a.1","a.2","a.3"],"consumes":[],"depends_on":[]},
              {"number":31,"title":"b","systems":[],"produces":["b.1"],"consumes":["a.1"],"depends_on":[30]},
              {"number":40,"title":"c","systems":[],"produces":["c.1"],"consumes":[],"depends_on":[]},
              {"number":41,"title":"d","systems":[],"produces":["d.1"],"consumes":["c.1"],"depends_on":[40]}
            ],"ids":{}}"#,
        ))
        .expect("a graph");
        let ready = |n: u64| issue(n, &[labels::MILESTONE, labels::READY]);
        let open = vec![ready(30), ready(31), ready(40), ready(41)];
        assert_eq!(
            critical_path(&open, Some(&graph)),
            [30, 31],
            "4 + 2 beats 2 + 2"
        );
        let closed = vec![
            Issue {
                state: "closed".to_string(),
                ..ready(30)
            },
            ready(31),
            ready(40),
            ready(41),
        ];
        assert_eq!(
            critical_path(&closed, Some(&graph)),
            [40, 41],
            "a closed producer weighs nothing: 0 + 2 loses to 2 + 2"
        );
    }

    #[test]
    fn the_boards_read_are_the_building_milestones_path_first_or_the_lowest_open_one() {
        let graph = graph();
        let ready = |n: u64| issue(n, &[labels::MILESTONE, labels::READY]);
        let building_one = |n: u64| issue(n, &[labels::MILESTONE, labels::TRIGGERED]);
        let milestones = vec![
            building_one(21),
            building_one(20),
            building_one(18),
            ready(19),
            issue(
                17,
                &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
            ),
            Issue {
                state: "closed".to_string(),
                ..building_one(16)
            },
        ];
        assert_eq!(building(&milestones, Some(&graph)), [18, 20, 21]);
        assert_eq!(
            building(&milestones, None),
            [18, 20, 21],
            "by number without a path"
        );
        let none_split = vec![ready(22), ready(19), issue(3, &[labels::HUMAN])];
        assert_eq!(
            building(&none_split, Some(&graph)),
            [19],
            "the lowest open milestone"
        );
        assert!(
            building(&[], Some(&graph)).is_empty(),
            "building(&[], Some(&graph)) should be empty"
        );
    }

    fn job(route: Route, key: u64, cost: Cost, exclusive: Option<&'static str>) -> Job {
        Job {
            weight: base_weight(&route),
            route,
            cost,
            key,
            exclusive,
        }
    }

    #[test]
    fn a_tick_starts_every_free_job_and_one_paid_job_per_free_lane() {
        let jobs = vec![
            job(Route::MergeMainAgent { milestone: 2 }, 2, Cost::Free, None),
            job(Route::Decide { issue: 50 }, 50, Cost::Paid, None),
            job(Route::Decide { issue: 51 }, 51, Cost::Paid, None),
            job(
                Route::PrReview {
                    pr: "40".to_string(),
                    base: String::new(),
                },
                40,
                Cost::Paid,
                None,
            ),
            job(
                Route::TechRefinement { issue: 7 },
                7,
                Cost::Paid,
                Some("tech-refinement"),
            ),
            job(
                Route::TechRefinement { issue: 8 },
                8,
                Cost::Paid,
                Some("tech-refinement"),
            ),
        ];
        let picked = plan(jobs.clone(), &[51], &[], 3, true);
        let keys = |jobs: &[Job]| jobs.iter().map(|j| j.key).collect::<Vec<_>>();
        assert_eq!(keys(&picked.free), [2]);
        assert_eq!(
            keys(&picked.paid),
            [50, 40, 7],
            "#51 is on a lane: not waiting"
        );
        assert_eq!(
            picked.left,
            [Route::TechRefinement { issue: 8 }],
            "one technical refinement at a time"
        );
        let resting = plan(jobs.clone(), &[], &[], 3, false);
        assert_eq!(keys(&resting.free), [2], "a quota rest keeps the free work");
        assert!(resting.paid.is_empty(), "resting.paid should be empty");
        assert_eq!(resting.left.len(), 5);
        let held = plan(jobs.clone(), &[], &["tech-refinement"], 9, true);
        assert_eq!(keys(&held.paid), [50, 51, 40], "a lane holds the group");
        let one_lane = plan(jobs, &[], &[], 1, true);
        assert_eq!(keys(&one_lane.paid), [50]);
        assert_eq!(one_lane.left.len(), 4);
        let twice = plan(
            vec![
                job(
                    Route::PrFix {
                        pr: "42".to_string(),
                    },
                    42,
                    Cost::Paid,
                    None,
                ),
                job(
                    Route::PrReview {
                        pr: "42".to_string(),
                        base: String::new(),
                    },
                    42,
                    Cost::Paid,
                    None,
                ),
            ],
            &[],
            &[],
            5,
            true,
        );
        assert_eq!(
            keys(&twice.paid),
            [42],
            "one job per PR: the repair, not also the review"
        );
        assert!(twice.left.is_empty(), "twice.left should be empty");
    }

    /// A task of a board, with its unit and what it needs.
    fn task_needing(number: u64, unit: &str, needs: &str) -> Issue {
        Issue {
            body: format!(
                "## Scope\n\nx\n\n## Architecture\n\nunit: {unit}\nsystem: media\nneeds: {needs}"
            ),
            ..issue(number, &[labels::AGENT, labels::READY])
        }
    }

    fn board(milestone: u64, tasks: Vec<Issue>) -> crate::dev_loop::data::board::Board {
        crate::dev_loop::data::board::Board {
            milestone: issue(milestone, &[labels::MILESTONE, labels::TRIGGERED]),
            tasks,
            resuming: None,
        }
    }

    #[test]
    fn a_board_s_tasks_are_jobs_and_ahead_of_its_producers_only_what_runs_on_the_mocks() {
        // #19 reads #20: its Micro-UI runs on #20's mocked contract, its
        // Capability on #20's table waits until #20 lands.
        let graph = parse_graph(Some(
            r#"{"version":1,"milestones":[
              {"number":20,"title":"sessions","systems":["session"],"produces":["session.listScenes","Session"],"consumes":[],"depends_on":[]},
              {"number":19,"title":"media","systems":["media"],"produces":["media.listMedia"],"consumes":["session.listScenes","Session"],"depends_on":[20]}
            ],"ids":{
              "media.listMedia":{"kind":"capability","producer":19,"state":"planned","version":1},
              "session.listScenes":{"kind":"capability","producer":20,"state":"planned","version":1},
              "Session":{"kind":"aggregate","producer":20,"state":"planned","version":1}
            }}"#,
        ));
        let tasks = vec![
            task_needing(60, "micro-ui", "session.listScenes"),
            task_needing(62, "capability", "Session"),
        ];
        let building = |state: &str| Snapshot {
            milestones: vec![
                issue(19, &[labels::MILESTONE, labels::TRIGGERED]),
                Issue {
                    state: state.to_string(),
                    ..issue(20, &[labels::MILESTONE, labels::TRIGGERED])
                },
            ],
            graph: graph.clone(),
            dev_loop_milestones: vec![19],
            boards: vec![board(19, tasks.clone())],
            ..empty()
        };
        let tasks_of = |snapshot: &Snapshot, running: &[u64]| {
            schedule(snapshot, running, &|_| 0)
                .into_iter()
                .filter_map(|job| match job.route {
                    Route::DevTask {
                        milestone: 19,
                        task,
                    } => Some(task),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            tasks_of(&building("open"), &[]),
            [60],
            "#20 builds: the mocks only"
        );
        assert_eq!(
            tasks_of(&building("closed"), &[]),
            [60, 62],
            "#20 landed: everything"
        );
        assert_eq!(
            tasks_of(&building("closed"), &[60]),
            [62],
            "one on a lane is not offered"
        );
        let other = Snapshot {
            boards: vec![board(21, tasks.clone())],
            ..building("closed")
        };
        assert_eq!(
            routes(&other),
            [Route::DevLoop { milestone: 19 }],
            "no board read for #19: its loop picks its task"
        );
    }

    /// The four tasks of the live failure, each on its declared branch.
    fn cycle_tasks() -> Vec<Issue> {
        let declared = |number: u64, branch: &str| Issue {
            body: format!("## Scope\n\nbranch: {branch}\n"),
            ..issue(number, &[labels::AGENT, labels::READY])
        };
        vec![
            declared(246, "feat/session-aggregate"),
            declared(248, "feat/battlemap-instance-aggregate"),
            declared(250, "feat/diffusion-aggregate"),
            declared(251, "feat/free"),
        ]
    }

    /// A PR into #18's branch.
    fn cycle_pr(num: &str, head: &str, title: &str, labels: &[&str], state: &str) -> Pr {
        Pr {
            num: num.to_string(),
            base: "milestone/18-campaigns".to_string(),
            head: head.to_string(),
            title: title.to_string(),
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Pr::default()
        }
    }

    /// #18 building, with these PRs in the cycle and these tasks green.
    fn cycle_plant(
        tasks: Vec<Issue>,
        task_prs: Vec<Pr>,
        tasks_to_merge: Vec<(u64, Pr)>,
    ) -> Snapshot {
        Snapshot {
            dev_loop_milestones: vec![18],
            boards: vec![board(18, tasks)],
            task_prs,
            tasks_to_merge,
            ..empty()
        }
    }

    /// The tasks the schedule offers a lane.
    fn dev_tasks_of(snapshot: &Snapshot) -> Vec<u64> {
        schedule(snapshot, &[], &|_| 0)
            .into_iter()
            .filter_map(|job| match job.route {
                Route::DevTask { task, .. } => Some(task),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_task_whose_pr_is_in_its_own_cycle_is_never_scheduled_again() {
        // The live failure: #248's PR #264 under review on a lane, #246's
        // PR #262 a tick from its merge — and both tasks offered to a lane.
        let under_review = cycle_pr(
            "264",
            "feat/battlemap-instance-aggregate",
            "BattlemapInstance aggregate",
            &[labels::TO_REVIEW],
            "OPEN",
        );
        let under_repair = cycle_pr(
            "260",
            "fix/elsewhere",
            "Diffusion aggregate (#250)",
            &[labels::PR_FIX],
            "OPEN",
        );
        let green = cycle_pr(
            "262",
            "feat/session-aggregate",
            "Session aggregate",
            &[labels::TO_REVIEW],
            "OPEN",
        );
        let held = cycle_plant(
            cycle_tasks(),
            vec![under_review, under_repair],
            vec![(246, green)],
        );
        assert_eq!(
            dev_tasks_of(&held),
            [251],
            "under review by its branch, under repair by its title, a tick from its merge"
        );
        let tasks = cycle_tasks();
        for number in [246, 248, 250] {
            let task = tasks.iter().find(|t| t.number == number).expect("the task");
            assert!(pr_cycle_owns(&held, task), "#{number}");
        }
        assert!(!pr_cycle_owns(&held, &tasks[3]));
        // Nothing in the cycle: everything runs.
        assert_eq!(
            dev_tasks_of(&cycle_plant(cycle_tasks(), Vec::new(), Vec::new())),
            [246, 248, 250, 251]
        );
    }

    #[test]
    fn a_pr_merged_or_closed_frees_its_task_s_branch() {
        let its_pr = |state: &str| {
            cycle_pr(
                "264",
                "feat/battlemap-instance-aggregate",
                "x",
                &[labels::TO_REVIEW],
                state,
            )
        };
        // Merged: the PR is no longer open, and the train closed the task —
        // nothing of it is offered either way.
        let mut closed = cycle_tasks();
        closed[1].state = "closed".to_string();
        let merged = cycle_plant(closed, vec![its_pr("MERGED")], Vec::new());
        assert_eq!(dev_tasks_of(&merged), [246, 250, 251]);
        // Closed unmerged: the branch is nobody's, the task runs again.
        let abandoned = cycle_plant(cycle_tasks(), vec![its_pr("CLOSED")], Vec::new());
        assert_eq!(dev_tasks_of(&abandoned), [246, 248, 250, 251]);
    }

    #[test]
    fn a_decision_on_a_task_of_the_critical_path_passes_one_off_it_and_the_plan_is_alone() {
        let graph = graph();
        let ready = |n: u64| issue(n, &[labels::MILESTONE, labels::READY]);
        let snapshot = Snapshot {
            milestones: vec![ready(18), ready(19), ready(20), ready(21)],
            graph: Some(graph),
            boards: vec![
                board(18, vec![issue(51, &[labels::AGENT])]),
                board(21, vec![issue(70, &[labels::AGENT])]),
            ],
            deciding: vec![
                issue(70, &[labels::NEEDS_DECISION]),
                issue(51, &[labels::NEEDS_DECISION]),
            ],
            roadmap: vec![issue(4, &[labels::ROADMAP, labels::READY])],
            ..empty()
        };
        let jobs = schedule(&snapshot, &[], &|_| 0);
        let decisions: Vec<(u64, u32)> = jobs
            .iter()
            .filter(|j| matches!(j.route, Route::Decide { .. }))
            .map(|j| (j.key, j.weight))
            .collect();
        assert_eq!(
            decisions,
            [(51, 960), (70, 860)],
            "#51 is #18's: the path's first"
        );
        let planner = jobs
            .iter()
            .find(|j| matches!(j.route, Route::Planner { .. }))
            .expect("the plan");
        assert_eq!(planner.exclusive, Some("planner"));
        assert_eq!(planner.key, 4);
    }

    /// A PR from `head` into `base`.
    fn pr_from(num: &str, head: &str, base: &str) -> Pr {
        Pr {
            head: head.to_string(),
            ..pr(num, base)
        }
    }

    #[test]
    fn a_milestone_behind_the_base_is_caught_up_before_any_merge_and_free() {
        let snapshot = Snapshot {
            catch_up: vec![(4, "milestone/4-x".to_string())],
            tasks_to_merge: vec![(31, pr("34", "milestone/4-x"))],
            ready_to_merge: vec![4],
            ..empty()
        };
        let jobs = schedule(&snapshot, &[], &|_| 0);
        assert_eq!(
            jobs[0].route,
            Route::CatchUp {
                milestone: 4,
                branch: "milestone/4-x".to_string()
            }
        );
        assert_eq!(
            (jobs[0].cost, jobs[0].key, jobs[0].weight),
            (Cost::Free, 4, 1010)
        );
        assert!(base_weight(&jobs[0].route) > base_weight(&jobs[1].route));
        // The catch-up and the landing share the milestone: one a tick, the
        // catch-up first; the task's merge goes on beside it.
        let picked = plan(jobs, &[], &[], 0, true);
        let free: Vec<&Route> = picked.free.iter().map(|job| &job.route).collect();
        assert_eq!(free.len(), 2, "{free:?}");
        assert!(matches!(
            free[1],
            Route::MergeIntoMilestone { task: 31, .. }
        ));
        assert_eq!(
            decide(&snapshot),
            Route::CatchUp {
                milestone: 4,
                branch: "milestone/4-x".to_string()
            }
        );
    }

    #[test]
    fn nothing_is_caught_up_while_the_base_is_red() {
        let snapshot = Snapshot {
            catch_up: vec![(4, "milestone/4-x".to_string())],
            base_checks: Some(false),
            ..empty()
        };
        assert!(routes(&snapshot).is_empty(), "nothing expected");
    }

    #[test]
    fn a_repair_of_a_landing_pr_waits_for_its_catch_up() {
        let landing = pr_from("90", "milestone/4-x", "main_agent");
        let task_pr = pr_from("91", "task/91", "milestone/4-x");
        let snapshot = Snapshot {
            catch_up: vec![(4, "milestone/4-x".to_string())],
            prs_to_fix: vec![landing.clone(), task_pr.clone()],
            task_prs: vec![landing.clone(), task_pr],
            ..empty()
        };
        let found = routes(&snapshot);
        assert!(
            found.contains(&Route::CatchUp {
                milestone: 4,
                branch: "milestone/4-x".to_string()
            }),
            "{found:?}"
        );
        assert!(
            !found.contains(&Route::PrFix {
                pr: "90".to_string()
            }),
            "the landing PR's repair waits for the catch-up: {found:?}"
        );
        assert!(
            found.contains(&Route::PrFix {
                pr: "91".to_string()
            }),
            "a task PR's repair pushes elsewhere: {found:?}"
        );
        // No catch-up due: the landing PR's repair goes on.
        let calm = Snapshot {
            catch_up: Vec::new(),
            prs_to_fix: vec![landing],
            ..empty()
        };
        assert_eq!(
            routes(&calm),
            [Route::PrFix {
                pr: "90".to_string()
            }]
        );
    }

    #[test]
    fn a_branch_behind_the_base_or_unable_to_take_it_holds_its_tasks() {
        let plant = |catch_up: Vec<(u64, String)>, held: Vec<u64>| Snapshot {
            milestones: vec![issue(19, &[labels::MILESTONE, labels::TRIGGERED])],
            dev_loop_milestones: vec![19],
            boards: vec![board(19, vec![task_needing(60, "micro-ui", "x.y")])],
            catch_up,
            catch_up_held: held,
            ..empty()
        };
        let task = Route::DevTask {
            milestone: 19,
            task: 60,
        };
        assert!(routes(&plant(Vec::new(), Vec::new())).contains(&task));
        assert!(
            routes(&plant(Vec::new(), vec![7])).contains(&task),
            "another's"
        );
        let behind = routes(&plant(vec![(19, "milestone/19-x".to_string())], Vec::new()));
        assert!(!behind.contains(&task), "{behind:?}");
        assert!(!routes(&plant(Vec::new(), vec![19])).contains(&task));
        let no_board = Snapshot {
            boards: Vec::new(),
            ..plant(Vec::new(), vec![19])
        };
        assert!(
            !routes(&no_board).contains(&Route::DevLoop { milestone: 19 }),
            "the dev loop of a held milestone waits too"
        );
    }

    #[test]
    fn the_catch_ups_due_are_read_from_the_pulse_once_the_head_is_green() {
        let milestones = vec![
            Issue {
                title: "Media".to_string(),
                ..issue(6, &[labels::MILESTONE, labels::TRIGGERED])
            },
            Issue {
                title: "Sound".to_string(),
                ..issue(7, &[labels::MILESTONE, labels::TRIGGERED])
            },
        ];
        let mut pulse = Pulse::default();
        assert!(pulse.catch_ups.moved(Some("h1".to_string())));
        pulse.catch_ups.seen_at = Some(100);
        pulse.catch_ups.record(
            7,
            &crate::common::branching::CaughtUp::Failed("x".to_string()),
            100,
        );
        // The integration branch's newest run is green, but it is h1's own
        // verdict that counts, and none is in yet.
        let mut running = Snapshot {
            milestones: milestones.clone(),
            base_checks: Some(true),
            ..empty()
        };
        let (said, log) = told();
        read_catch_ups(&mut running, &mut pulse, 101, &log);
        assert!(running.catch_up.is_empty(), "never before its own green");
        assert_eq!(
            running.catch_up_held,
            [6, 7],
            "#6 lags the pending head, #7 is parked"
        );
        assert!(pulse.catch_ups.behind_seen, "its checks are read next tick");
        pulse.catch_ups.local_left = 0;
        pulse.catch_ups.verdict = Some(true);
        let mut green = Snapshot {
            milestones: milestones.clone(),
            ..empty()
        };
        read_catch_ups(&mut green, &mut pulse, 101, &log);
        assert_eq!(green.catch_up, [(6, "milestone/6-media".to_string())]);
        assert_eq!(pulse.catch_ups.local_left, 1, "a new tick's local catch-up");
        // h1 green, then h2 read: green for an older sha, nothing due.
        assert!(pulse.catch_ups.moved(Some("h2".to_string())));
        let mut moved = Snapshot {
            milestones,
            base_checks: Some(true),
            ..empty()
        };
        read_catch_ups(&mut moved, &mut pulse, 101, &log);
        assert!(moved.catch_up.is_empty(), "{:?}", moved.catch_up);
        assert!(said.0.borrow().0.is_empty(), "{:?}", said.0.borrow().0);
    }

    #[test]
    fn the_219_case_a_task_of_a_branch_behind_a_pending_head_waits() {
        // 2026-10-24, 22:06 Paris: #24 landed on main_agent; 22:07, its
        // checks still running, #219 started on milestone/28, which did not
        // hold #24 yet, and stopped on what #24 brought ("missing", $0.16).
        let mut pulse = Pulse::default();
        assert!(pulse.catch_ups.moved(Some("a3823e4".to_string())));
        pulse.catch_ups.seen_at = Some(1000);
        pulse
            .catch_ups
            .held
            .insert(28, "7dc21d3-before".to_string());
        let mut found = Snapshot {
            milestones: vec![issue(28, &[labels::MILESTONE, labels::TRIGGERED])],
            dev_loop_milestones: vec![28],
            boards: vec![board(28, vec![task_needing(219, "micro-ui", "x.y")])],
            base_checks: Some(true),
            ..empty()
        };
        let (_, log) = told();
        read_catch_ups(&mut found, &mut pulse, 1060, &log);
        assert!(found.catch_up.is_empty(), "the head is not green yet");
        assert_eq!(found.catch_up_held, [28]);
        let waiting = routes(&found);
        assert!(
            !waiting.iter().any(|route| matches!(
                route,
                Route::DevTask { milestone: 28, .. } | Route::DevLoop { milestone: 28 }
            )),
            "{waiting:?}"
        );
        // Green and caught up: the task starts on a base that holds #24.
        pulse.catch_ups.verdict = Some(true);
        pulse
            .catch_ups
            .record(28, &crate::common::branching::CaughtUp::OnGitHub, 1500);
        read_catch_ups(&mut found, &mut pulse, 1500, &log);
        assert!(found.catch_up_held.is_empty(), "#28 holds the head");
        assert!(routes(&found).contains(&Route::DevTask {
            milestone: 28,
            task: 219
        }));
    }

    #[test]
    fn an_expired_lag_hold_is_said_once_per_head() {
        let mut pulse = Pulse::default();
        assert!(pulse.catch_ups.moved(Some("a55bd4d0c0ffee".to_string())));
        pulse.catch_ups.seen_at = Some(0);
        let mut found = Snapshot {
            milestones: vec![
                issue(28, &[labels::MILESTONE, labels::TRIGGERED]),
                issue(24, &[labels::MILESTONE, labels::TRIGGERED]),
            ],
            ..empty()
        };
        let (said, log) = told();
        let over = crate::common::catch_up::LAG_HOLD_SECONDS;
        read_catch_ups(&mut found, &mut pulse, over - 1, &log);
        assert_eq!(found.catch_up_held, [24, 28]);
        assert!(said.0.borrow().0.is_empty(), "held, nothing said");
        read_catch_ups(&mut found, &mut pulse, over, &log);
        assert!(found.catch_up_held.is_empty(), "let go");
        read_catch_ups(&mut found, &mut pulse, over + 60, &log);
        assert_eq!(
            said.0.borrow().0,
            [
                "warning: watch: the integration branch's head a55bd4d has no verdict 20 min after \
                 its commit — the tasks of milestone(s) #24, #28 start on their branch as it is"
            ],
            "once"
        );
    }
}
