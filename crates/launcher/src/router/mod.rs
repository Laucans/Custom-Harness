//! `harness watch`: polls on an interval, decides what to run next
//! (`common::routing`), dispatches, and loops.
//!
//! The only place that builds the `GitHub` adapter used purely for the
//! routing decision — a lightweight read, no checkout mounted for it.
//! Dispatching to a workflow is each workflow's own launcher module's job
//! (`dev_loop.rs`, `planner.rs`, `split.rs`, `refinement.rs`,
//! `pr_review.rs`, `pr_fix.rs`, `arbiter.rs`, `test_debt.rs`) — each mounts what it needs
//! itself, and only
//! when actually dispatched to. A tick that routes to [`Route::Nothing`]
//! therefore never touches disk.
//!
//! The PR-facing reads cost one extra API call per labelled candidate (the
//! failing checks, or the comments) and stop at the first match. That is
//! deliberately paid here rather than inside the workflows: a PR that would
//! skip must not cost a mounted checkout every thirty seconds.
//!
//! **A quiet tick costs no quota.** Every tick starts with one conditional
//! read of the board's activity (`GitHub::activity`): a `304` is free, and
//! when nothing moved — and no pull request is waiting on its checks, and no
//! lane just freed — the snapshot is not read at all (`routing::may_skip`).
//! The forty-odd reads a snapshot costs are paid only when something changed,
//! or every `routing::FULL_READ_EVERY` ticks as a valve.
//!
//! **A failed routing read stops the loop** — a persistent problem (bad
//! credentials, an unreadable repo) should not retry silently forever.
//! **A failed dispatch does not** — one workflow's own failure (a session
//! hiccup) is that workflow's business, logged here, and likely to succeed
//! on a later tick. **A `QUOTA` stop makes the watch rest**
//! (`routing::Rest`): nothing is read or dispatched for a few ticks, twice
//! as many after each stop in a row, since an exhausted window does not
//! come back in thirty seconds and every try left a `QUOTA` row and a
//! doctor's visit behind it.
//!
//! **`SIGTERM` is a soft stop.** The tick under way finishes, no new task
//! starts, the running lanes are waited for, then the watch exits. A hard
//! stop is the caller's `SIGKILL` to the watch's process group.
//!
//! Three files: this one is the loop — a tick, its reads of the board, the
//! rest after a `QUOTA` stop; `routes` runs what a tick decided in this
//! process; `lanes` fills the lanes of a parallel watch with children.

mod lanes;
mod routes;

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use harness_adapters::shell::github::GhCli;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Issue, Outcome, Pr, Slug};
use harness_core::ports::shell::github::{Activity, GitHub};
use harness_core::traces::{Event, Logbook, Sink, Verbosity};
use harness_workflows::common::routing::{self, Pulse, Route, Snapshot};
use harness_workflows::common::{branching, labels, review};
use harness_workflows::dev_loop::data::{board, tasks};
use harness_workflows::main_agent_merge::data::audit;
use harness_workflows::pr_review::data::skip_rules;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::cli::WatchArgs;
use crate::dispatch::data_snapshot;
use crate::dispatch::lanes::Lanes;

use routes::dispatch;

/// Runs the watch loop: one tick now, then every `--interval` seconds,
/// forever — or exactly once under `--once`.
///
/// # Errors
///
/// Only under `--once`, where a single pass's outcome is what the caller
/// asked for. A watch that keeps polling **swallows a failed tick**: it is
/// recorded in the error ledger and the next tick is taken. A dispatch
/// failure was already swallowed — see the module doc.
pub async fn run(args: &WatchArgs, here: &Path) -> Outcome<()> {
    let journal = Workspace::new(here).loop_dir().join("watch.log");
    let sink = Rc::new(Both::new(&journal).map_err(|e| {
        Halt::Failed(format!(
            "the watch journal {} cannot be opened: {e}",
            journal.display()
        ))
    })?);
    // Appended across restarts on purpose: the question this answers — what
    // has the loop been doing — does not stop at a restart.
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);
    log.event(&Event::WatchStarted {
        interval: args.interval,
        target: if args.target_repo_url.is_empty() {
            "this checkout's own origin".to_string()
        } else {
            args.target_repo_url.clone()
        },
        journal: Workspace::new(here).rel(&journal),
    });
    // The changes the human left in the view reach the runs here, and only
    // here: a start is the one moment no run is reading the book.
    match crate::dispatch::shared::apply_drafts(here, crate::dispatch::dev_loop::now_seconds()) {
        Ok(prompts) if prompts.is_empty() => {}
        Ok(prompts) => log.event(&Event::ChangesApplied { prompts }),
        Err(halt) => log.event(&Event::TickFailed {
            reason: format!("changes left waiting: {}", halt.reason()),
        }),
    }
    let draining = Arc::new(AtomicBool::new(false));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| Halt::Failed(format!("SIGTERM cannot be listened to: {e}")))?;
    let flag = Arc::clone(&draining);
    let mut listener = tokio::spawn(async move {
        term.recv().await;
        flag.store(true, Ordering::SeqCst);
    });
    let mut lanes = Lanes::new(args.parallel).closed_by(draining);
    let mut pulse = Pulse::default();
    loop {
        let went = tick(args, here, &log, &mut lanes, &mut pulse).await;
        if args.once {
            // One pass was asked for, and its exit code is the answer — once
            // the lanes it may have opened are done, so the pass is whole.
            lanes.wait_all(&log).await;
            listener.abort();
            return went;
        }
        if let Err(halt) = went {
            // A read that failed is **not** the end of the loop. A single
            // `connection reset by peer` on one GitHub call used to kill an
            // unattended watch outright, and the milestone it was about to
            // merge waited for a human to notice.
            log.event(&Event::TickFailed {
                reason: halt.reason().to_string(),
            });
            crate::dispatch::doctor::record(here, &spending::run_id(), "watch", &halt);
        }
        // A finished listener is never polled again: the check below returns.
        if !lanes.is_closed() {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(args.interval)) => {}
                _ = &mut listener => {}
            }
        }
        if lanes.is_closed() {
            listener.abort();
            return drain(&mut lanes, &log).await;
        }
    }
}

/// The soft stop: says what it waits for, waits for every lane, says so.
async fn drain(lanes: &mut Lanes, log: &Logbook) -> Outcome<()> {
    log.event(&Event::WatchDraining {
        lanes: lanes.running(),
    });
    lanes.wait_all(log).await;
    log.event(&Event::WatchStopped);
    Ok(())
}

/// One pass: read the snapshot, decide, dispatch.
async fn tick(
    args: &WatchArgs,
    here: &Path,
    log: &Logbook,
    lanes: &mut Lanes,
    pulse: &mut Pulse,
) -> Outcome<()> {
    let gh = resolve_gh(args, here)?;
    let freed = lanes.reap(log);
    if lanes.struck_by_quota() {
        rest(pulse, log);
    }
    if !pulse.rest.passes() {
        // Nothing is read either: a resting watch costs neither quota nor a
        // board read, and the lanes already running keep running.
        log.debug(&format!(
            "resting: quota exhausted — {} tick(s) left",
            pulse.rest.ticks_left
        ));
        return Ok(());
    }
    let activity = gh.activity(pulse.etag.as_deref()).await?;
    if routing::may_skip(pulse, &activity, freed) {
        pulse.stayed_quiet();
        log.debug("quiet: nothing moved on the board — no snapshot read");
        return Ok(());
    }
    let etag = match &activity {
        Activity::Changed { etag, moved_at } => {
            if let Some(at) = moved_at {
                log.debug(&format!("board moved at {at}"));
            }
            etag.clone()
        }
        Activity::Unchanged => pulse.etag.clone(),
    };
    let (found, idle, watching_ci) = snapshot(gh.as_ref()).await?;
    pulse.read_in_full(etag, watching_ci);
    // The board moved: a merge may have moved a branch, whose data model is
    // archived before anything else is dispatched.
    snapshot_data(args, here, &gh, &found.milestones, log).await;
    // What the decision was made from, for a tick whose choice later looks
    // wrong: the counts are the whole input to `routing::decide`.
    log.debug(&format!(
        "saw: {} roadmap, {} milestone(s), {} refining, {} tech-refining, {} arbitrating, \
         {} test-debt, pr_fix={:?}, pr_review={:?}, ready_to_merge={:?}, dev_loop={:?}",
        found.roadmap.len(),
        found.milestones.len(),
        found.refining.len(),
        found.tech_refining.len(),
        found.arbitrating.len(),
        found.test_debt.len(),
        found.pr_to_fix.as_ref().map(|pr| pr.num.clone()),
        found.pr_to_review.as_ref().map(|pr| pr.num.clone()),
        found.ready_to_merge,
        found.dev_loop_milestone,
    ));
    let route = routing::decide(&found);
    if matches!(route, Route::Nothing)
        && let Some(reason) = idle
    {
        // An idle tick names the gesture it waits for, since "nothing" alone
        // reads as "done" to whoever watches the journal.
        log.say(&format!("idle: {reason}"));
    }
    if lanes.is_closed() {
        // A soft stop arrived during the read: nothing new is dispatched.
        return Ok(());
    }
    match dispatch(args, here, gh.as_ref(), &found, route, log, lanes).await {
        Dispatched::Quota => rest(pulse, log),
        Dispatched::Went => pulse.rest.cleared(),
        Dispatched::Nothing | Dispatched::Failed => {}
    }
    Ok(())
}

/// A `QUOTA` stop: the watch rests, longer with every stop in a row, and
/// says so once.
fn rest(pulse: &mut Pulse, log: &Logbook) {
    let ticks = pulse.rest.struck();
    log.event(&Event::QuotaRest {
        ticks,
        strikes: pulse.rest.strikes,
    });
}

/// What a tick's dispatch came to, as far as the rest is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dispatched {
    /// No route, nothing ran.
    Nothing,
    /// A route ran and did not fail.
    Went,
    /// A route failed, and not on the quota.
    Failed,
    /// A route stopped on `QUOTA`.
    Quota,
}

/// Archives the data model of every branch whose head moved: `main`, the
/// integration branch, each open milestone's. Never fails the tick — a
/// branch that could not be archived is an event, and is tried again when
/// the board next moves. A target named by this checkout's own `origin`
/// has no slug to clone: skipped.
async fn snapshot_data(
    args: &WatchArgs,
    here: &Path,
    gh: &Rc<dyn GitHub>,
    milestones: &[Issue],
    log: &Logbook,
) {
    // A dry run writes nothing, and a snapshot pushes the archive.
    if args.dry_run {
        return;
    }
    let Some(slug) = Slug::parse(&args.target_repo_url) else {
        return;
    };
    let mut branches = vec![
        data_snapshot::RELEASE_BRANCH.to_string(),
        args.branch.clone(),
    ];
    branches.extend(
        milestones
            .iter()
            .map(|m| branching::milestone_branch(m.number, &m.title)),
    );
    branches.dedup();
    let pass = data_snapshot::Pass {
        here,
        slug: &slug,
        integration: &args.branch,
        permission_mode: &args.permission_mode,
    };
    data_snapshot::snapshot_and_check(&pass, Rc::clone(gh), branches, log).await;
}

/// The `GitHub` used for the routing decision alone — no checkout: a named
/// target resolves through `gh api` by slug; an unnamed one falls back to
/// this launch directory's own `origin`, like `dev_loop`'s `--no-workspace`.
fn resolve_gh(args: &WatchArgs, here: &Path) -> Outcome<Rc<dyn GitHub>> {
    if args.target_repo_url.is_empty() {
        return Ok(Rc::new(GhCli::new(here)));
    }
    let slug = Slug::parse(&args.target_repo_url).ok_or_else(|| {
        Halt::Failed(format!(
            "{:?} is not a usable repository URL",
            args.target_repo_url
        ))
    })?;
    Ok(Rc::new(GhCli::for_slug(&slug)))
}

/// Reads everything [`routing::decide`] needs, and the one line that says
/// why the current milestone offers no task when it does not.
/// The snapshot, the idle reason if the board has nothing runnable, and
/// whether a pull request the board waits on is open — what tells the next
/// tick it cannot trust a quiet activity read.
async fn snapshot(gh: &dyn GitHub) -> Outcome<(Snapshot, Option<String>, bool)> {
    let roadmap = gh.issues_labelled(labels::ROADMAP, "open").await?;
    let lowest_roadmap_has_milestone = match roadmap
        .iter()
        .filter(|issue| issue.is_open())
        .min_by_key(|issue| issue.number)
    {
        Some(lowest) => !gh.sub_issues(lowest.number).await?.is_empty(),
        None => false,
    };
    let milestones =
        with_split_blockers(gh, gh.issues_labelled(labels::MILESTONE, "open").await?).await?;
    // The two labelled lists once, for the three reads that need them.
    let to_fix = by_number(gh.open_prs_labelled(labels::PR_FIX).await?);
    let to_review = by_number(gh.open_prs_labelled(labels::TO_REVIEW).await?);
    let watching_ci = !to_fix.is_empty() || !to_review.is_empty();
    let pr_to_fix = lowest_red_pr(gh, &to_fix, &to_review).await?;
    let pr_to_review = lowest_pr_worth_reviewing(gh, &to_review).await?;
    let ready_to_merge = lowest_ready_to_merge(gh, &milestones).await?;
    let refining = gh.issues_labelled(labels::REFINEMENT, "open").await?;
    let tech_refining = gh.issues_labelled(labels::TECH_REFINEMENT, "open").await?;
    let arbitrating = gh
        .issues_labelled(labels::NEEDS_AI_DECISION, "open")
        .await?;
    let test_debt = gh.issues_labelled(labels::TEST_DEBT, "open").await?;
    // Tolerant on purpose: "no open milestone at all" is this probe's most
    // common answer, not a failure — `dev_loop::run` itself is where that
    // distinction matters and is already made.
    let board = board::read(gh).await.ok();
    let dev_loop_milestone = board
        .as_ref()
        .filter(|b| b.next().is_some())
        .map(|b| b.milestone.number);
    let idle = board
        .as_ref()
        .filter(|b| b.next().is_none())
        .map(|b| tasks::idle_reason(b.milestone.number, &b.tasks));
    let task_to_merge = match &board {
        Some(board) => green_task_pr(gh, &board.tasks, &to_review).await?,
        None => None,
    };
    let snapshot = Snapshot {
        task_to_merge,
        roadmap,
        lowest_roadmap_has_milestone,
        milestones,
        pr_to_fix,
        pr_to_review,
        ready_to_merge,
        refining,
        tech_refining,
        arbitrating,
        dev_loop_milestone,
        test_debt,
    };
    Ok((snapshot, idle, watching_ci))
}

/// The lowest-numbered open PR that carries `harness:pr-fix` **and** has a
/// check that concluded in failure — one CI read per candidate, in number
/// order, stopping at the first match.
///
/// The pairing is the point: the label alone would send a paid session at a
/// PR that is merely still building.
async fn lowest_red_pr(gh: &dyn GitHub, to_fix: &[Pr], to_review: &[Pr]) -> Outcome<Option<Pr>> {
    let mut found: Vec<Pr> = Vec::new();
    for candidate in to_fix {
        if !gh.pr_failing_checks(&candidate.num).await?.is_empty() {
            found.push(candidate.clone());
            break;
        }
    }
    // A blocking agent review, or a branch that no longer merges into its
    // base, asks for a repair as a red check does — the label is posed for
    // it at dispatch, the same request a human makes. Bounded like the
    // rest: past `MAX_FIXES` repairs, the PR is a human's.
    for candidate in to_review {
        let comments = gh.pr_comments(&candidate.num).await?;
        let blocked = review::status(&comments).wants_a_fix();
        let conflicting = review::repairs(&comments) < review::MAX_FIXES
            && gh.pr_mergeable(&candidate.num).await? == Some(false);
        if blocked || conflicting {
            found.push(candidate.clone());
            break;
        }
    }
    Ok(by_number(found).into_iter().next())
}

/// The lowest-numbered open PR that carries `harness:to-review` and that the
/// review itself would not skip — one comments read per candidate, in number
/// order, stopping at the first match.
///
/// The decision reuses `pr_review`'s own rules rather than restating them:
/// policy in two places is policy that drifts. Applying them here is only
/// an optimization — a PR that would skip (a draft, a test PR, one already
/// carrying a review) must not cost a mounted checkout on every poll, and
/// its label can be left in place harmlessly because of this filter.
async fn lowest_pr_worth_reviewing(gh: &dyn GitHub, to_review: &[Pr]) -> Outcome<Option<Pr>> {
    for candidate in to_review {
        let comments = gh.pr_comments(&candidate.num).await?;
        // `candidate.base` as the expected base: the label is the
        // authorization, and a task PR legitimately targets its milestone's
        // branch. What stays live are the other three rules.
        if skip_rules::skip_reason(false, &candidate.base, candidate, &comments).is_none() {
            return Ok(Some(candidate.clone()));
        }
    }
    Ok(None)
}

/// PRs in number order, the ones whose number is unreadable last — a PR ref
/// that is not a number cannot be ordered, and dropping it silently would
/// hide it instead.
fn by_number(mut prs: Vec<Pr>) -> Vec<Pr> {
    prs.sort_by_key(|pr| pr.num.parse::<u64>().unwrap_or(u64::MAX));
    prs
}

/// The lowest-numbered open milestone, not yet `harness:waiting-merge`,
/// whose own tasks are all closed — one extra read per candidate, in
/// number order, stopping at the first match.
async fn lowest_ready_to_merge(
    gh: &dyn harness_core::ports::shell::github::GitHub,
    milestones: &[harness_core::domain::Issue],
) -> Outcome<Option<u64>> {
    let mut candidates: Vec<&harness_core::domain::Issue> = milestones
        .iter()
        .filter(|issue| issue.is_open() && !issue.has(labels::WAITING_MERGE))
        .collect();
    candidates.sort_by_key(|issue| issue.number);
    for candidate in candidates {
        let tasks = gh.sub_issues(candidate.number).await?;
        if audit::all_tasks_delivered(&tasks) {
            return Ok(Some(candidate.number));
        }
    }
    Ok(None)
}

/// The task whose PR is open on its milestone, reviewed and green, with that
/// PR — what `milestone_merge` merges. A PR still owed its review is left to
/// the review route, which outranks this one.
async fn green_task_pr(
    gh: &dyn GitHub,
    tasks: &[Issue],
    to_review: &[Pr],
) -> Outcome<Option<(u64, Pr)>> {
    if !tasks
        .iter()
        .any(harness_workflows::dev_loop::data::tasks::review_pending)
    {
        return Ok(None);
    }
    let Some((task, pr)) = harness_workflows::milestone_merge::candidate(tasks, to_review) else {
        return Ok(None);
    };
    // Its last review must let it through: a blocking one is the repair
    // route's, and one its repairs did not clear is a human's.
    let comments = gh.pr_comments(&pr.num).await?;
    match review::status(&comments) {
        review::Status::Clean => {}
        status if status.exhausted() => {
            harness_workflows::milestone_merge::escalate(gh, task, pr).await?;
            return Ok(None);
        }
        _ => return Ok(None),
    }
    // And it must merge: a conflict with the base is the repair route's too,
    // until the repairs are used up.
    if gh.pr_mergeable(&pr.num).await? == Some(false) {
        if review::repairs(&comments) >= review::MAX_FIXES {
            harness_workflows::milestone_merge::escalate(gh, task, pr).await?;
        }
        return Ok(None);
    }
    if !gh.pr_checks_green(&pr.num).await? {
        return Ok(None);
    }
    Ok(Some((task.number, pr.clone())))
}

/// The milestones, with the blockers of every split candidate read — what
/// [`routing::splittable`] needs. Only the candidates: the others are not
/// split whatever blocks them, and each read is one `gh api` call per tick.
async fn with_split_blockers(gh: &dyn GitHub, milestones: Vec<Issue>) -> Outcome<Vec<Issue>> {
    let (candidates, others): (Vec<Issue>, Vec<Issue>) =
        milestones.into_iter().partition(routing::split_candidate);
    let mut all = gh.with_blockers(candidates).await?;
    all.extend(others);
    Ok(all)
}
