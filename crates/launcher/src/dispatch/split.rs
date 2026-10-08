//! Launcher wiring for the `split` workflow.
//!
//! Against the shared read-only checkout (`shared::mount_shared`) like
//! `planner.rs` — `split` never commits either. Unlike `planner`, it does
//! not wire the shared repository map: splitting a milestone into tasks
//! doesn't need it.

use std::path::Path;
use std::rc::Rc;

use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::split::config::Config;
use harness_workflows::split::data::state::SplitState;
use harness_workflows::split::ports::Ports;
use harness_workflows::split::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// Splits one milestone into tasks: mounts the checkout, runs the workflow,
/// unmounts.
///
/// # Errors
/// Whatever the workflow propagates — a precheck refusal, a session
/// failure, a quota exhausted.
pub async fn run(
    milestone: u64,
    here: &Path,
    checkout: &shared::Checkout<'_>,
    permission_mode: &str,
    dry_run: bool,
) -> Outcome<Verdict> {
    let run_id = spending::run_id_for(milestone);
    let branch = checkout.branch;
    let sink = Rc::new(
        Both::new(
            &Workspace::new(here)
                .log_dir("split")
                .join(&run_id)
                .join("run.log"),
        )
        .map_err(|e| {
            harness_core::domain::Halt::Failed(format!("log file cannot be opened: {e}"))
        })?,
    );
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);

    let mount = shared::mount_checkout(here, checkout, &run_id, dry_run, &log).await?;
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
        disk: Rc::clone(&built.disk),
    };
    let config = Config {
        split_dir: workspace.state_root().join(".llocal/split-locks"),
        base_branch: branch.to_string(),
        slice: SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        },
        root: workspace.root().to_path_buf(),
    };

    let workflow = run::build(
        &ports,
        &config,
        &run::Request { milestone },
        Gate::empty("outillage"),
    );
    let mut ctx = Context::new(
        Settings {
            dry_run,
            stages: String::new(),
        },
        SplitState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::unmount_shared(&mount, &log).await;
    verdict
}
