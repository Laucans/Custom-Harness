//! Launcher wiring for the `pr_fix` workflow.
//!
//! **The one router-dispatched workflow with a writable checkout of its
//! own.** A repair commits and pushes, so it cannot share the read-only
//! checkout the other four use: a failed attempt would leave commits there,
//! and the next `planner` or `split` run would refuse to reuse a workspace
//! carrying local work. It gets a third id, and `force_reset: true` — what
//! a previous attempt left behind is never what this one wants to start
//! from.
//!
//! It is also the only one mounted on a branch read from GitHub rather than
//! passed in: the PR's own head branch, which is where the repair has to
//! land.

use std::path::Path;
use std::rc::Rc;

use harness_core::adapters::shell::github::GhCli;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome, Slug, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::pr_fix::config::Config;
use harness_workflows::pr_fix::data::state::FixState;
use harness_workflows::pr_fix::ports::Ports;
use harness_workflows::pr_fix::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// The id of the checkout a repair works in — its own, never shared.
const FIX_WORKSPACE_ID: &str = "router-prfix";

/// Attempts one repair on a red PR: reads which branch it lives on, mounts
/// a writable checkout there, runs the workflow, unmounts.
///
/// # Errors
/// Whatever the workflow propagates — a precheck refusal, a session
/// failure, a quota exhausted — plus a PR whose head branch cannot be read
/// at all, since there would be nowhere to mount.
pub async fn run(
    pr_ref: &str,
    here: &Path,
    target_repo_url: &str,
    permission_mode: &str,
    dry_run: bool,
) -> Outcome<Verdict> {
    let run_id = spending::run_id();
    let sink = Rc::new(
        Both::new(
            &Workspace::new(here)
                .log_dir("pr-fix")
                .join(&run_id)
                .join("run.log"),
        )
        .map_err(|e| {
            harness_core::domain::Halt::Failed(format!("log file cannot be opened: {e}"))
        })?,
    );
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);

    // Read before mounting: the checkout has to land on the PR's own branch,
    // and only GitHub knows which one that is. This read is deliberately not
    // reused from the router's snapshot — a route carries a decision, not a
    // payload, and the workflow's precheck re-reads the PR anyway.
    let head = head_branch(here, target_repo_url, pr_ref).await?;
    let mount = shared::mount_named(
        here,
        target_repo_url,
        &head,
        FIX_WORKSPACE_ID,
        true,
        &run_id,
        dry_run,
        &log,
    )
    .await?;
    let workspace = &mount.workspace;
    let built = shared::adapters(
        workspace,
        &run_id,
        permission_mode,
        dry_run,
        &log,
        Some(shared::trace_dir(sink.path())),
    )
    .await;

    let ports = Ports {
        gh: Rc::clone(&built.gh),
        sessions: Rc::clone(&built.sessions),
        spending: Rc::clone(&built.spending),
        locks: Rc::clone(&built.locks),
    };
    let config = Config {
        fix_dir: workspace.state_root().join(".llocal/pr-fix-locks"),
        // Diagnosing why CI broke from logs and a diff is the kind of work
        // the better model earns its cost on — a wrong fix pushed to a PR is
        // more expensive than the session that found the right one.
        fix: SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        },
    };

    let workflow = run::build(
        &ports,
        &config,
        &run::Request {
            pr_ref: pr_ref.to_string(),
        },
        Gate::empty("tooling"),
    );
    let mut ctx = Context::new(
        Settings {
            dry_run,
            stages: String::new(),
        },
        FixState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::unmount_shared(&mount, &log).await;
    verdict
}

/// The branch this PR carries its change on, read without a checkout.
async fn head_branch(here: &Path, target_repo_url: &str, pr_ref: &str) -> Outcome<String> {
    let gh: Rc<dyn GitHub> = if target_repo_url.is_empty() {
        Rc::new(GhCli::new(here))
    } else {
        let slug = Slug::parse(target_repo_url).ok_or_else(|| {
            Halt::Failed(format!(
                "{target_repo_url:?} is not a usable repository URL"
            ))
        })?;
        Rc::new(GhCli::for_slug(&slug))
    };
    let pr = gh.pr(pr_ref).await?;
    if pr.head.is_empty() {
        return Err(Halt::Failed(format!(
            "PR {pr_ref} reports no head branch — there is nowhere to mount a repair"
        )));
    }
    Ok(pr.head)
}
