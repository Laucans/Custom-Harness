//! `harness watch`: polls on an interval, decides what to run next
//! (`common::routing`), dispatches, and loops.
//!
//! The only place that builds the `GitHub` adapter used purely for the
//! routing decision — a lightweight read, no checkout mounted for it.
//! Dispatching to a workflow is each workflow's own launcher module's job
//! (`dev_loop.rs`, `planner.rs`, `split.rs`, `refinement.rs`,
//! `pr_review.rs`, `pr_fix.rs`) — each mounts what it needs itself, and only
//! when actually dispatched to. A tick that routes to [`Route::Nothing`]
//! therefore never touches disk.
//!
//! The PR-facing reads cost one extra API call per labelled candidate (the
//! failing checks, or the comments) and stop at the first match. That is
//! deliberately paid here rather than inside the workflows: a PR that would
//! skip must not cost a mounted checkout every thirty seconds.
//!
//! **A failed routing read stops the loop** — a persistent problem (bad
//! credentials, an unreadable repo) should not retry silently forever.
//! **A failed dispatch does not** — one workflow's own failure (a quota
//! exhausted, a session hiccup) is that workflow's business, logged here,
//! and likely to succeed on a later tick.
//!
//! **`SIGTERM` is a soft stop.** The tick under way finishes, no new task
//! starts, the running lanes are waited for, then the watch exits. A hard
//! stop is the caller's `SIGKILL` to the watch's process group.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::Parser as _;
use harness_core::adapters::shell::github::GhCli;
use harness_core::domain::doctor::Repair;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Issue, Outcome, Pr, Slug, Verdict};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::common::routing::{self, Route, Snapshot};
use harness_workflows::common::{branching, hierarchy, labels, review};
use harness_workflows::dev_loop::data::{board, tasks};
use harness_workflows::main_agent_merge::data::audit;
use harness_workflows::main_agent_merge::data::report::Outcome as MergeReport;
use harness_workflows::pr_review::data::skip_rules;
use harness_workflows::refinement::data::phase::Phase;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::cli::WatchArgs;
use crate::dispatch::lanes::Lanes;

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
    log.say(&format!(
        "watch: every {}s on {} — journal {}",
        args.interval,
        if args.target_repo_url.is_empty() {
            "this checkout's own origin".to_string()
        } else {
            args.target_repo_url.clone()
        },
        Workspace::new(here).rel(&journal)
    ));
    let draining = Arc::new(AtomicBool::new(false));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| Halt::Failed(format!("SIGTERM cannot be listened to: {e}")))?;
    let flag = Arc::clone(&draining);
    let mut listener = tokio::spawn(async move {
        term.recv().await;
        flag.store(true, Ordering::SeqCst);
    });
    let mut lanes = Lanes::new(args.parallel).closed_by(draining);
    loop {
        let went = tick(args, here, &log, &mut lanes).await;
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
            log.warn(&format!("watch: tick -> {}", halt.reason()));
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
    let running = lanes.running();
    log.say(&format!(
        "watch: draining — soft stop asked, no new task; waiting for {} lane(s){}",
        running.len(),
        if running.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                running
                    .iter()
                    .map(|task| format!("#{task}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    ));
    lanes.wait_all(log).await;
    log.say("watch: stopped — soft stop, every lane done");
    Ok(())
}

/// One pass: read the snapshot, decide, dispatch.
async fn tick(args: &WatchArgs, here: &Path, log: &Logbook, lanes: &mut Lanes) -> Outcome<()> {
    let gh = resolve_gh(args, here)?;
    lanes.reap(log);
    let (found, idle) = snapshot(gh.as_ref()).await?;
    // What the decision was made from, for a tick whose choice later looks
    // wrong: the counts are the whole input to `routing::decide`.
    log.debug(&format!(
        "saw: {} roadmap, {} milestone(s), {} refining, {} tech-refining, \
         pr_fix={:?}, pr_review={:?}, ready_to_merge={:?}, dev_loop={:?}",
        found.roadmap.len(),
        found.milestones.len(),
        found.refining.len(),
        found.tech_refining.len(),
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
    dispatch(args, here, gh.as_ref(), route, log, lanes).await;
    Ok(())
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
async fn snapshot(gh: &dyn GitHub) -> Outcome<(Snapshot, Option<String>)> {
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
    let pr_to_fix = lowest_red_pr(gh).await?;
    let pr_to_review = lowest_pr_worth_reviewing(gh).await?;
    let ready_to_merge = lowest_ready_to_merge(gh, &milestones).await?;
    let refining = gh.issues_labelled(labels::REFINEMENT, "open").await?;
    let tech_refining = gh.issues_labelled(labels::TECH_REFINEMENT, "open").await?;
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
        Some(board) => green_task_pr(gh, &board.tasks).await?,
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
        dev_loop_milestone,
    };
    Ok((snapshot, idle))
}

/// The lowest-numbered open PR that carries `harness:pr-fix` **and** has a
/// check that concluded in failure — one CI read per candidate, in number
/// order, stopping at the first match.
///
/// The pairing is the point: the label alone would send a paid session at a
/// PR that is merely still building.
async fn lowest_red_pr(gh: &dyn GitHub) -> Outcome<Option<Pr>> {
    let mut found: Vec<Pr> = Vec::new();
    for candidate in by_number(gh.open_prs_labelled(labels::PR_FIX).await?) {
        if !gh.pr_failing_checks(&candidate.num).await?.is_empty() {
            found.push(candidate);
            break;
        }
    }
    // A blocking agent review, or a branch that no longer merges into its
    // base, asks for a repair as a red check does — the label is posed for
    // it at dispatch, the same request a human makes. Bounded like the
    // rest: past `MAX_FIXES` repairs, the PR is a human's.
    for candidate in by_number(gh.open_prs_labelled(labels::TO_REVIEW).await?) {
        let comments = gh.pr_comments(&candidate.num).await?;
        let blocked = review::status(&comments).wants_a_fix();
        let conflicting = review::repairs(&comments) < review::MAX_FIXES
            && gh.pr_mergeable(&candidate.num).await? == Some(false);
        if blocked || conflicting {
            found.push(candidate);
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
async fn lowest_pr_worth_reviewing(gh: &dyn GitHub) -> Outcome<Option<Pr>> {
    for candidate in by_number(gh.open_prs_labelled(labels::TO_REVIEW).await?) {
        let comments = gh.pr_comments(&candidate.num).await?;
        // `candidate.base` as the expected base: the label is the
        // authorization, and a task PR legitimately targets its milestone's
        // branch. What stays live are the other three rules.
        if skip_rules::skip_reason(false, &candidate.base, &candidate, &comments).is_none() {
            return Ok(Some(candidate));
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

/// Runs whichever workflow the route names. Reports, never propagates: see
/// the module doc on why a dispatch failure does not stop the loop.
async fn dispatch(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    route: Route,
    log: &Logbook,
    lanes: &mut Lanes,
) {
    // The route with its parameters, before anything runs: it is the one line
    // that says what this tick decided and on what.
    log.say(&format!("tick: {route:?}"));
    let failed = dispatched(args, here, gh, route, log, lanes).await;
    // Recorded before anything is repaired: the record is what the repair
    // reads, and what says — next tick — that it has already been treated.
    if let Some((workflow, halt)) = failed {
        crate::dispatch::doctor::record(here, &spending::run_id(), workflow, &halt);
        if args.no_doctor {
            return;
        }
        match crate::dispatch::doctor::treat(here, &args.workspaces_dir, args.dry_run, log).await {
            Ok(Repair::Nothing) => {}
            Ok(done) => log.say(&format!("watch: doctor -> {}", done.outcome())),
            Err(broke) => log.warn(&format!("watch: doctor -> {}", broke.reason())),
        }
    }
}

/// Runs the route, and says which workflow failed, if one did.
async fn dispatched(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    route: Route,
    log: &Logbook,
    lanes: &mut Lanes,
) -> Option<(&'static str, Halt)> {
    match route {
        Route::Nothing => None,
        Route::DevLoop { milestone } if args.parallel > 1 => {
            run_lanes(args, here, gh, milestone, lanes, log)
                .await
                .map(|halt| ("dev_loop", halt))
        }
        Route::DevLoop { milestone } => run_dev_loop(here, gh, milestone, args.force_reset, log)
            .await
            .map(|halt| ("dev_loop", halt)),
        Route::Planner { roadmap } => report_failure(
            "planner",
            crate::dispatch::planner::run(
                roadmap,
                here,
                &args.target_repo_url,
                &args.branch,
                &args.permission_mode,
                args.dry_run,
            )
            .await,
            log,
        ),
        Route::Split { .. } | Route::Refinement { .. } if args.parallel > 1 => {
            run_router_lanes(args, here, gh, lanes, log)
                .await
                .map(|halt| ("router_lanes", halt))
        }
        Route::Split { milestone } => report_failure(
            "split",
            crate::dispatch::split::run(
                milestone,
                here,
                &shared_checkout(args),
                &args.permission_mode,
                args.dry_run,
            )
            .await,
            log,
        ),
        Route::PrFix { pr } => {
            if let Err(halt) = ask_for_the_repair(gh, &pr, args.dry_run).await {
                log.warn(&format!("watch: pr_fix -> {}", halt.reason()));
                return Some(("pr_fix", halt));
            }
            report_failure(
                "pr_fix",
                crate::dispatch::pr_fix::run(
                    &pr,
                    here,
                    &args.target_repo_url,
                    &args.permission_mode,
                    args.dry_run,
                )
                .await,
                log,
            )
        }
        Route::PrReview { pr, base } => report_failure(
            "pr_review",
            crate::dispatch::pr_review::run(
                &pr,
                &base,
                here,
                &args.target_repo_url,
                &args.branch,
                &args.permission_mode,
                args.dry_run,
            )
            .await,
            log,
        ),
        Route::MergeIntoMilestone { task, pr, base } => {
            run_milestone_merge(gh, here, task, pr, base, log).await
        }
        Route::MergeMainAgent { milestone } => {
            run_main_agent_merge(args, here, milestone, log).await
        }
        Route::Refinement { issue } => {
            refine_here(args, here, gh, Phase::Business, issue, log).await
        }
        // Sequential on purpose, even under `--parallel`: the technical half
        // reads code a task may be changing, so it is not spread over lanes.
        Route::TechRefinement { issue } => {
            refine_here(args, here, gh, Phase::Technical, issue, log).await
        }
    }
}

/// One refinement round in this process, on the shared checkout.
async fn refine_here(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    phase: Phase,
    issue: u64,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    let workflow = match phase {
        Phase::Business => "refinement",
        Phase::Technical => "tech_refinement",
    };
    let branch = match gh.issue(issue).await {
        Ok(task) => refinement_branch(gh, &args.branch, &task).await,
        Err(_) => args.branch.clone(),
    };
    let checkout = crate::dispatch::shared::Checkout {
        branch: &branch,
        ..shared_checkout(args)
    };
    report_failure(
        workflow,
        crate::dispatch::refinement::run(
            phase,
            issue,
            here,
            &checkout,
            &args.permission_mode,
            args.dry_run,
        )
        .await,
        log,
    )
}

/// The branch a refinement of this task reads: its milestone's own when that
/// branch already exists on the remote, else the integration branch.
///
/// The map `explore` draws describes the checkout it reads. Drawn on
/// `main_agent`, the last task of a milestone saw a repository with none of
/// its sisters' work in it and listed "base unresolved" as its first
/// contradiction. A milestone whose branch `split` has not created yet has no
/// sisters' work to see, and reads the integration branch as before.
async fn refinement_branch(gh: &dyn GitHub, fallback: &str, task: &Issue) -> String {
    let Ok(Some(scope)) = hierarchy::around(gh, task).await else {
        return fallback.to_string();
    };
    let Ok(number) = scope.milestone.number.parse::<u64>() else {
        return fallback.to_string();
    };
    let branch = branching::milestone_branch(number, &scope.milestone.title);
    match gh.branch_sha(&branch).await {
        Ok(Some(_)) => branch,
        _ => fallback.to_string(),
    }
}

/// The shared read-only checkout a refinement run in this process reads.
fn shared_checkout(args: &WatchArgs) -> crate::dispatch::shared::Checkout<'_> {
    crate::dispatch::shared::Checkout {
        target_repo_url: &args.target_repo_url,
        branch: &args.branch,
        workspace: None,
    }
}

/// The task whose PR is open on its milestone, reviewed and green, with that
/// PR — what `milestone_merge` merges. A PR still owed its review is left to
/// the review route, which outranks this one.
async fn green_task_pr(gh: &dyn GitHub, tasks: &[Issue]) -> Outcome<Option<(u64, Pr)>> {
    if !tasks
        .iter()
        .any(harness_workflows::dev_loop::data::tasks::review_pending)
    {
        return Ok(None);
    }
    let open = gh.open_prs_labelled(labels::TO_REVIEW).await?;
    let Some((task, pr)) = harness_workflows::milestone_merge::candidate(tasks, &open) else {
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

/// Poses `harness:pr-fix` on a PR the route found blocked by its review —
/// the request `pr_fix` consumes, made on the review's behalf.
async fn ask_for_the_repair(gh: &dyn GitHub, pr_ref: &str, dry_run: bool) -> Outcome<()> {
    if dry_run {
        return Ok(());
    }
    let pr = gh.pr(pr_ref).await?;
    if pr.has(labels::PR_FIX) {
        return Ok(());
    }
    let number = pr
        .num
        .parse::<u64>()
        .map_err(|_| Halt::Failed(format!("{:?} is not a PR number", pr.num)))?;
    gh.add_label(number, labels::PR_FIX).await
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

/// Fills the free lanes with the router's own work, splits first: every
/// milestone ready to split, then every issue waiting on its **business**
/// refinement, that no lane is on — each a `harness split <n>` or `harness
/// refine <n>` child in a checkout of its own (`router-lane-<k>`), up to
/// `--parallel` at once. Two splits cut two different milestones, two
/// refinements write two different bodies, and a milestone is never split
/// while its refinement is pending: nothing here waits on anything else.
async fn run_router_lanes(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    lanes: &mut Lanes,
    log: &Logbook,
) -> Option<Halt> {
    let listed = async {
        let milestones =
            with_split_blockers(gh, gh.issues_labelled(labels::MILESTONE, "open").await?).await?;
        let refining = gh.issues_labelled(labels::REFINEMENT, "open").await?;
        Ok::<_, Halt>((milestones, refining))
    };
    let (milestones, refining) = match listed.await {
        Ok(found) => found,
        Err(halt) => {
            log.warn(&format!(
                "watch: router lanes -> cannot list the work: {}",
                halt.reason()
            ));
            return Some(halt);
        }
    };
    let running = lanes.running();
    let mut splits: Vec<u64> = milestones
        .iter()
        .filter(|issue| routing::splittable(issue))
        .map(|issue| issue.number)
        .filter(|number| !running.contains(number))
        .collect();
    splits.sort_unstable();
    let mut refining: Vec<&Issue> = refining
        .iter()
        .filter(|issue| issue.is_open() && !running.contains(&issue.number))
        .collect();
    refining.sort_unstable_by_key(|issue| issue.number);
    let mut refines: Vec<(u64, String)> = Vec::with_capacity(refining.len());
    for issue in refining {
        let branch = refinement_branch(gh, &args.branch, issue).await;
        refines.push((issue.number, branch));
    }
    let work = splits
        .into_iter()
        .map(|number| ("split", number, args.branch.clone()))
        .chain(
            refines
                .into_iter()
                .map(|(number, branch)| ("refine", number, branch)),
        );
    let mut started = 0usize;
    for (what, number, branch) in work {
        let Some(slot) = lanes.free_slot() else {
            log.say("watch: router lanes -> every lane is busy");
            break;
        };
        let Some(command) = router_lane_command(args, here, what, number, &branch, slot) else {
            log.warn("watch: router lanes -> cannot find this executable");
            return None;
        };
        match lanes.spawn(slot, number, command, here) {
            Ok(pid) => {
                started += 1;
                log.say(&format!(
                    "watch: router lanes -> lane {slot} {what}s #{number} (pid {pid})"
                ));
            }
            Err(e) => log.warn(&format!(
                "watch: router lanes -> cannot start {what} #{number}: {e}"
            )),
        }
    }
    if started == 0 && lanes.free_slot().is_some() {
        log.say("watch: router lanes -> every split and refinement is already on a lane");
    }
    None
}

/// `harness <what> <number>` on lane `slot`, in its own checkout.
fn router_lane_command(
    args: &WatchArgs,
    here: &Path,
    what: &str,
    number: u64,
    branch: &str,
    slot: usize,
) -> Option<tokio::process::Command> {
    let mut command = tokio::process::Command::new(std::env::current_exe().ok()?);
    command
        .current_dir(here)
        .arg(what)
        .arg(number.to_string())
        .arg("--use-workspace")
        .arg(format!("router-lane-{slot}"))
        .arg("--target-repo-url")
        .arg(&args.target_repo_url)
        .arg("--branch")
        .arg(branch)
        .arg("--permission-mode")
        .arg(&args.permission_mode);
    if args.dry_run {
        command.arg("--dry-run");
    }
    Some(command)
}

/// Leaves a run of `line` in the traces — the folder the view reads a line's
/// work from — for a command that works without a session of its own and
/// would otherwise show as never having run.
fn record_run(here: &Path, line: &str, issue: u64, said: &str, log: &Logbook) {
    let path = Workspace::new(here)
        .log_dir(line)
        .join(spending::run_id_for(issue))
        .join("run.log");
    match Both::new(&path) {
        Ok(sink) => Logbook::new(Rc::new(sink) as Rc<dyn Sink>, Verbosity::Quiet).say(said),
        Err(e) => log.warn(&format!("watch: cannot record the {line} run: {e}")),
    }
}

/// Merges one task's PR into its milestone, and says so.
async fn run_milestone_merge(
    gh: &dyn GitHub,
    here: &Path,
    task: u64,
    pr: String,
    base: String,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    let merged = async {
        let issue = gh.issue(task).await?;
        let pr = Pr {
            num: pr,
            base,
            ..Pr::default()
        };
        harness_workflows::milestone_merge::run(gh, &issue, &pr).await
    };
    match merged.await {
        Ok(()) => {
            log.say(&format!(
                "watch: milestone_merge -> #{task} merged into its milestone"
            ));
            record_run(
                here,
                "milestone-merge",
                task,
                &format!("task #{task}: reviewed and green — merged into its milestone, closed"),
                log,
            );
            None
        }
        Err(halt) => {
            log.warn(&format!("watch: milestone_merge -> {}", halt.reason()));
            Some(("milestone_merge", halt))
        }
    }
}

/// Merges one milestone, and says so. Its own function only because the
/// dispatch table is long enough already.
async fn run_main_agent_merge(
    args: &WatchArgs,
    here: &Path,
    milestone: u64,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    match crate::dispatch::main_agent_merge::run(
        milestone,
        here,
        &args.target_repo_url,
        &args.branch,
    )
    .await
    {
        Ok(outcome) => {
            log.say(&format!("watch: main_agent_merge -> {outcome:?}"));
            // Only what changed something gets a run of its own: a milestone
            // not ready yet, or waiting on its checks, is every tick's answer.
            let said = match &outcome {
                MergeReport::OpenedPr(url) => Some(format!(
                    "milestone #{milestone}: every task merged — opened {url} to the integration branch"
                )),
                MergeReport::Merged => Some(format!(
                    "milestone #{milestone}: CI green — merged into the integration branch, its tasks closed"
                )),
                MergeReport::NotReady | MergeReport::WaitingOnChecks => None,
            };
            if let Some(line) = said {
                record_run(here, "main-agent-merge", milestone, &line, log);
            }
            None
        }
        Err(halt) => {
            log.warn(&format!("watch: main_agent_merge -> {}", halt.reason()));
            Some(("main_agent_merge", halt))
        }
    }
}

/// What a parked task is compared on: its body and labels. Answering the
/// stop — editing the SPEC, a label — moves it.
fn issue_version(issue: &Issue) -> String {
    let mut labels = issue.labels.clone();
    labels.sort_unstable();
    harness_core::domain::breaker::fingerprint(&format!("{}\n{}", issue.body, labels.join(",")))
}

/// Runs the dev loop pointed at this milestone's own branch, derived from
/// its number and title (`common::branching`) — never the fixed
/// `--branch`/`INTEGRATION_BRANCH` a direct `harness` invocation would use.
/// Gives every runnable task of the milestone that no lane is on to a free
/// lane — a `harness --task <n>` child in its own workspace — up to
/// `--parallel` at once. Nothing runs in this process.
async fn run_lanes(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    milestone: u64,
    lanes: &mut Lanes,
    log: &Logbook,
) -> Option<Halt> {
    let board = match board::read(gh).await {
        Ok(board) => board,
        Err(halt) => {
            log.warn(&format!(
                "watch: lanes -> cannot read the board: {}",
                halt.reason()
            ));
            return Some(halt);
        }
    };
    if board.milestone.number != milestone {
        log.warn(&format!(
            "watch: lanes -> the board moved to milestone #{} since the route was decided",
            board.milestone.number
        ));
        return None;
    }
    let branch = branching::milestone_branch(milestone, &board.milestone.title);
    let running = lanes.running();
    let mut parked = Vec::new();
    let mut candidates: Vec<(u64, String)> = Vec::new();
    for task in harness_workflows::dev_loop::data::tasks::runnable_tasks(&board.tasks) {
        if running.contains(&task.number) {
            continue;
        }
        let version = issue_version(task);
        if lanes.is_parked(task.number, &version) {
            parked.push(task.number);
        } else {
            candidates.push((task.number, version));
        }
    }
    if candidates.is_empty() {
        let list = |tasks: &[u64]| {
            tasks
                .iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        log.say(&format!(
            "watch: lanes -> every runnable task is already on a lane ({}){}",
            list(&running),
            if parked.is_empty() {
                String::new()
            } else {
                format!(" or parked until its issue changes ({})", list(&parked))
            }
        ));
        return None;
    }
    for (number, version) in candidates {
        let Some(slot) = lanes.free_slot() else {
            log.say("watch: lanes -> every lane is busy");
            break;
        };
        let mut command = tokio::process::Command::new(match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                log.warn(&format!("watch: lanes -> cannot find this executable: {e}"));
                return None;
            }
        });
        command
            .current_dir(here)
            .arg("--task")
            .arg(number.to_string())
            .arg("--branch")
            .arg(&branch)
            .arg("--use-workspace")
            .arg(format!("lane-{slot}"))
            .arg("--lane")
            .arg("--quiet");
        if args.force_reset {
            command.arg("--force-reset");
        }
        if args.dry_run {
            command.arg("--dry-run");
        }
        lanes.taken(number, version);
        match lanes.spawn(slot, number, command, here) {
            Ok(pid) => log.say(&format!(
                "watch: lanes -> lane {slot} takes #{number} on {branch} (pid {pid})"
            )),
            Err(e) => log.warn(&format!("watch: lanes -> cannot start #{number}: {e}")),
        }
    }
    None
}

async fn run_dev_loop(
    here: &Path,
    gh: &dyn GitHub,
    milestone: u64,
    force_reset: bool,
    log: &Logbook,
) -> Option<Halt> {
    let title = match gh.issue(milestone).await {
        Ok(issue) => issue.title,
        Err(halt) => {
            log.warn(&format!(
                "watch: dev_loop -> cannot read milestone #{milestone}: {}",
                halt.reason()
            ));
            return Some(halt);
        }
    };
    let branch = branching::milestone_branch(milestone, &title);
    let mut cli = match crate::cli::Cli::try_parse_from(["harness"]) {
        Ok(cli) => cli,
        Err(e) => {
            log.warn(&format!(
                "watch: cannot build default dev_loop arguments: {e}"
            ));
            return None;
        }
    };
    cli.run.branch = branch;
    cli.run.force_reset = force_reset;
    match crate::dispatch::dev_loop::run(&cli.run, here).await {
        Ok(ran) => {
            log.say(&format!("watch: dev_loop -> {:?}", ran.verdict));
            None
        }
        Err(halt) => {
            log.warn(&format!("watch: dev_loop -> {}", halt.reason()));
            Some(halt)
        }
    }
}

/// Says what the workflow did, and hands back the `Halt` when it failed.
///
/// The return is what lets one place — [`dispatch`] — record every failure,
/// rather than each arm remembering to.
fn report_failure(
    name: &'static str,
    result: Outcome<Verdict>,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    match result {
        Ok(verdict) => {
            log.say(&format!("watch: {name} -> {verdict:?}"));
            None
        }
        Err(halt) => {
            log.warn(&format!("watch: {name} -> {}", halt.reason()));
            Some((name, halt))
        }
    }
}
