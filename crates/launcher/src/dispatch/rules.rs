//! Launcher wiring for the `rules` workflow.
//!
//! **A writable checkout of its own**, like `test_debt`: the sessions branch,
//! commit and push the rules and their tests, so they cannot share the
//! read-only checkout. Its own id, and `force_reset: true`: what a previous
//! attempt left behind is never what this one starts from.
//!
//! Mounted on the integration branch: the rules' pull request targets it.

use std::path::Path;
use std::rc::Rc;

use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::rules::config::Config;
use harness_workflows::rules::data::state::RulesState;
use harness_workflows::rules::ports::Ports;
use harness_workflows::rules::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// The id of the checkout a rules' task works in — its own, never shared.
const RULES_WORKSPACE_ID: &str = "router-rules";

/// Attempts what one rules' issue asks for: mounts a writable checkout on
/// the integration branch, runs the workflow, unmounts.
///
/// # Errors
/// Whatever the workflow propagates — an unreadable issue, a session
/// failure, a quota exhausted — and a checkout that cannot be mounted.
pub async fn run(
    issue: u64,
    here: &Path,
    target_repo_url: &str,
    branch: &str,
    permission_mode: &str,
    workspace: Option<&str>,
    dry_run: bool,
) -> Outcome<Verdict> {
    let run_id = spending::run_id_for(issue);
    let sink = Rc::new(
        Both::new(
            &Workspace::new(here)
                .log_dir("rules")
                .join(&run_id)
                .join("run.log"),
        )
        .map_err(|e| Halt::Failed(format!("log file cannot be opened: {e}")))?,
    );
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);

    let mount = shared::mount_named(
        here,
        target_repo_url,
        branch,
        workspace.unwrap_or(RULES_WORKSPACE_ID),
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
        prompts: shared::prompts(workspace, &log, &shared::trace_dir(sink.path()))?,
        rules_dir: workspace.state_root().join(".llocal/rules-locks"),
        branch: branch.to_string(),
        // Writing a test from a criterion is narrow work with its own
        // proof; judging it is the harder read, and the one that merges.
        write: SessionSpec {
            model: "sonnet".to_string(),
            effort: "high".to_string(),
        },
        fidelity: SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        },
    };

    let workflow = run::build(
        &ports,
        &config,
        &run::Request { issue },
        Gate::empty("tooling"),
    );
    let mut ctx = Context::new(
        Settings {
            dry_run,
            stages: String::new(),
        },
        RulesState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::unmount_shared(&mount, &log).await;
    verdict
}
