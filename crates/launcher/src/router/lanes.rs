//! The lanes of a parallel watch: every free lane is given a runnable task
//! of the current milestone, then a milestone to split or an issue waiting on
//! its business refinement — each a `harness` child in a checkout of its
//! own. Nothing here runs in this process.

use std::path::Path;

use harness_core::domain::{Halt, Issue};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook};
use harness_workflows::common::routing::{self, Snapshot};
use harness_workflows::common::{branching, labels};
use harness_workflows::dev_loop::data::board;

use crate::cli::WatchArgs;
use crate::dispatch::lanes::Lanes;

use super::routes::refinement_branch;
use super::with_split_blockers;

/// Gives every free lane work, whatever the route: the runnable tasks of the
/// current milestone first, then milestones to split and issues waiting on
/// their business refinement in the lanes left
/// ([`routing::lane_work`]).
pub(super) async fn fill_lanes(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    found: &Snapshot,
    log: &Logbook,
    lanes: &mut Lanes,
) -> Option<(&'static str, Halt)> {
    let work = routing::lane_work(found);
    if let Some(milestone) = work.tasks_of
        && lanes.free_slot().is_some()
        && let Some(halt) = run_lanes(args, here, gh, milestone, lanes, log).await
    {
        return Some(("dev_loop", halt));
    }
    if work.preparation
        && lanes.free_slot().is_some()
        && let Some(halt) = run_router_lanes(args, here, gh, lanes, log).await
    {
        return Some(("router_lanes", halt));
    }
    None
}

/// Fills the free lanes with the router's own work, splits first: every
/// milestone ready to split, then every issue waiting on its **business**
/// refinement, that no lane is on — each a `harness split <n>` or `harness
/// refine <n>` child in a checkout of its own (`router-lane-<k>`), up to
/// `--parallel` at once. Two splits cut two different milestones, two
/// refinements write two different bodies, and a milestone is never split
/// while its refinement is pending: nothing here waits on anything else.
pub(super) async fn run_router_lanes(
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
        .filter(|issue| routing::splittable(issue, &milestones))
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
            log.error("watch: router lanes -> cannot find this executable");
            return None;
        };
        match lanes.spawn(slot, number, command, here) {
            Ok(pid) => {
                started += 1;
                log.event(&Event::PreparationTaken {
                    lane: u32::try_from(slot).unwrap_or(u32::MAX),
                    what: what.to_string(),
                    issue: number,
                    pid,
                });
            }
            Err(e) => log.event(&Event::LaneNotStarted {
                what: Some(what.to_string()),
                issue: number,
                reason: e.to_string(),
            }),
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
pub(super) async fn run_lanes(
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
    // The runnable tasks not on a lane, those changing an aggregate another
    // holds left for a later tick.
    for task in harness_workflows::dev_loop::data::tasks::startable_tasks(&board.tasks, &running) {
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
    if !args.dry_run {
        branching::catch_up(gh, &branch, &args.branch, log).await;
    }
    for (number, version) in candidates {
        let Some(slot) = lanes.free_slot() else {
            log.say("watch: lanes -> every lane is busy");
            break;
        };
        let mut command = tokio::process::Command::new(match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                log.error(&format!("watch: lanes -> cannot find this executable: {e}"));
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
            Ok(pid) => log.event(&Event::LaneTaken {
                lane: u32::try_from(slot).unwrap_or(u32::MAX),
                task: number,
                branch: branch.clone(),
                pid,
            }),
            Err(e) => log.event(&Event::LaneNotStarted {
                what: None,
                issue: number,
                reason: e.to_string(),
            }),
        }
    }
    None
}
