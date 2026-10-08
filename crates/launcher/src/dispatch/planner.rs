//! Launcher wiring for the `planner` workflow.
//!
//! The second place (besides `dev_loop.rs`) that builds concrete adapters —
//! against the shared read-only checkout (`shared::mount_shared`) rather
//! than an exclusive one, since this workflow never commits.

use std::path::Path;
use std::rc::Rc;

use harness_core::adapters::shell::git::GitCli;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::common::explore;
use harness_workflows::planner::config::Config;
use harness_workflows::planner::data::state::PlannerState;
use harness_workflows::planner::ports::Ports;
use harness_workflows::planner::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// Plans one roadmap item: mounts the shared checkout, runs the workflow,
/// unmounts.
///
/// # Errors
/// Whatever the workflow propagates — a precheck refusal, a session
/// failure, a quota exhausted.
pub async fn run(
    roadmap: u64,
    here: &Path,
    target_repo_url: &str,
    branch: &str,
    permission_mode: &str,
    dry_run: bool,
) -> Outcome<Verdict> {
    let run_id = spending::run_id_for(roadmap);
    let sink = Rc::new(
        Both::new(
            &Workspace::new(here)
                .log_dir("planner")
                .join(&run_id)
                .join("run.log"),
        )
        .map_err(|e| {
            harness_core::domain::Halt::Failed(format!("log file cannot be opened: {e}"))
        })?,
    );
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);

    let mount = shared::mount_shared(here, target_repo_url, branch, &run_id, dry_run, &log).await?;
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

    let repo = built.gh.repo().await?;
    let grill_dir = workspace.state_root().join(".llocal/grill").join(&repo);
    let planner_dir = workspace.state_root().join(".llocal/planner-locks");
    let artifacts_dir = planner_dir.join(roadmap.to_string());

    let ports = Ports {
        gh: Rc::clone(&built.gh),
        sessions: Rc::clone(&built.sessions),
        spending: Rc::clone(&built.spending),
        locks: Rc::clone(&built.locks),
        disk: Rc::clone(&built.disk),
    };
    let config = Config {
        planner_dir,
        grill_dir,
        artifacts_dir: artifacts_dir.clone(),
        explore: false,
        plan: SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        },
    };
    let explore_ports = explore::Ports {
        repo: Rc::new(GitCli::new(workspace.root())),
        disk: Rc::clone(&built.disk),
        sessions: Rc::clone(&built.sessions),
        spending: Rc::clone(&built.spending),
    };
    let explore_config = explore::Config {
        root: workspace.root().to_path_buf(),
        explore: false,
        spec: SessionSpec {
            model: "sonnet".to_string(),
            effort: "high".to_string(),
        },
        artifacts_dir,
    };

    let workflow = run::build(
        &ports,
        &config,
        &explore_ports,
        &explore_config,
        &run::Request { roadmap },
        Gate::empty("outillage"),
    );
    let mut ctx = Context::new(
        Settings {
            dry_run,
            stages: String::new(),
        },
        PlannerState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::unmount_shared(&mount, &log).await;
    verdict
}
