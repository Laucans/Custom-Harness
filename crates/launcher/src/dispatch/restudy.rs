//! Launcher wiring for the `restudy` workflow.
//!
//! **Its lane's own checkout, on the milestone's branch**: the session
//! builds and runs the product to probe it, which the shared read-only
//! checkout cannot host beside the others, and the slices being built live
//! on the milestone's branch, not on the integration branch. Read-only all
//! the same: mounted without `force_reset`, so a commit it should never
//! have made stops the next mount and names itself (and its gate fails the
//! round first, `TheCheckoutIsUntouched`).

use std::path::Path;
use std::rc::Rc;

use harness_adapters::shell::git::GitCli;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::restudy::config::Config;
use harness_workflows::restudy::data::state::StudyState;
use harness_workflows::restudy::ports::Ports;
use harness_workflows::restudy::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// The id of the checkout a re-study works in when no lane names one.
pub const RESTUDY_WORKSPACE_ID: &str = "router-restudy";

/// Re-studies one task: mounts a checkout of its milestone's branch, runs
/// the workflow, unmounts.
///
/// # Errors
/// Whatever the workflow propagates — a precheck refusal, a session
/// failure, a quota exhausted — and a checkout that cannot be mounted.
pub async fn run(
    task: u64,
    here: &Path,
    checkout: &shared::Checkout<'_>,
    permission_mode: &str,
    dry_run: bool,
) -> Outcome<Verdict> {
    let run_id = spending::run_id_for(task);
    let sink = Rc::new(
        Both::new(
            &Workspace::new(here)
                .log_dir("restudy")
                .join(&run_id)
                .join("run.log"),
        )
        .map_err(|e| Halt::Failed(format!("log file cannot be opened: {e}")))?,
    );
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);

    let own = shared::Checkout {
        target_repo_url: checkout.target_repo_url,
        branch: checkout.branch,
        workspace: Some(checkout.workspace.unwrap_or(RESTUDY_WORKSPACE_ID)),
    };
    let mount = shared::mount_checkout(here, &own, &run_id, dry_run, &log).await?;
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
        git: Rc::new(GitCli::new(workspace.root())),
        sessions: Rc::clone(&built.sessions),
        spending: Rc::clone(&built.spending),
        locks: Rc::clone(&built.locks),
    };
    let config = Config {
        restudy_dir: workspace.state_root().join(".llocal/restudy-locks"),
        // A walk of every rule against the code and the running server:
        // the strongest model, once per streak at most.
        restudy: SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        },
        prompts: shared::prompts(workspace, &log, &shared::trace_dir(sink.path()))?,
    };

    let workflow = run::build(
        &ports,
        &config,
        &run::Request { task },
        Gate::empty("tooling"),
    );
    let mut ctx = Context::new(
        Settings {
            dry_run,
            stages: String::new(),
        },
        StudyState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::release_checkout(&mount, &log).await;
    verdict
}
