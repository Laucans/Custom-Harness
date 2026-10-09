//! Launcher wiring for the `data_diagnosis` workflow: one paid Opus session
//! in the snapshot clone, where the diverging version is checked out on the
//! archive branch and the code is read through `git show`.
//!
//! Reached by the router right after `data_snapshot` archived a version of
//! `main` or the integration branch whose two sides disagree.

use std::path::Path;
use std::rc::Rc;

use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome};
use harness_core::execution::{Context, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::data_diagnosis::config::Config;
use harness_workflows::data_diagnosis::data::diagnosis::Diagnosis;
use harness_workflows::data_diagnosis::ports::Ports;
use harness_workflows::data_diagnosis::run::{self, Request};

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// Diagnoses one diverging version.
///
/// # Errors
/// The log could not be opened, or the session failed — a quota, a reply
/// that is not a diagnosis.
pub async fn run(
    here: &Path,
    clone: &Path,
    request: Request,
    permission_mode: &str,
) -> Outcome<Diagnosis> {
    let run_id = spending::run_id();
    let workspace = Workspace::new(here).at(clone);
    let sink = Rc::new(
        Both::new(
            &workspace
                .log_dir("data-diagnosis")
                .join(&run_id)
                .join("run.log"),
        )
        .map_err(|e| Halt::Failed(format!("log file cannot be opened: {e}")))?,
    );
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);
    let built = shared::adapters(
        &workspace,
        &run_id,
        permission_mode,
        false,
        &log,
        Some(shared::trace_dir(sink.path())),
    )
    .await;
    let ports = Ports {
        sessions: Rc::clone(&built.sessions),
        spending: Rc::clone(&built.spending),
    };
    // Reading a schema's history and judging which side lies is the kind of
    // work the strongest model is for; it runs only when a version diverges.
    let config = Config {
        spec: SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        },
    };
    let workflow = run::build(&ports, &config);
    let mut ctx = Context::new(
        Settings {
            dry_run: false,
            stages: String::new(),
        },
        request.state(),
        log,
    );
    Guarded::execute(&workflow, &mut ctx).await?;
    ctx.state
        .diagnosis
        .ok_or_else(|| Halt::Failed("the session gave no diagnosis".to_string()))
}
