//! Launcher wiring for the `refinement` workflow.
//!
//! **This is the first launcher entry point `refinement` has ever had.**
//! Its `run::build` existed and was tested since the workflow was written,
//! but nothing in this crate called it — the router is what finally
//! triggers it on its own documented label (`harness:refinement`), rather
//! than only by hand.
//!
//! Against the shared read-only checkout (`shared::mount_shared`), like
//! `planner.rs` — `refinement` never commits either.

use std::path::Path;
use std::rc::Rc;

use harness_core::adapters::shell::git::GitCli;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::common::explore;
use harness_workflows::refinement::config::Config;
use harness_workflows::refinement::data::phase::Phase;
use harness_workflows::refinement::data::state::RefinementState;
use harness_workflows::refinement::ports::Ports;
use harness_workflows::refinement::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// A uniform model/effort for every refinement step this automated trigger
/// runs — a human tuning each section's cost/quality tradeoff is a later
/// refinement of this wiring, not a blocker for having one at all.
fn spec() -> SessionSpec {
    SessionSpec {
        model: "sonnet".to_string(),
        effort: "high".to_string(),
    }
}

/// Refines one issue's body: mounts the checkout, runs the workflow,
/// unmounts.
///
/// # Errors
/// Whatever the workflow propagates — a precheck refusal, a session
/// failure, a quota exhausted.
pub async fn run(
    phase: Phase,
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
                .log_dir("refinement")
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

    let refinement_dir = workspace.refinement_dir();
    let artifacts_dir = refinement_dir.join(issue.to_string());

    let ports = Ports {
        gh: Rc::clone(&built.gh),
        sessions: Rc::clone(&built.sessions),
        spending: Rc::clone(&built.spending),
        locks: Rc::clone(&built.locks),
        disk: Rc::clone(&built.disk),
    };
    let config = Config {
        refinement_dir,
        context: String::new(),
        explore: false,
        goal: spec(),
        technical: spec(),
        criteria: spec(),
        rules: spec(),
        plan: spec(),
        router: spec(),
        coherence: spec(),
        advice: spec(),
        artifacts_dir: artifacts_dir.clone(),
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
        spec: spec(),
        artifacts_dir,
    };

    let workflow = run::build(
        &ports,
        &config,
        &explore_ports,
        &explore_config,
        run::Request {
            phase,
            issue,
            context: String::new(),
            force: false,
        },
        Gate::empty("outillage"),
    );
    let mut ctx = Context::new(
        Settings {
            dry_run,
            stages: String::new(),
        },
        RefinementState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::unmount_shared(&mount, &log).await;
    verdict
}
