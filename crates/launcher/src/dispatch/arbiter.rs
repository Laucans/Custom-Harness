//! Launcher wiring for the `arbiter` workflow.
//!
//! Against the shared read-only checkout (`shared::mount_checkout`), like
//! `split.rs`: the arbiter reads the repository's manifests to know which
//! ids already exist, and never commits.

use std::path::Path;
use std::rc::Rc;

use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::arbiter::config::Config;
use harness_workflows::arbiter::data::state::ArbiterState;
use harness_workflows::arbiter::ports::Ports;
use harness_workflows::arbiter::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// Arbitrates one task's graph question: mounts the checkout, runs the
/// workflow, unmounts.
///
/// # Errors
/// Whatever the workflow propagates — a precheck refusal, a session
/// failure, a quota exhausted.
pub async fn run(
    issue: u64,
    force: bool,
    here: &Path,
    checkout: &shared::Checkout<'_>,
    permission_mode: &str,
    dry_run: bool,
) -> Outcome<Verdict> {
    let run_id = spending::run_id_for(issue);
    let sink = Rc::new(
        Both::new(
            &Workspace::new(here)
                .log_dir("arbiter")
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
        disk: Rc::clone(&built.disk),
    };
    let config = Config {
        arbiter_dir: workspace.state_root().join(".llocal/arbiter-locks"),
        root: workspace.root().to_path_buf(),
        // One short session over an index, never a SPEC: a mid model holds
        // it, and a verdict it is not sure of goes to a human anyway.
        arbitrate: SessionSpec {
            model: "sonnet".to_string(),
            effort: "high".to_string(),
        },
        prompts: shared::prompts(workspace, &log, &shared::trace_dir(sink.path()))?,
    };

    let workflow = run::build(
        &ports,
        &config,
        &run::Request { issue, force },
        Gate::empty("tooling"),
    );
    let mut ctx = Context::new(
        Settings {
            dry_run,
            stages: String::new(),
        },
        ArbiterState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::release_checkout(&mount, &log).await;
    verdict
}
