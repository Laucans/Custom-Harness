//! Launcher wiring for the `decide` workflow.
//!
//! Against the shared read-only checkout (`shared::mount_checkout`), like
//! the arbiter: the session reads the repository and the design dossier to
//! decide, and never commits — what it carries out it does on GitHub.

use std::path::Path;
use std::rc::Rc;

use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::decide::config::Config;
use harness_workflows::decide::data::state::DecideState;
use harness_workflows::decide::ports::Ports;
use harness_workflows::decide::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// Takes the decision one issue asks for: mounts the checkout, runs the
/// workflow, unmounts.
///
/// # Errors
/// Whatever the workflow propagates — a precheck refusal, a session
/// failure, a quota exhausted.
pub async fn run(
    issue: u64,
    here: &Path,
    checkout: &shared::Checkout<'_>,
    permission_mode: &str,
    dry_run: bool,
) -> Outcome<Verdict> {
    let run_id = spending::run_id_for(issue);
    let sink = Rc::new(
        Both::new(
            &Workspace::new(here)
                .log_dir("decide")
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
        decide_dir: workspace.state_root().join(".llocal/decide-locks"),
        // The decisions a human used to take: the strongest model, one
        // short session, read after the fact.
        decide: SessionSpec {
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
        DecideState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::release_checkout(&mount, &log).await;
    verdict
}
