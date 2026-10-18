//! Launcher wiring for the `contract_manager` workflow.
//!
//! Against the shared read-only checkout (`shared::mount_checkout`), like
//! `decide`: the session reads the repository — the manifest, its schema,
//! the fixture, the consumers — and never commits; what the workflow writes
//! it writes on GitHub, on the integration branch.

use std::path::Path;
use std::rc::Rc;

use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::contract_manager::config::Config;
use harness_workflows::contract_manager::data::state::ManagerState;
use harness_workflows::contract_manager::ports::Ports;
use harness_workflows::contract_manager::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// Evolves the contract one `harness:contract-change` issue asks for:
/// mounts the checkout, runs the workflow, unmounts.
///
/// # Errors
/// Whatever the workflow propagates — a precheck refusal, a session
/// failure, a quota exhausted.
pub async fn run(
    issue: u64,
    here: &Path,
    checkout: &shared::Checkout<'_>,
    base_branch: &str,
    permission_mode: &str,
    dry_run: bool,
) -> Outcome<Verdict> {
    let run_id = spending::run_id_for(issue);
    let sink = Rc::new(
        Both::new(
            &Workspace::new(here)
                .log_dir("contract-manager")
                .join(&run_id)
                .join("run.log"),
        )
        .map_err(|e| Halt::Failed(format!("log file cannot be opened: {e}")))?,
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
    };
    let config = Config {
        lock_dir: workspace.state_root().join(".llocal/contract-locks"),
        base_branch: base_branch.to_string(),
        // An interface every consumer builds against: the strongest model,
        // one short session.
        evolve: SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        },
        prompts: shared::prompts(workspace, &log, &shared::trace_dir(sink.path()))?,
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
        ManagerState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::unmount_shared(&mount, &log).await;
    verdict
}
