//! What a tick decided, run in this process: the route's workflow, the
//! merges, one refinement round on the shared checkout — and the one place
//! that records a failed dispatch for the doctor.

use std::path::Path;
use std::rc::Rc;

use clap::Parser as _;
use harness_core::domain::doctor::Repair;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Issue, Outcome, Pr, Verdict};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook, Sink, Verbosity};
use harness_workflows::common::routing::{Route, Snapshot};
use harness_workflows::common::{branching, hierarchy, labels};
use harness_workflows::main_agent_merge::data::report::Outcome as MergeReport;
use harness_workflows::refinement::data::phase::Phase;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::cli::WatchArgs;
use crate::dispatch::lanes::Lanes;

use super::Dispatched;
use super::lanes::fill_lanes;

/// Runs whichever workflow the route names. Reports, never propagates: see
/// the module doc on why a dispatch failure does not stop the loop.
pub(super) async fn dispatch(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    found: &Snapshot,
    route: Route,
    log: &Logbook,
    lanes: &mut Lanes,
) -> Dispatched {
    // The route with its parameters, before anything runs: it is the one line
    // that says what this tick decided and on what.
    log.say(&format!("tick: {route:?}"));
    let nothing = matches!(route, Route::Nothing);
    let filled = if args.parallel > 1 {
        fill_lanes(args, here, gh, found, log, lanes).await
    } else {
        None
    };
    let failed = filled.or(dispatched(args, here, gh, route, log).await);
    // Recorded before anything is repaired: the record is what the repair
    // reads, and what says — next tick — that it has already been treated.
    let Some((workflow, halt)) = failed else {
        return if nothing && args.parallel <= 1 {
            Dispatched::Nothing
        } else {
            Dispatched::Went
        };
    };
    let came_to = if matches!(halt, Halt::Quota(_)) {
        Dispatched::Quota
    } else {
        Dispatched::Failed
    };
    crate::dispatch::doctor::record(here, &spending::run_id(), workflow, &halt);
    if args.no_doctor {
        return came_to;
    }
    match crate::dispatch::doctor::treat(here, &args.workspaces_dir, args.dry_run, log).await {
        Ok(Repair::Nothing) => {}
        Ok(done) => log.say(&format!("watch: doctor -> {}", done.outcome())),
        Err(broke) => log.error(&format!("watch: doctor -> {}", broke.reason())),
    }
    came_to
}

/// Runs the route, and says which workflow failed, if one did.
async fn dispatched(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    route: Route,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    match route {
        Route::Nothing => None,
        // Under `--parallel`, the lanes were filled before the route was
        // dispatched — see [`fill_lanes`].
        Route::DevLoop { .. } | Route::Split { .. } | Route::Refinement { .. }
            if args.parallel > 1 =>
        {
            None
        }
        Route::DevLoop { milestone } => run_dev_loop(here, gh, milestone, args, log)
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
        Route::Split { milestone } => report_failure(
            "split",
            crate::dispatch::split::run(
                milestone,
                here,
                &shared_checkout(args),
                &args.permission_mode,
                args.dry_run,
                &harness_workflows::split::config::profile_for(&args.split_profile, ""),
                explicit_profile(&args.split_profile),
            )
            .await,
            log,
        ),
        Route::PrFix { pr } => {
            if let Err(halt) = ask_for_the_repair(gh, &pr, args.dry_run).await {
                log.error(&format!("watch: pr_fix -> {}", halt.reason()));
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
        Route::Arbiter { issue } => arbitrate_here(args, here, issue, log).await,
        Route::Decide { issue } => decide_here(args, here, issue, log).await,
        Route::Rules { issue } => rules_here(args, here, issue, log).await,
        Route::TestDebt { issue } => report_failure(
            "test_debt",
            crate::dispatch::test_debt::run(
                issue,
                here,
                &args.target_repo_url,
                &args.branch,
                &args.permission_mode,
                args.dry_run,
            )
            .await,
            log,
        ),
    }
}

/// One decision in this process, on the shared checkout.
async fn decide_here(
    args: &WatchArgs,
    here: &Path,
    issue: u64,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    report_failure(
        "decide",
        crate::dispatch::decide::run(
            issue,
            here,
            &shared_checkout(args),
            &args.permission_mode,
            args.dry_run,
        )
        .await,
        log,
    )
}

/// One rules' task in this process, in a writable checkout of its own.
async fn rules_here(
    args: &WatchArgs,
    here: &Path,
    issue: u64,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    report_failure(
        "rules",
        crate::dispatch::rules::run(
            issue,
            here,
            &args.target_repo_url,
            &args.branch,
            &args.permission_mode,
            args.dry_run,
        )
        .await,
        log,
    )
}

/// One arbitration in this process, on the shared checkout: one short
/// session over the milestone's index, and the task it settles is held
/// until it ran.
async fn arbitrate_here(
    args: &WatchArgs,
    here: &Path,
    issue: u64,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    report_failure(
        "arbiter",
        crate::dispatch::arbiter::run(
            issue,
            false,
            here,
            &shared_checkout(args),
            &args.permission_mode,
            args.dry_run,
        )
        .await,
        log,
    )
}

/// One refinement round in this process, on the shared checkout.
pub(super) async fn refine_here(
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
pub(super) async fn refinement_branch(gh: &dyn GitHub, fallback: &str, task: &Issue) -> String {
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
pub(super) fn shared_checkout(args: &WatchArgs) -> crate::dispatch::shared::Checkout<'_> {
    crate::dispatch::shared::Checkout {
        target_repo_url: &args.target_repo_url,
        branch: &args.branch,
        workspace: None,
    }
}

/// Poses `harness:pr-fix` on a PR the route found blocked by its review —
/// the request `pr_fix` consumes, made on the review's behalf.
pub(super) async fn ask_for_the_repair(
    gh: &dyn GitHub,
    pr_ref: &str,
    dry_run: bool,
) -> Outcome<()> {
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

/// Leaves a run of `line` in the traces — the folder the view reads a line's
/// work from — for a command that works without a session of its own and
/// would otherwise show as never having run.
pub(super) fn record_run(here: &Path, line: &str, issue: u64, said: &str, log: &Logbook) {
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
pub(super) async fn run_milestone_merge(
    gh: &dyn GitHub,
    here: &Path,
    task: u64,
    pr: String,
    base: String,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    let told = (format!("#{pr}"), base.clone());
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
            let (pr, branch) = told;
            log.event(&Event::TaskDelivered { task, pr, branch });
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
            log.error(&format!("watch: milestone_merge -> {}", halt.reason()));
            Some(("milestone_merge", halt))
        }
    }
}

/// Merges one milestone, and says so. Its own function only because the
/// dispatch table is long enough already.
pub(super) async fn run_main_agent_merge(
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
                MergeReport::Merged { pr, roadmap } => {
                    log.event(&Event::MilestoneLanded {
                        milestone,
                        pr: pr.clone(),
                    });
                    if let Some(roadmap) = roadmap {
                        log.event(&Event::RoadmapDelivered { roadmap: *roadmap });
                    }
                    Some(format!(
                        "milestone #{milestone}: CI green — merged into the integration branch, its tasks closed"
                    ))
                }
                MergeReport::NotReady | MergeReport::WaitingOnChecks => None,
            };
            if let Some(line) = said {
                record_run(here, "main-agent-merge", milestone, &line, log);
            }
            None
        }
        Err(halt) => {
            log.error(&format!("watch: main_agent_merge -> {}", halt.reason()));
            Some(("main_agent_merge", halt))
        }
    }
}

pub(super) async fn run_dev_loop(
    here: &Path,
    gh: &dyn GitHub,
    milestone: u64,
    args: &WatchArgs,
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
    branching::catch_up(gh, &branch, &args.branch, log).await;
    let mut cli = match crate::cli::Cli::try_parse_from(["harness"]) {
        Ok(cli) => cli,
        Err(e) => {
            log.error(&format!(
                "watch: cannot build default dev_loop arguments: {e}"
            ));
            return None;
        }
    };
    cli.run.branch = branch;
    cli.run.force_reset = args.force_reset;
    cli.run.no_human_gate = args.no_human_gate;
    match crate::dispatch::dev_loop::run(&cli.run, here).await {
        Ok(ran) => {
            log.say(&format!("watch: dev_loop -> {:?}", ran.verdict));
            None
        }
        Err(halt) => {
            log.error(&format!("watch: dev_loop -> {}", halt.reason()));
            Some(halt)
        }
    }
}

/// Says what the workflow did, and hands back the `Halt` when it failed.
///
/// The return is what lets one place — [`dispatch`] — record every failure,
/// rather than each arm remembering to.
pub(super) fn report_failure(
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
            log.error(&format!("watch: {name} -> {}", halt.reason()));
            Some((name, halt))
        }
    }
}

/// Whether the human named the split's profile (`--split-profile`,
/// `SPLIT_PROFILE`): a named profile is never adjusted by the ledger.
fn explicit_profile(split_profile: &str) -> bool {
    !split_profile.trim().is_empty()
}

#[cfg(test)]
mod explicit_profile_tests {
    use super::explicit_profile;

    #[test]
    fn a_named_profile_is_explicit_and_blank_is_not() {
        assert!(explicit_profile("small"));
        assert!(!explicit_profile(""));
        assert!(!explicit_profile("   "));
    }
}
