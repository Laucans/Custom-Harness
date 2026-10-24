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
use harness_workflows::common::catch_up::Busy;
use harness_workflows::common::routing::{Pulse, Route};
use harness_workflows::common::{branching, hierarchy, labels};
use harness_workflows::main_agent_merge::data::report::Outcome as MergeReport;
use harness_workflows::milestone_merge::Attempt;
use harness_workflows::refinement::data::phase::Phase;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::cli::{RouteKind, WatchArgs};

use super::Dispatched;

/// Runs whichever workflow the route names, in this process — a watch
/// that runs one route at a time. Reports, never propagates: see the module
/// doc on why a dispatch failure does not stop the loop.
pub(super) async fn dispatch(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    route: Route,
    pulse: &mut Pulse,
    log: &Logbook,
) -> Dispatched {
    // The route with its parameters, before anything runs: it is the one line
    // that says what this tick decided and on what.
    log.say(&format!("tick: {route:?}"));
    let went = !matches!(route, Route::Nothing);
    let failed = dispatched(args, here, gh, route, pulse, &Busy::default(), log).await;
    after(args, here, failed, went, log).await
}

/// What a tick's dispatch came to: a failure recorded for the doctor, and
/// treated, before anything else is repaired — the record is what the
/// repair reads, and what says, next tick, that it has already been
/// treated. `went`: something ran and went through.
pub(super) async fn after(
    args: &WatchArgs,
    here: &Path,
    failed: Option<(&'static str, Halt)>,
    went: bool,
    log: &Logbook,
) -> Dispatched {
    let Some((workflow, halt)) = failed else {
        return if went {
            Dispatched::Went
        } else {
            Dispatched::Nothing
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

/// Runs the route, and says which workflow failed, if one did. `pulse` is
/// what a catch-up remembers; `busy`, what lanes work on, which a
/// catch-up leaves alone (a landing PR under repair, a task PR whose
/// session still runs).
pub(super) async fn dispatched(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    route: Route,
    pulse: &mut Pulse,
    busy: &Busy,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    match route {
        Route::Nothing => None,
        // Told in the journal, never a halt: a branch that cannot take the
        // base is parked or routed by the catch-up itself.
        Route::CatchUp { milestone, branch } => {
            crate::dispatch::catch_up::milestone(
                here,
                &args.target_repo_url,
                &args.branch,
                args.dry_run,
                gh,
                (milestone, &branch),
                &mut pulse.catch_ups,
                busy,
                log,
            )
            .await;
            None
        }
        Route::DevLoop { milestone } | Route::DevTask { milestone, .. } => {
            run_dev_loop(here, gh, milestone, args, log)
                .await
                .map(|halt| ("dev_loop", halt))
        }
        Route::Planner { roadmap } => report_failure(
            "planner",
            crate::dispatch::planner::run(
                roadmap,
                here,
                &shared_checkout(args),
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
        Route::PrFix { pr } => fix_here(args, here, gh, &pr, log).await,
        Route::PrReview { pr, base } => report_failure(
            "pr_review",
            crate::dispatch::pr_review::run(
                &pr,
                &base,
                here,
                &shared_checkout(args),
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
        Route::ContractChange { issue } => contract_here(args, here, issue, log).await,
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

/// One repair, its request posed first: the label `pr_fix` consumes.
async fn fix_here(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    pr: &str,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    if let Err(halt) = ask_for_the_repair(gh, pr, args.dry_run).await {
        log.error(&format!("watch: pr_fix -> {}", halt.reason()));
        return Some(("pr_fix", halt));
    }
    report_failure(
        "pr_fix",
        crate::dispatch::pr_fix::run(
            pr,
            here,
            &args.target_repo_url,
            &args.permission_mode,
            route_workspace(args),
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
            route_workspace(args),
            args.dry_run,
        )
        .await,
        log,
    )
}

/// One contract evolved in this process, on the shared checkout.
async fn contract_here(
    args: &WatchArgs,
    here: &Path,
    issue: u64,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    report_failure(
        "contract_manager",
        crate::dispatch::contract_manager::run(
            issue,
            here,
            &shared_checkout(args),
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

/// The route `harness route <kind> <number>` names.
pub(super) fn lane_route(kind: RouteKind, number: u64, base: &str) -> Route {
    match kind {
        RouteKind::Decide => Route::Decide { issue: number },
        RouteKind::Rules => Route::Rules { issue: number },
        RouteKind::Contract => Route::ContractChange { issue: number },
        RouteKind::PrFix => Route::PrFix {
            pr: number.to_string(),
        },
        RouteKind::PrReview => Route::PrReview {
            pr: number.to_string(),
            base: base.to_string(),
        },
        RouteKind::Split => Route::Split { milestone: number },
        RouteKind::Refine => Route::Refinement { issue: number },
        RouteKind::TechRefine => Route::TechRefinement { issue: number },
        RouteKind::Planner => Route::Planner { roadmap: number },
        RouteKind::Arbiter => Route::Arbiter { issue: number },
        RouteKind::TestDebt => Route::TestDebt { issue: number },
    }
}

/// What `harness route` runs on its lane: the tick's own arm. A lane runs
/// a paid route only, which remembers nothing between ticks: its pulse is
/// its own.
pub(super) async fn run_on_lane(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    route: Route,
    log: &Logbook,
) -> Option<(&'static str, Halt)> {
    dispatched(
        args,
        here,
        gh,
        route,
        &mut Pulse::default(),
        &Busy::default(),
        log,
    )
    .await
}

/// The paid route a lane takes, with what names it there: its kind, the
/// issue or PR number, and a PR's base. `None` for the free routes — the
/// merges, run in the tick — and for a task, which a lane runs as the dev
/// loop's `--task` child.
pub(super) fn lane_work(route: &Route) -> Option<(RouteKind, u64, String)> {
    let plain = |kind: RouteKind, number: u64| Some((kind, number, String::new()));
    match route {
        Route::Decide { issue } => plain(RouteKind::Decide, *issue),
        Route::Rules { issue } => plain(RouteKind::Rules, *issue),
        Route::ContractChange { issue } => plain(RouteKind::Contract, *issue),
        Route::PrFix { pr } => pr.parse().ok().and_then(|n| plain(RouteKind::PrFix, n)),
        Route::PrReview { pr, base } => pr
            .parse()
            .ok()
            .map(|n| (RouteKind::PrReview, n, base.clone())),
        Route::Split { milestone } => plain(RouteKind::Split, *milestone),
        Route::Refinement { issue } => plain(RouteKind::Refine, *issue),
        Route::TechRefinement { issue } => plain(RouteKind::TechRefine, *issue),
        Route::Planner { roadmap } => plain(RouteKind::Planner, *roadmap),
        Route::Arbiter { issue } => plain(RouteKind::Arbiter, *issue),
        Route::TestDebt { issue } => plain(RouteKind::TestDebt, *issue),
        Route::CatchUp { .. }
        | Route::MergeIntoMilestone { .. }
        | Route::MergeMainAgent { .. }
        | Route::DevLoop { .. }
        | Route::DevTask { .. }
        | Route::Nothing => None,
    }
}

/// `harness route <what> <number>` on lane `slot`, with the watch's own
/// settings and a checkout of its own.
pub(super) fn lane_command(
    args: &WatchArgs,
    here: &Path,
    exe: &Path,
    what: &str,
    number: u64,
    base: &str,
    slot: usize,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(exe);
    command
        .current_dir(here)
        .arg("route")
        .arg(what)
        .arg(number.to_string())
        .arg("--route-workspace")
        .arg(format!("router-lane-{slot}"))
        .arg("--target-repo-url")
        .arg(&args.target_repo_url)
        .arg("--branch")
        .arg(&args.branch)
        .arg("--permission-mode")
        .arg(&args.permission_mode);
    if !base.is_empty() {
        command.arg("--base").arg(base);
    }
    for (on, flag) in [
        (args.dry_run, "--dry-run"),
        (args.no_human_gate, "--no-human-gate"),
        (args.force_reset, "--force-reset"),
    ] {
        if on {
            command.arg(flag);
        }
    }
    command
}

/// The checkout a route run on a lane was given, if any.
fn route_workspace(args: &WatchArgs) -> Option<&str> {
    (!args.route_workspace.is_empty()).then_some(args.route_workspace.as_str())
}

/// The shared read-only checkout a refinement run in this process reads.
pub(super) fn shared_checkout(args: &WatchArgs) -> crate::dispatch::shared::Checkout<'_> {
    crate::dispatch::shared::Checkout {
        target_repo_url: &args.target_repo_url,
        branch: &args.branch,
        workspace: (!args.route_workspace.is_empty()).then_some(args.route_workspace.as_str()),
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
        Ok(Attempt::Merged) => {
            let (pr, branch) = told;
            log.say(&format!(
                "watch: milestone_merge -> #{task} merged into {branch}"
            ));
            log.event(&Event::TaskDelivered {
                task,
                pr,
                branch: branch.clone(),
            });
            record_run(
                here,
                "milestone-merge",
                task,
                &format!("task #{task}: reviewed and green — merged into {branch}, closed"),
                log,
            );
            None
        }
        // The route read the checks green a moment ago; the merge reads
        // them again and refuses what changed since. Nothing is posed here:
        // the train reads the PR again next tick, and a red one stays where
        // it is — reviewed clean, it is no repair route's until a label or
        // a review asks for the repair.
        Ok(Attempt::Pending) => {
            log.say(&format!(
                "watch: milestone_merge -> #{task} waits: a check of PR {} is still running",
                told.0
            ));
            None
        }
        Ok(Attempt::Red(checks)) => {
            log.warn(&format!(
                "watch: milestone_merge -> #{task} refused: {} failing check(s) on PR {} — {}",
                checks.len(),
                told.0,
                checks.join("; ")
            ));
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
                MergeReport::Merged { pr, roadmap, built } => {
                    log.event(&Event::MilestoneLanded {
                        milestone,
                        pr: pr.clone(),
                    });
                    if let Some(roadmap) = roadmap {
                        log.event(&Event::RoadmapDelivered { roadmap: *roadmap });
                    }
                    Some(format!(
                        "milestone #{milestone}: CI green — merged into the integration branch, its tasks \
                         closed; {} id(s) marked built — the branches still building take it at the \
                         next tick its checks are green",
                        built.len()
                    ))
                }
                // Red is told every tick it holds: a milestone PR that
                // failed does not turn green by waiting.
                MergeReport::Red { pr, checks } => {
                    log.warn(&format!(
                        "watch: main_agent_merge -> PR {pr} of #{milestone} is red: {} failing \
                         check(s), its repair asked — {}",
                        checks.len(),
                        checks.join("; ")
                    ));
                    None
                }
                // Neither stops the watch: a conflict waits for a catch-up
                // (`Route::CatchUp`, the same key, heavier), a refusal for a
                // later tick — told every tick it holds.
                MergeReport::Conflicting { pr, why } => {
                    log.warn(&format!(
                        "watch: main_agent_merge -> PR {pr} of #{milestone} conflicts with the \
                         integration branch: not merged, it waits for a catch-up — {why}"
                    ));
                    None
                }
                MergeReport::MergeabilityUnread { pr, why } => {
                    log.warn(&format!(
                        "watch: main_agent_merge -> whether PR {pr} of #{milestone} can merge \
                         cannot be read: {why} — not merged, read again at a later tick"
                    ));
                    None
                }
                MergeReport::MergeRefused { pr, why } => {
                    log.warn(&format!(
                        "watch: main_agent_merge -> GitHub refused to merge PR {pr} of \
                         #{milestone}: {why} — tried again at a later tick"
                    ));
                    None
                }
                MergeReport::NotReady
                | MergeReport::WaitingOnProducers(_)
                | MergeReport::WaitingOnChecks => None,
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
    // A branch behind the base, or one that could not take it, is held by
    // the router (`routing::Snapshot::catch_up`): the loop runs on a branch
    // the catch-up already brought up.
    let branch = branching::milestone_branch(milestone, &title);
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

#[cfg(test)]
mod lane_route_tests {
    use super::*;

    fn watch() -> WatchArgs {
        match crate::cli::Cli::try_parse_from(["harness", "watch", "--parallel", "3"]) {
            Ok(crate::cli::Cli {
                command: Some(crate::cli::Command::Watch(args)),
                ..
            }) => args,
            other => panic!("a watch command line: {other:?}"),
        }
    }

    #[test]
    fn the_paid_routes_go_to_a_lane_and_come_back_as_they_were() {
        for route in [
            Route::Decide { issue: 41 },
            Route::Rules { issue: 86 },
            Route::ContractChange { issue: 70 },
            Route::PrFix {
                pr: "85".to_string(),
            },
            Route::PrReview {
                pr: "77".to_string(),
                base: "milestone/17-x".to_string(),
            },
        ] {
            let (kind, number, base) = lane_work(&route).expect("a lane's");
            assert_eq!(lane_route(kind, number, &base), route);
        }
        assert_eq!(lane_work(&Route::MergeMainAgent { milestone: 17 }), None);
        assert_eq!(
            lane_work(&Route::PrFix {
                pr: "x".to_string()
            }),
            None
        );
    }

    #[test]
    fn a_lane_s_command_carries_the_watch_s_settings_and_its_own_checkout() {
        let mut args = watch();
        args.no_human_gate = true;
        let command = lane_command(
            &args,
            Path::new("/f"),
            Path::new("/bin/harness"),
            "pr-review",
            77,
            "milestone/17-x",
            2,
        );
        let said: Vec<String> = command
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let joined = said.join(" ");
        assert!(
            joined.starts_with("route pr-review 77 --route-workspace router-lane-2"),
            "{joined}"
        );
        assert!(joined.contains("--base milestone/17-x"), "{joined}");
        assert!(joined.contains("--no-human-gate"), "{joined}");
        assert!(!joined.contains("--dry-run"), "{joined}");
        let bare = lane_command(
            &watch(),
            Path::new("/f"),
            Path::new("/bin/harness"),
            "decide",
            41,
            "",
            0,
        );
        assert!(
            !bare.as_std().get_args().any(|a| a == "--base"),
            "no base for a decision"
        );
    }

    #[test]
    fn a_lane_s_checkout_is_its_own_and_the_router_s_otherwise() {
        let mut args = watch();
        assert_eq!(route_workspace(&args), None);
        assert_eq!(shared_checkout(&args).workspace, None);
        args.route_workspace = "router-lane-1".to_string();
        assert_eq!(route_workspace(&args), Some("router-lane-1"));
        assert_eq!(shared_checkout(&args).workspace, Some("router-lane-1"));
    }

    #[test]
    fn every_paid_route_has_a_lane_kind_and_the_free_ones_none() {
        for route in [
            Route::Split { milestone: 19 },
            Route::Refinement { issue: 6 },
            Route::TechRefinement { issue: 7 },
            Route::Planner { roadmap: 4 },
            Route::Arbiter { issue: 9 },
            Route::TestDebt { issue: 8 },
        ] {
            let (kind, number, base) = lane_work(&route).expect("a lane's");
            assert!(base.is_empty(), "base should be empty");
            assert_eq!(lane_route(kind, number, &base), route);
        }
        for free in [
            Route::MergeIntoMilestone {
                task: 1,
                pr: "2".to_string(),
                base: "milestone/3-x".to_string(),
            },
            Route::DevLoop { milestone: 3 },
            Route::DevTask {
                milestone: 3,
                task: 4,
            },
            Route::Nothing,
        ] {
            assert_eq!(lane_work(&free), None, "{free:?}");
        }
    }
}

#[cfg(test)]
mod merge_route_tests {
    use super::*;
    use crate::testing::NoGitHub;

    #[tokio::test]
    async fn a_merge_the_board_refuses_is_the_route_s_failure_not_a_quiet_tick() {
        // The task cannot even be read: the halt comes back under the route's
        // name, for the tick to tell — never swallowed as a tick that merged
        // nothing and said nothing.
        let failed = run_milestone_merge(
            &NoGitHub,
            Path::new("/nonexistent"),
            64,
            "68".to_string(),
            "milestone/52-socle".to_string(),
            &Logbook::null(),
        )
        .await;
        let Some((route, halt)) = failed else {
            panic!("a refused merge is the route's failure");
        };
        assert_eq!(route, "milestone_merge");
        assert!(
            halt.reason().contains("no call should leave this test"),
            "{}",
            halt.reason()
        );
    }
}
