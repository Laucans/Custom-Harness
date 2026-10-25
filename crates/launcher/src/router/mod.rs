//! `harness watch`: polls on an interval, decides what to run next
//! (`common::routing`), dispatches, and loops.
//!
//! The only place that builds the `GitHub` adapter used purely for the
//! routing decision — a lightweight read, no checkout mounted for it.
//! Dispatching to a workflow is each workflow's own launcher module's job
//! (`dev_loop.rs`, `planner.rs`, `split.rs`, `refinement.rs`,
//! `pr_review.rs`, `pr_fix.rs`, `arbiter.rs`, `test_debt.rs`, `rules.rs`) — each mounts what it needs
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
//! **Under the disk floor no lane starts** (`floor`): the volume the lanes
//! build on is measured every tick; below `DISK_FLOOR_GB` the free lanes'
//! build folders are pruned and it is measured again, and if it is still
//! short only the free work runs — said once when it falls, once when it
//! is back.
//!
//! Four files: this one is the loop — a tick, its reads of the board, the
//! rest after a `QUOTA` stop; `routes` runs what a tick decided in this
//! process; `lanes` fills the lanes of a parallel watch with children;
//! `floor` holds them while the disk is short.

mod floor;
mod lanes;
mod routes;

use std::path::{Path, PathBuf};
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
use harness_workflows::common::{branching, catch_up, close_merged, contest, labels, sentinel};
use harness_workflows::dev_loop::data::{board, tasks};
use harness_workflows::main_agent_merge::data::audit;
use harness_workflows::main_agent_merge::data::merge_train;
use harness_workflows::milestone_merge::RulesPr;
use harness_workflows::pr_fix::data::candidates;
use harness_workflows::pr_review::data::skip_rules;
use harness_workflows::test_debt::data::landing;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::cli::WatchArgs;
use crate::dispatch::data_snapshot;
use crate::dispatch::dev_loop::now_seconds;
use crate::dispatch::lanes::Lanes;

use routes::dispatch;

/// The name of the one-watch-per-checkout lock, under [`watch_lock_dir`].
pub const WATCH_LOCK: &str = "watch";

/// Where the checkout at `here` keeps its watch lock: held by a running
/// watch, and by whatever must not run beside one.
pub fn watch_lock_dir(here: &Path) -> PathBuf {
    Workspace::new(here).state_root().join(".llocal/watch-lock")
}

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
    // One watch per checkout: two started at the same second (two restarts
    // racing, `ForgeDesMondes` 21:20Z) merged the same PR twice and shared
    // their lanes' checkouts. An OS lock, released when the process ends.
    let lock_dir = watch_lock_dir(here);
    if !harness_core::ports::store::lock::Locks::acquire(
        &harness_adapters::store::lock::DirLocks,
        &lock_dir,
        WATCH_LOCK,
    )? {
        return Err(Halt::Failed(format!(
            "another watch already runs on this checkout ({} is held) — stop it first",
            lock_dir.display()
        )));
    }
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
    // The tools every process of this watch runs with, said once: what an
    // inline catch-up's cargo and a session's shell will be.
    crate::toolchain::at_watch_start(here, &args.target_repo_url, &args.branch, &log).await;
    // The changes the human left in the view reach the runs here, and only
    // here: a start is the one moment no run is reading the book.
    match crate::dispatch::shared::apply_drafts(here, crate::dispatch::dev_loop::now_seconds()) {
        Ok(prompts) if prompts.is_empty() => {}
        Ok(prompts) => log.event(&Event::ChangesApplied { prompts }),
        Err(halt) => log.event(&Event::TickFailed {
            reason: format!("changes left waiting: {}", halt.reason()),
        }),
    }
    // What the harness installed in the target follows the harness: a fix
    // to an installed script or contract reaches a product made before.
    if !args.dry_run {
        crate::dispatch::init_repo::upgrade(&args.target_repo_url, &args.branch, here, &log).await;
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
    let mut floor = floor::Floor::new(
        args.disk_floor_gb,
        lanes_volume(args, here),
        Rc::new(harness_adapters::shell::volume::Statvfs),
    );
    // The tests' stale scratch databases: swept from the loop, not the
    // tick, which returns early on a quiet board or a quota rest — while
    // the lanes keep testing, and leaking.
    let mut scratch = crate::dispatch::scratch_sweep::Sweeper::of(
        std::env::var("DATABASE_URL").ok(),
        std::env::var("DATABASE_URL_READONLY").ok().as_deref(),
    );
    loop {
        let went = tick(args, here, &log, &mut lanes, &mut pulse, &mut floor).await;
        // A dry run drops nothing; the sweep says its own failures.
        if !args.dry_run
            && let Some(sweeper) = &mut scratch
        {
            sweeper
                .sweep(crate::dispatch::dev_loop::now_seconds(), &log)
                .await;
        }
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

/// The catch-ups' read of the integration branch's head this tick
/// (`catch_up::read_head`, at the clock's second): whether the board must
/// be read in full for them.
async fn catching_up(gh: &dyn GitHub, pulse: &mut Pulse, integration: &str) -> Outcome<bool> {
    catch_up::read_head(gh, &mut pulse.catch_ups, integration, now_seconds()).await
}

/// The folder whose volume the floor measures: the lanes' workspaces when
/// they exist — where the builds land — else the factory itself.
fn lanes_volume(args: &WatchArgs, here: &Path) -> PathBuf {
    let workspaces = Workspace::new(here).workspaces_in(&args.workspaces_dir);
    if workspaces.exists() {
        workspaces
    } else {
        here.to_path_buf()
    }
}

/// What the disk lets this tick start. Under the floor the free lanes'
/// build folders go first, so space the watch owns never holds it — a dry
/// run removes nothing.
fn room(
    args: &WatchArgs,
    here: &Path,
    lanes: &Lanes,
    floor: &mut floor::Floor,
    log: &Logbook,
) -> floor::Room {
    floor.check(log, || {
        reclaim(
            args,
            here,
            &lanes.idle(),
            &harness_adapters::shell::disk::RealDisk,
            log,
        );
    })
}

/// Prunes the build folders of the `idle` lanes' checkouts, under the
/// floor — a dry run removes nothing.
fn reclaim(
    args: &WatchArgs,
    here: &Path,
    idle: &[usize],
    disk: &dyn harness_core::ports::shell::disk::Disk,
    log: &Logbook,
) {
    if !args.dry_run {
        for slot in idle {
            lanes::prune_slot(args, here, *slot, disk, log);
        }
    }
}

/// This tick's pace once the lanes' quota news is taken: a lane struck by
/// the quota rests the watch, a session that went through ends the rest.
fn quota_pace(args: &WatchArgs, lanes: &mut Lanes, pulse: &mut Pulse, log: &Logbook) -> Pace {
    if lanes.struck_by_quota() {
        rest(pulse, log);
    }
    if lanes.went_through() {
        // A session went through on a lane: the quota is back.
        pulse.rest.cleared();
    }
    let pace = pace(args.parallel, !pulse.rest.passes());
    if pace == Pace::Paused || pace == Pace::FreeOnly {
        log.debug(&format!(
            "resting: quota exhausted — {} tick(s) left{}",
            pulse.rest.ticks_left,
            if pace == Pace::FreeOnly {
                ", the free work goes on"
            } else {
                ""
            }
        ));
    }
    pace
}

/// One pass: read the snapshot, decide, dispatch.
async fn tick(
    args: &WatchArgs,
    here: &Path,
    log: &Logbook,
    lanes: &mut Lanes,
    pulse: &mut Pulse,
    floor: &mut floor::Floor,
) -> Outcome<()> {
    let gh = resolve_gh(args, here)?;
    // Room found again is a reason to read the board, like a freed lane:
    // the jobs the floor held back moved nothing on it.
    let room = room(args, here, lanes, floor, log);
    let freed = lanes.reap(log) + usize::from(room == floor::Room::Back);
    let pace = quota_pace(args, lanes, pulse, log);
    // One route at a time builds in this process: under the floor, none.
    if pace == Pace::Paused || (pace == Pace::OneRoute && room == floor::Room::Short) {
        return Ok(());
    }
    let activity = gh.activity(pulse.etag.as_deref()).await?;
    // The integration branch's head, every tick (one call), and its own
    // checks while a branch waits on them (one more): a push that moves no
    // issue — an upgrade, a human's — and the green that follows it are
    // what the catch-ups follow.
    let catching_up = catching_up(gh.as_ref(), pulse, &args.branch).await?;
    if routing::may_skip(pulse, &activity, freed, catching_up) {
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
    // The base first: what the rest of the read may build on, and the one
    // issue a red base opens, before anything is decided on it.
    let base_checks = sentinel::watch(gh.as_ref(), &args.branch, args.dry_run, log).await?;
    let (mut found, idle, watching_ci) = snapshot(
        gh.as_ref(),
        &args.branch,
        args.parallel.max(1),
        &lanes.running(),
        base_checks,
        log,
    )
    .await?;
    found.database_down = database_down(log).await;
    found.rule_waits = rule_waits(gh.as_ref(), pulse, &args.branch, log).await;
    // A red base is read in full every tick like a pull request waiting on
    // its checks: the push that turns it green moves no issue. The
    // catch-ups due at the head just read, once its own checks are green.
    pulse.read_in_full(etag, watching_ci || routing::base_red(&found));
    routing::read_catch_ups(&mut found, pulse, now_seconds(), log);
    routing::hold_debt_merges(&mut found, pulse, now_seconds());
    // A PR held on a settled contest is told once it stalls; a rules' task
    // held on its faithful PR is released past the bound — before the taps,
    // which read the hold.
    routing::tell_settling(&found, pulse, log);
    routing::bound_rules_on_pr(&mut found, pulse, log);
    if args.no_human_gate {
        routing::open_taps(gh.as_ref(), &mut found, pulse, log).await?;
    }
    // The board moved: a merge may have moved a branch, whose data model is
    // archived before anything else is dispatched.
    snapshot_data(args, here, &gh, &found.milestones, log).await;
    say_saw(&found, log);
    if pace != Pace::OneRoute {
        if lanes.is_closed() {
            // A soft stop arrived during the read: nothing new is dispatched.
            return Ok(());
        }
        if routing::schedule(&found, &lanes.running(), &|_| 0).is_empty()
            && let Some(reason) = &idle
        {
            log.say(&format!("idle: {reason}"));
        }
        // A free job never meets the quota, and a lane's is told by its
        // exit: what the schedule came to is the doctor's, already recorded.
        // Under the floor, the free work only: no lane starts.
        let _ = lanes::run_schedule(
            args,
            here,
            gh.as_ref(),
            &found,
            pulse,
            pace == Pace::Lanes && room != floor::Room::Short,
            lanes,
            log,
        )
        .await;
        return Ok(());
    }
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
    match dispatch(args, here, gh.as_ref(), route, pulse, log).await {
        Dispatched::Quota => rest(pulse, log),
        Dispatched::Went => pulse.rest.cleared(),
        Dispatched::Nothing | Dispatched::Failed => {}
    }
    Ok(())
}

/// Whether the product's database (its `DATABASE_URL`) refuses a
/// connection: a code session or a repair started then tests without it
/// (see `harness_adapters::shell::database`). Said when it does.
async fn database_down(log: &Logbook) -> bool {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return false;
    };
    let down = harness_adapters::shell::database::answers(&url).await == Some(false);
    if down {
        log.warn(&format!(
            "the database at {} does not answer — code sessions and repairs wait \
             (is Docker up? `orbctl start`)",
            harness_adapters::shell::database::address_of(&url).unwrap_or_default()
        ));
    }
    down
}

/// What the integration branch's `rules-status.json` says the rules' tests
/// wait for (`routing::Snapshot::rule_waits`): read once per integration
/// head, when its checks concluded (`routing::rules_status_due`), kept in
/// the pulse in between. A read that fails is said and tried again at the
/// next full read; the waits it had stay — they order work, never hold it.
async fn rule_waits(
    gh: &dyn GitHub,
    pulse: &mut Pulse,
    branch: &str,
    log: &Logbook,
) -> harness_workflows::common::rule_waits::Waits {
    if let Some(head) = routing::rules_status_due(pulse) {
        match gh
            .artifact_file("gates.yml", branch, "rules-status", "rules-status.json")
            .await
        {
            Ok(kept) => {
                routing::took_rules_status(
                    pulse,
                    head.clone(),
                    kept.as_ref().map(|k| k.text.as_str()),
                );
                log.debug(&format!(
                    "rules-status: read at {head} — {} pending rule(s) wait on a unit",
                    pulse.rule_waits.0.len()
                ));
            }
            Err(halt) => log.debug(&format!(
                "rules-status: not read at {head} — {}",
                halt.reason()
            )),
        }
    }
    pulse.rule_waits.clone()
}

/// The tasks that unblock the most pending rules, as the `saw:` line names
/// them (`#157 unblocks 17`).
const SAID_UNBLOCKERS: usize = 3;

/// What the decision was made from, for a tick whose choice later looks
/// wrong: the counts are the whole input to `routing::schedule`.
fn say_saw(found: &Snapshot, log: &Logbook) {
    log.debug(&format!(
        "saw: base={:?}, {} roadmap, {} milestone(s), {} refining, {} tech-refining, \
         {} arbitrating, {} test-debt, {} rules, pr_fix={:?}, pr_review={:?}, to_merge={:?}, \
         ready_to_merge={:?}, dev_loop={:?}, catch_up={:?}, catch_up_held={:?}, \
         rules_on_pr={:?}, resume={:?}, rules: {}",
        found.base_checks,
        found.roadmap.len(),
        found.milestones.len(),
        found.refining.len(),
        found.tech_refining.len(),
        found.arbitrating.len(),
        found.test_debt.len(),
        found.rules.len(),
        found
            .prs_to_fix
            .iter()
            .map(|pr| pr.num.clone())
            .collect::<Vec<_>>(),
        found
            .prs_to_review
            .iter()
            .map(|pr| pr.num.clone())
            .collect::<Vec<_>>(),
        found
            .tasks_to_merge
            .iter()
            .map(|(task, _)| *task)
            .collect::<Vec<_>>(),
        found.ready_to_merge,
        found.dev_loop_milestones,
        found
            .catch_up
            .iter()
            .map(|(milestone, _)| *milestone)
            .collect::<Vec<_>>(),
        found.catch_up_held,
        found.rules_on_pr,
        found
            .settled
            .iter()
            .map(|settled| settled.contest)
            .collect::<Vec<_>>(),
        unblockers(found),
    ));
}

/// The tasks that unblock the most pending rules, said: `#157 unblocks 17,
/// #160 unblocks 4` — `none` when no task of the boards read unblocks one.
fn unblockers(found: &Snapshot) -> String {
    let said: Vec<String> = routing::rule_unblockers(found)
        .into_iter()
        .take(SAID_UNBLOCKERS)
        .map(|(task, unblocked)| format!("#{task} unblocks {}", unblocked.touched))
        .collect();
    if said.is_empty() {
        "none".to_string()
    } else {
        said.join(", ")
    }
}

/// What a tick may start, given the lanes and the quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pace {
    /// One route at a time, resting: nothing to read, nothing to start.
    Paused,
    /// One route at a time, run in the tick.
    OneRoute,
    /// Lanes, resting: the free work only — the merges.
    FreeOnly,
    /// Lanes: every job, the paid ones on free lanes.
    Lanes,
}

/// The tick's [`Pace`]: `parallel` lanes, and whether a quota rest holds.
const fn pace(parallel: usize, resting: bool) -> Pace {
    match (parallel > 1, resting) {
        (false, true) => Pace::Paused,
        (false, false) => Pace::OneRoute,
        (true, true) => Pace::FreeOnly,
        (true, false) => Pace::Lanes,
    }
}

/// A `QUOTA` stop: the watch rests, longer with every stop in a row, and
/// says so once.
fn rest(pulse: &mut Pulse, log: &Logbook) {
    let ticks = pulse.quota_struck();
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

/// One of the router's paid routes on a lane (`harness route`): the same
/// arm the tick runs in its own process, so the review's repair request and
/// every other effect stay where they are. Exit 0 when it went through, 1
/// when it halted (recorded as the tick records it).
pub async fn run_route(args: &crate::cli::RouteArgs, here: &Path) -> std::process::ExitCode {
    let journal = Workspace::new(here).loop_dir().join("watch.log");
    let Ok(sink) = Both::new(&journal) else {
        return std::process::ExitCode::from(2);
    };
    let log = Logbook::new(Rc::new(sink) as Rc<dyn Sink>, Verbosity::Normal);
    let gh = match resolve_gh(&args.watch, here) {
        Ok(gh) => gh,
        Err(halt) => {
            log.error(&format!("route: {}", halt.reason()));
            return std::process::ExitCode::from(2);
        }
    };
    let route = routes::lane_route(args.kind, args.number, &args.base);
    match routes::run_on_lane(&args.watch, here, gh.as_ref(), route, &log).await {
        None => std::process::ExitCode::SUCCESS,
        Some((workflow, halt)) => {
            crate::dispatch::doctor::record(here, &spending::run_id(), workflow, &halt);
            std::process::ExitCode::from(1)
        }
    }
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
/// tick it cannot trust a quiet activity read. `base_checks` is what the
/// sentinel read of the integration branch, already.
async fn snapshot(
    gh: &dyn GitHub,
    branch: &str,
    lanes: usize,
    running: &[u64],
    base_checks: Option<bool>,
    log: &Logbook,
) -> Outcome<(Snapshot, Option<String>, bool)> {
    let roadmap = gh.issues_labelled(labels::ROADMAP, "open").await?;
    let lowest_roadmap_has_milestone = match routing::plannable_roadmap(&roadmap) {
        Some(lowest) => !gh.sub_issues(lowest.number).await?.is_empty(),
        None => false,
    };
    let milestones =
        with_split_blockers(gh, gh.issues_labelled(labels::MILESTONE, "open").await?).await?;
    // The two labelled lists once, for the three reads that need them.
    let to_fix = by_number(gh.open_prs_labelled(labels::PR_FIX).await?);
    let to_review = by_number(gh.open_prs_labelled(labels::TO_REVIEW).await?);
    let review_backlog_full = backlog_full(&to_fix, &to_review, lanes, log);
    let watching_ci = watching_ci(&to_fix, &to_review);
    // Every PR of the harness's cycle, as read: what holds a task's branch
    // against a second session, whether a lane is on the PR or not.
    let task_prs = task_prs(&to_fix, &to_review);
    // A PR a lane is already repairing or reviewing is not the tick's to
    // pick again: picked, it ended the tick on "already on a lane" while
    // the other PRs waited (`ForgeDesMondes`' four reviews ran one at a
    // time on five lanes, 19:44Z–20:10Z).
    let (fixable, reviewable) = (
        not_running(&to_fix, running),
        not_running(&to_review, running),
    );
    let rules = gh.issues_labelled(labels::RULES, "open").await?;
    let (prs_waiting, prs_to_fix, closed_rules) =
        repairs_due(gh, &task_prs, (&fixable, &reviewable), &rules, log).await?;
    let prs_to_review = prs_worth_reviewing(gh, &reviewable).await?;
    // The plan's graph, when the plan wrote one: what orders the milestones
    // and says which may build side by side.
    let graph = routing::parse_graph(gh.file_text(routing::GRAPH_PATH, branch).await?.as_deref());
    let ready_to_merge = milestones_ready_to_merge(gh, &milestones, graph.as_ref()).await?;
    let refining = gh.issues_labelled(labels::REFINEMENT, "open").await?;
    let tech_refining = gh.issues_labelled(labels::TECH_REFINEMENT, "open").await?;
    let arbitrating = gh
        .issues_labelled(labels::NEEDS_AI_DECISION, "open")
        .await?;
    let test_debt = gh.issues_labelled(labels::TEST_DEBT, "open").await?;
    let deciding = gh.issues_labelled(labels::NEEDS_DECISION, "open").await?;
    let contract_changes = gh.issues_labelled(labels::CONTRACT_CHANGE, "open").await?;
    // One board per milestone building (`harness:triggered`), the critical
    // path's first; the lowest open one when none is split yet. Tolerant on
    // purpose: "no open milestone at all" is this probe's most common
    // answer, not a failure — `dev_loop::run` itself is where that
    // distinction matters and is already made.
    let boards = board::building(gh, &milestones, graph.as_ref(), lanes).await;
    let dev_loop_milestones: Vec<u64> = boards
        .iter()
        .filter(|b| b.next().is_some())
        .map(|b| b.milestone.number)
        .collect();
    let idle = if dev_loop_milestones.is_empty() {
        boards
            .first()
            .map(|b| tasks::idle_reason(b.milestone.number, &b.tasks))
    } else {
        None
    };
    // One green PR per board: a milestone's branch takes one merge a tick,
    // the next one is read against it.
    let mut tasks_to_merge = Vec::new();
    let mergeable = contest::not_waiting(&to_review, &prs_waiting);
    for board in &boards {
        tasks_to_merge.extend(green_task_pr(gh, &board.tasks, &mergeable, running, log).await?);
    }
    let (rules_merge, rules_on_pr) = rules_train(gh, &rules).await?;
    tasks_to_merge.extend(rules_merge);
    // The settled contests, followed through for free: a few reads per
    // contest whose PR or milestone is still open.
    let settled =
        contest::settled(gh, &closed_rules, &rules, &task_prs, &milestones, branch).await?;
    let (tasks_merged, contracts_landed, debt_landings) =
        follow_ups(gh, &boards, running, &prs_waiting, branch).await?;
    let boards_read = boards;
    let snapshot = Snapshot {
        tasks_to_merge,
        task_prs,
        roadmap,
        lowest_roadmap_has_milestone,
        milestones,
        prs_to_fix,
        prs_to_review,
        ready_to_merge,
        refining,
        tech_refining,
        arbitrating,
        dev_loop_milestones,
        boards: boards_read,
        graph,
        contract_changes,
        test_debt,
        rules,
        deciding,
        base_checks,
        database_down: false,
        review_backlog_full,
        catch_up: Vec::new(),
        catch_up_held: Vec::new(),
        prs_waiting,
        rules_on_pr,
        settled,
        tasks_merged,
        contracts_landed,
        debt_landings,
        // Read from the pulse by the tick, once per integration head.
        rule_waits: harness_workflows::common::rule_waits::Waits::default(),
    };
    // The tests come first: a development that waits on them says so.
    let idle = routing::rules_first(&snapshot)
        .map(|n| {
            format!(
                "development waits on the rules' task #{n} — {} on it asks for an attempt",
                labels::READY
            )
        })
        .or(idle);
    Ok((snapshot, idle, watching_ci))
}

/// The PRs that wait, the PRs to repair among the others, and the page of
/// closed rules' issues read for it — the settled contests are followed
/// through from the same page.
///
/// The rules' issues, open and closed, are read before the repairs: a PR
/// whose contest is open waits, so does one whose contest closed and that
/// is not resumed yet, and one a decision told to wait — no repair, no
/// decision on its task, no merge (`common::contest`). Its review runs.
/// A PR labelled for repair past its cap that no merge path escalates is
/// escalated here (`milestone_merge::exhausted_repairs`).
async fn repairs_due(
    gh: &dyn GitHub,
    task_prs: &[Pr],
    (fixable, reviewable): (&[Pr], &[Pr]),
    rules: &[Issue],
    log: &Logbook,
) -> Outcome<(Vec<(Pr, contest::PrWait)>, Vec<Pr>, Vec<Issue>)> {
    let closed = gh.issues_labelled(labels::RULES, "closed").await?;
    let waiting = contest::waiting(gh, task_prs, rules, &closed).await?;
    say_waits(&waiting, rules, log);
    let fixable = contest::not_waiting(fixable, &waiting);
    let reviewable = contest::not_waiting(reviewable, &waiting);
    let due = candidates::due(gh, &fixable, &reviewable).await?;
    // Past their repairs, the PRs no merge path escalates (a task PR never
    // put under review): one decision asked on the task each.
    let exhausted =
        harness_workflows::milestone_merge::exhausted_repairs(gh, &fixable, &reviewable, log)
            .await?;
    for (task, pr) in exhausted {
        log.event(&Event::RepairsExhausted {
            task,
            pr,
            fixes: u64::try_from(harness_workflows::common::review::MAX_FIXES).unwrap_or(u64::MAX),
        });
    }
    Ok((waiting, by_number(due), closed))
}

/// The rules' task whose PR the train merges now, if one, and the rules'
/// tasks whose faithful PR is mergeable or still checking — neither tapped
/// nor attempted again while it is.
///
/// The rules' PR sits on no board, so the train reads it here — once green
/// it merges into the integration branch and the task closes, which is what
/// frees the development the rules hold. Nothing is read while no rules'
/// task is open.
async fn rules_train(gh: &dyn GitHub, rules: &[Issue]) -> Outcome<(Option<(u64, Pr)>, Vec<u64>)> {
    if !rules.iter().any(Issue::is_open) {
        return Ok((None, Vec::new()));
    }
    let open_rules = by_number(gh.open_prs_labelled(labels::RULES).await?);
    let stands = harness_workflows::milestone_merge::rules_prs(gh, rules, &open_rules).await?;
    Ok(train_of(&stands))
}

/// What the train takes of where the rules' PRs stand: the first mergeable
/// one, and the tasks whose PR is mergeable or still checking.
fn train_of(stands: &[(&Issue, &Pr, RulesPr)]) -> (Option<(u64, Pr)>, Vec<u64>) {
    let merge = stands
        .iter()
        .find(|(_, _, stands)| *stands == RulesPr::Mergeable)
        .map(|(task, pr, _)| (task.number, (*pr).clone()));
    let on_pr = stands
        .iter()
        .filter(|(_, _, stands)| matches!(stands, RulesPr::Mergeable | RulesPr::Checking))
        .map(|(task, _, _)| task.number)
        .collect();
    (merge, on_pr)
}

/// Says, once a full read, each PR that waits and on what, and warns of an
/// open contest that names no rule: it holds nothing beyond its PR.
fn say_waits(waiting: &[(Pr, contest::PrWait)], rules: &[Issue], log: &Logbook) {
    for (pr, on) in waiting {
        log.say(&format!("watch: pr #{} waits on {}", pr.num, on.describe()));
    }
    for issue in rules.iter().filter(|issue| {
        issue.is_open()
            && contest::contested_pr(&issue.title).is_some()
            && harness_workflows::common::dossier::rule_ids(&issue.body).is_empty()
    }) {
        log.warn(&format!(
            "watch: contest #{} names no rule — it holds nothing beyond its PR",
            issue.number
        ));
    }
}

/// Whether as many task PRs are open as there are lanes — said when so: no
/// new task starts until reviews, repairs and merges bring them down.
fn backlog_full(to_fix: &[Pr], to_review: &[Pr], lanes: usize, log: &Logbook) -> bool {
    let open = open_task_prs(to_fix, to_review);
    if open >= lanes {
        log.say(&format!(
            "watch: {open} task PRs open for {lanes} lane(s) — no new task until reviews, \
             repairs and merges bring them down"
        ));
    }
    open >= lanes
}

/// How many task PRs are open — those into a milestone's branch, the
/// labelled lists joined, each once.
fn open_task_prs(to_fix: &[Pr], to_review: &[Pr]) -> usize {
    let mut nums: Vec<&str> = to_fix
        .iter()
        .chain(to_review)
        .filter(|pr| pr.base.starts_with("milestone/"))
        .map(|pr| pr.num.as_str())
        .collect();
    nums.sort_unstable();
    nums.dedup();
    nums.len()
}

/// The open PRs carrying `harness:to-review` that the review itself would
/// not skip, in number order — one comments read each.
///
/// The decision reuses `pr_review`'s own rules rather than restating them:
/// policy in two places is policy that drifts. Applying them here is only
/// an optimization — a PR that would skip (a draft, a test PR, one already
/// carrying a review) must not cost a mounted checkout on every poll, and
/// its label can be left in place harmlessly because of this filter.
async fn prs_worth_reviewing(gh: &dyn GitHub, to_review: &[Pr]) -> Outcome<Vec<Pr>> {
    let mut found = Vec::new();
    for candidate in to_review {
        let comments = gh.pr_comments(&candidate.num).await?;
        // `candidate.base` as the expected base: the label is the
        // authorization, and a task PR legitimately targets its milestone's
        // branch. What stays live are the other three rules.
        if skip_rules::skip_reason(false, &candidate.base, candidate, &comments).is_none() {
            found.push(candidate.clone());
        }
    }
    Ok(found)
}

/// Whether a PR of the harness's is under CI at all: the pulse then reads
/// the board even when nothing moved on it.
const fn watching_ci(to_fix: &[Pr], to_review: &[Pr]) -> bool {
    !to_fix.is_empty() || !to_review.is_empty()
}

/// The open PRs of both labelled lists, each once, in number order — what
/// `routing::pr_cycle_owns` reads.
fn task_prs(to_fix: &[Pr], to_review: &[Pr]) -> Vec<Pr> {
    let mut found: Vec<Pr> = to_fix.to_vec();
    for pr in to_review {
        if !found.iter().any(|seen| seen.num == pr.num) {
            found.push(pr.clone());
        }
    }
    by_number(found)
}

/// The PRs no lane holds: a lane is keyed by the number it works on, a PR's
/// for a repair or a review.
fn not_running(prs: &[Pr], running: &[u64]) -> Vec<Pr> {
    prs.iter()
        .filter(|pr| {
            pr.num
                .parse::<u64>()
                .is_ok_and(|number| !running.contains(&number))
        })
        .cloned()
        .collect()
}

/// PRs in number order, each once, the ones whose number is unreadable last — a PR ref
/// that is not a number cannot be ordered, and dropping it silently would
/// hide it instead.
fn by_number(mut prs: Vec<Pr>) -> Vec<Pr> {
    prs.sort_by_key(|pr| pr.num.parse::<u64>().unwrap_or(u64::MAX));
    // A PR carrying both `harness:pr-fix` and `harness:to-review` is found
    // twice; it is one repair, not two (#304, 2026-10-20).
    prs.dedup_by(|later, earlier| later.num == earlier.num);
    prs
}

/// The open milestones, not yet `harness:waiting-merge`, whose own tasks
/// are all delivered and whose producers have landed — those the plan's
/// graph names, then those of what its tasks need
/// (`merge_train::landing_waits`, closed tasks included) — one extra read
/// per candidate, in number order. The train's order: a consumer never
/// lands before what it reads, an edge the graph forgot included.
async fn milestones_ready_to_merge(
    gh: &dyn harness_core::ports::shell::github::GitHub,
    milestones: &[harness_core::domain::Issue],
    graph: Option<&harness_workflows::common::routing::Graph>,
) -> Outcome<Vec<u64>> {
    let mut candidates: Vec<&harness_core::domain::Issue> = milestones
        .iter()
        .filter(|issue| issue.is_open() && !issue.has(labels::WAITING_MERGE))
        .filter(|issue| merge_train::producers_landed(graph, issue.number, milestones).is_empty())
        .collect();
    candidates.sort_by_key(|issue| issue.number);
    let mut found = Vec::new();
    for candidate in candidates {
        let tasks = gh.sub_issues(candidate.number).await?;
        if lands_now(graph, candidate.number, milestones, &tasks) {
            found.push(candidate.number);
        }
    }
    Ok(found)
}

/// Whether `milestone` may land now: its own `tasks` all delivered, and no
/// producer its landing waits on.
fn lands_now(
    graph: Option<&harness_workflows::common::routing::Graph>,
    milestone: u64,
    milestones: &[Issue],
    tasks: &[Issue],
) -> bool {
    audit::all_tasks_delivered(tasks)
        && merge_train::landing_waits(graph, milestone, milestones, tasks).is_empty()
}

/// The task whose PR is open on its milestone, reviewed and green, with that
/// PR — what `milestone_merge` merges ([`milestone_merge::ready_to_merge`]).
/// A task met on the way whose repairs are used up is escalated there; the
/// event is recorded here, where the journal is.
async fn green_task_pr(
    gh: &dyn GitHub,
    tasks: &[Issue],
    to_review: &[Pr],
    running: &[u64],
    log: &Logbook,
) -> Outcome<Option<(u64, Pr)>> {
    let (found, escalated) =
        harness_workflows::milestone_merge::ready_to_merge(gh, tasks, to_review, running).await?;
    for (task, pr) in escalated {
        log.event(&Event::RepairsExhausted {
            task,
            pr,
            fixes: u64::try_from(harness_workflows::common::review::MAX_FIXES).unwrap_or(u64::MAX),
        });
    }
    Ok(found.map(|(task, pr)| (task.number, pr.clone())))
}

/// The free follow-ups a full read finds: the tasks merged outside the
/// train (one read of each building milestone's merged PRs), the PRs whose
/// contract change landed (one issue read per PR waiting on one), the
/// test-debt PRs the harness lands (a few reads per labelled PR).
async fn follow_ups(
    gh: &dyn GitHub,
    boards: &[board::Board],
    running: &[u64],
    prs_waiting: &[(Pr, contest::PrWait)],
    branch: &str,
) -> Outcome<(
    Vec<close_merged::Delivered>,
    Vec<contest::ContractLanded>,
    Vec<landing::Landing>,
)> {
    let tasks_merged = merged_outside_the_train(gh, boards, running).await?;
    let contracts_landed = contest::contract_landed(gh, prs_waiting, branch).await?;
    let debt_prs = by_number(gh.open_prs_labelled(labels::TEST_DEBT_PR).await?);
    let debt_landings = landing::ready(gh, &debt_prs, branch).await?;
    Ok((tasks_merged, contracts_landed, debt_landings))
}

/// The open tasks of each board whose PR was merged into the milestone's
/// branch outside the merge train (`common::close_merged::due`): one
/// `merged_prs` read per building milestone, a few more per task found.
async fn merged_outside_the_train(
    gh: &dyn GitHub,
    boards: &[board::Board],
    running: &[u64],
) -> Outcome<Vec<close_merged::Delivered>> {
    let mut found = Vec::new();
    for board in boards {
        if !board.tasks.iter().any(Issue::is_open) {
            continue;
        }
        let branch = branching::milestone_branch(board.milestone.number, &board.milestone.title);
        let merged = gh.merged_prs(&branch).await?;
        found.extend(close_merged::due(gh, &board.tasks, &merged, &branch, running).await?);
    }
    Ok(found)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that keeps what the log said.
    #[derive(Default)]
    struct Lines(std::cell::RefCell<Vec<String>>);

    impl Sink for Lines {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    /// What `run` says through a log, joined.
    fn said(run: impl FnOnce(&Logbook)) -> String {
        let lines = Rc::new(Lines::default());
        let log = Logbook::new(Rc::clone(&lines) as Rc<dyn Sink>, Verbosity::Normal);
        run(&log);
        lines.0.borrow().join("\n")
    }

    /// The arguments of `harness watch` followed by `extra`.
    fn watch(extra: &[&str]) -> WatchArgs {
        use clap::Parser as _;
        let mut line = vec!["harness", "watch"];
        line.extend_from_slice(extra);
        match crate::cli::Cli::try_parse_from(line) {
            Ok(crate::cli::Cli {
                command: Some(crate::cli::Command::Watch(args)),
                ..
            }) => args,
            other => panic!("a watch command line: {other:?}"),
        }
    }

    /// A throwaway folder, removed on drop.
    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_floor_measures_the_lanes_workspaces_once_they_exist() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let scratch = Scratch(
            std::env::temp_dir().join(format!("watch-volume-{}-{stamp}", std::process::id())),
        );
        let factory = scratch.0.join("factory");
        std::fs::create_dir_all(&factory).expect("mkdir");
        let args = watch(&["--workspaces-dir", "../ws"]);
        assert_eq!(lanes_volume(&args, &factory), factory, "no workspaces yet");
        std::fs::create_dir_all(scratch.0.join("ws")).expect("mkdir");
        assert_eq!(lanes_volume(&args, &factory), factory.join("../ws"));
    }

    #[test]
    fn under_the_floor_the_idle_lanes_are_pruned_unless_dry_run() {
        use crate::dispatch::lanes::tests::Builds;
        let here = Path::new("/f/factory");
        let built = || {
            Builds::with(&[(
                "/f/factory/../ws/lane-1/target/debug/deps/a-0123456789abcdef",
                3 * floor::GIB,
                Some(1),
            )])
        };
        let flags = [
            "--parallel",
            "3",
            "--workspaces-dir",
            "../ws",
            "--lane-target-cap-gb",
            "1",
        ];
        let disk = built();
        let dry = [&flags[..], &["--dry-run"]].concat();
        reclaim(&watch(&dry), here, &[1], &disk, &Logbook::null());
        assert!(
            disk.removed.borrow().is_empty(),
            "a dry run removes nothing"
        );
        let disk = built();
        reclaim(&watch(&flags), here, &[1], &disk, &Logbook::null());
        assert_eq!(
            *disk.removed.borrow(),
            [PathBuf::from("/f/factory/../ws/lane-1/target/debug")]
        );
    }

    #[tokio::test]
    async fn the_rule_waits_kept_stay_while_no_read_is_due() {
        use crate::testing::NoGitHub;
        use harness_workflows::common::rule_waits::Waits;
        let mut pulse = Pulse {
            rule_waits: Waits(std::collections::BTreeMap::from([(
                "R3".to_string(),
                vec!["capability:a".to_string()],
            )])),
            ..Pulse::default()
        };
        let kept = pulse.rule_waits.clone();
        let read = rule_waits(&NoGitHub, &mut pulse, "main_agent", &Logbook::null()).await;
        assert_eq!(read, kept);
    }

    #[tokio::test]
    async fn the_rules_train_reads_github_only_while_a_rules_task_is_open() {
        use crate::testing::NoGitHub;
        let rules = |state: &str| {
            vec![Issue {
                number: 7,
                state: state.to_string(),
                ..Issue::default()
            }]
        };
        let (merge, on_pr) = rules_train(&NoGitHub, &rules("closed"))
            .await
            .expect("nothing read");
        assert!(merge.is_none() && on_pr.is_empty());
        assert!(
            rules_train(&NoGitHub, &rules("open")).await.is_err(),
            "an open rules task reads its PRs"
        );
    }

    #[test]
    fn the_train_merges_the_first_mergeable_rules_pr_and_holds_the_checking_ones() {
        let task = |number: u64| Issue {
            number,
            ..Issue::default()
        };
        let pr = |num: &str| Pr {
            num: num.to_string(),
            ..Pr::default()
        };
        let (t1, t2, t3, t4) = (task(1), task(2), task(3), task(4));
        let (p1, p2, p3, p4) = (pr("11"), pr("12"), pr("13"), pr("14"));
        let stands = [
            (&t1, &p1, RulesPr::Red),
            (&t2, &p2, RulesPr::Checking),
            (&t3, &p3, RulesPr::Mergeable),
            (&t4, &p4, RulesPr::Unjudged),
        ];
        let (merge, on_pr) = train_of(&stands);
        assert_eq!(
            merge.map(|(task, pr)| (task, pr.num)),
            Some((3, "13".to_string()))
        );
        assert_eq!(on_pr, [2, 3]);
        assert!(train_of(&[]).0.is_none());
    }

    #[test]
    fn a_milestone_lands_once_its_tasks_are_delivered() {
        let task = |state: &str| Issue {
            number: 9,
            state: state.to_string(),
            ..Issue::default()
        };
        assert!(lands_now(None, 5, &[], &[task("closed")]));
        assert!(
            !lands_now(None, 5, &[], &[task("open")]),
            "a task still building"
        );
        assert!(!lands_now(None, 5, &[], &[]), "no task, nothing delivered");
    }

    #[test]
    fn a_waiting_pr_is_said_and_only_an_open_contest_naming_no_rule_is_warned() {
        let issue = |number: u64, state: &str, title: &str, body: &str| Issue {
            number,
            state: state.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            ..Issue::default()
        };
        let rules = [
            issue(40, "open", "Rules contested: PR #12", "no rule here"),
            issue(41, "open", "Rules contested: PR #13", "contests R3"),
            issue(42, "open", "rules for milestone 4", "no rule here"),
            issue(43, "closed", "Rules contested: PR #14", "no rule here"),
            issue(44, "closed", "rules for milestone 5", ""),
        ];
        let waiting = [(
            Pr {
                num: "12".to_string(),
                ..Pr::default()
            },
            contest::PrWait::Contest { issue: 40 },
        )];
        let out = said(|log| say_waits(&waiting, &rules, log));
        assert!(out.contains("watch: pr #12 waits on contest #40"), "{out}");
        assert!(out.contains("watch: contest #40 names no rule"), "{out}");
        for quiet in [41, 42, 43, 44] {
            assert!(!out.contains(&format!("#{quiet} names no rule")), "{out}");
        }
    }

    #[test]
    fn a_full_backlog_is_said_and_one_with_room_is_not() {
        let pr = |num: &str| Pr {
            num: num.to_string(),
            base: "milestone/2-x".to_string(),
            ..Pr::default()
        };
        let full = said(|log| {
            backlog_full(&[pr("1")], &[pr("3")], 2, log);
        });
        assert!(
            full.contains("watch: 2 task PRs open for 2 lane(s)"),
            "{full}"
        );
        let room = said(|log| {
            backlog_full(&[pr("1")], &[], 2, log);
        });
        assert!(room.is_empty(), "{room}");
    }

    #[test]
    fn the_watch_lock_sits_under_the_checkouts_own_state() {
        assert_eq!(
            watch_lock_dir(Path::new("/repo")),
            PathBuf::from("/repo/.llocal/watch-lock")
        );
    }

    #[test]
    fn a_pr_a_lane_holds_is_not_offered_again() {
        let pr = |num: &str| Pr {
            num: num.to_string(),
            ..Pr::default()
        };
        let prs = vec![pr("251"), pr("252"), pr("x")];
        let left: Vec<String> = not_running(&prs, &[252, 7])
            .into_iter()
            .map(|pr| pr.num)
            .collect();
        assert_eq!(
            left,
            ["251"],
            "the one on a lane and the unnumbered are out"
        );
        assert_eq!(not_running(&prs, &[]).len(), 2);
    }

    #[test]
    fn the_cycle_s_prs_are_both_lists_each_once_lane_or_not() {
        let pr = |num: &str| Pr {
            num: num.to_string(),
            ..Pr::default()
        };
        let found: Vec<String> = task_prs(&[pr("264"), pr("9")], &[pr("264"), pr("262")])
            .into_iter()
            .map(|pr| pr.num)
            .collect();
        assert_eq!(found, ["9", "262", "264"]);
        assert!(
            task_prs(&[], &[]).is_empty(),
            "task_prs(&[], &[]) should be empty"
        );
    }

    #[test]
    fn a_pr_found_twice_is_listed_once_in_number_order() {
        let pr = |num: &str| Pr {
            num: num.to_string(),
            ..Pr::default()
        };
        let found: Vec<String> = by_number(vec![pr("304"), pr("280"), pr("304"), pr("x")])
            .into_iter()
            .map(|pr| pr.num)
            .collect();
        assert_eq!(found, ["280", "304", "x"]);
    }

    #[test]
    fn ci_is_watched_while_any_pr_is_to_fix_or_to_review() {
        let one = vec![Pr::default()];
        assert!(!watching_ci(&[], &[]));
        assert!(watching_ci(&one, &[]));
        assert!(watching_ci(&[], &one));
    }

    #[test]
    fn a_quota_strike_rests_the_watch_and_forgets_its_taps() {
        let mut pulse = Pulse::default();
        assert!(pulse.may_open(2) && pulse.may_open(2) && !pulse.may_open(2));
        rest(&mut pulse, &Logbook::null());
        assert_eq!(pulse.rest.ticks_left, routing::FIRST_REST);
        assert_eq!(pulse.rest.strikes, 1);
        assert!(pulse.may_open(2), "the refused attempt did not count");
    }

    #[test]
    fn a_quota_rest_pauses_one_route_and_keeps_the_free_work_of_the_lanes() {
        assert_eq!(pace(1, true), Pace::Paused);
        assert_eq!(pace(0, false), Pace::OneRoute);
        assert_eq!(pace(1, false), Pace::OneRoute);
        assert_eq!(pace(2, true), Pace::FreeOnly);
        assert_eq!(pace(5, false), Pace::Lanes);
    }

    #[test]
    fn the_backlog_is_full_once_as_many_task_prs_are_open_as_lanes() {
        let pr = |num: &str, base: &str| Pr {
            num: num.to_string(),
            base: base.to_string(),
            ..Pr::default()
        };
        let to_fix = [pr("1", "milestone/2-x"), pr("2", "main_agent")];
        let to_review = [pr("1", "milestone/2-x"), pr("3", "milestone/2-x")];
        let log = Logbook::null();
        assert!(
            backlog_full(&to_fix, &to_review, 2, &log),
            "two task PRs, two lanes"
        );
        assert!(
            !backlog_full(&to_fix, &to_review, 3, &log),
            "a landing PR is no task PR"
        );
        assert!(backlog_full(&[], &[], 0, &log));
        assert!(!backlog_full(&[], &[], 1, &log));
    }

    #[tokio::test]
    async fn the_follow_up_reads_reach_github_only_for_what_can_be_due() {
        use crate::testing::NoGitHub;
        let closed = |number: u64| Issue {
            number,
            state: "closed".to_string(),
            ..Issue::default()
        };
        let done = board::Board {
            tasks: vec![closed(11)],
            ..board::Board::default()
        };
        let building = board::Board {
            tasks: vec![Issue {
                number: 12,
                state: "open".to_string(),
                ..Issue::default()
            }],
            ..board::Board::default()
        };
        // A board with no open task reads nothing; one with an open task
        // reads its milestone's merged PRs, refused here.
        let nothing = merged_outside_the_train(&NoGitHub, std::slice::from_ref(&done), &[])
            .await
            .expect("nothing read");
        assert_eq!(nothing.len(), 0);
        assert!(
            merged_outside_the_train(&NoGitHub, &[done.clone(), building], &[])
                .await
                .is_err()
        );
        // The debt PRs are always read: refused, the full read fails.
        assert!(
            follow_ups(&NoGitHub, &[done], &[], &[], "main_agent")
                .await
                .is_err()
        );
        // The repairs read the closed rules' issues first.
        assert!(
            repairs_due(&NoGitHub, &[], (&[], &[]), &[], &Logbook::null())
                .await
                .is_err()
        );
    }

    #[test]
    fn the_saw_line_names_the_tasks_that_unblock_the_most_rules() {
        use harness_workflows::common::rule_waits::Waits;
        let task = |number: u64, produces: &str| Issue {
            number,
            state: "open".to_string(),
            labels: vec![labels::AGENT.to_string(), labels::READY.to_string()],
            body: format!("## Architecture\n\nunit: capability\nsystem: c\nproduces: {produces}"),
            ..Issue::default()
        };
        let waits = Waits(
            (0..5)
                .map(|i| {
                    let mut needs = vec!["capability:a".to_string()];
                    if i < 2 {
                        needs.push("capability:b".to_string());
                    }
                    (format!("R{i}"), needs)
                })
                .collect(),
        );
        let found = |rule_waits: Waits| Snapshot {
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
            boards: vec![board::Board {
                tasks: vec![
                    task(9, "z"),
                    task(7, "b"),
                    task(8, "a"),
                    task(6, "c"),
                    task(5, "a"),
                ],
                ..board::Board::default()
            }],
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
            rule_waits,
        };
        assert_eq!(
            unblockers(&found(waits)),
            "#5 unblocks 5, #8 unblocks 5, #7 unblocks 2",
            "most first, then by number, three at most"
        );
        assert_eq!(unblockers(&found(Waits::default())), "none");
    }
}
