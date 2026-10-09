//! Launcher wiring for `data_model_draft`: a first `data/model.json` for a
//! branch, drafted by a paid session in a clone of its own
//! (`.llocal/data-model-draft/`) and proposed as a pull request.
//!
//! Reached by `harness data-model-draft --branch <b>`, which the view's data
//! pane starts when a branch has no model.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use harness_adapters::shell::disk::RealDisk;
use harness_adapters::shell::git::GitRepos;
use harness_adapters::shell::github::GhCli;
use harness_adapters::shell::process::Processes;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome, Slug, Verdict};
use harness_core::execution::{Context, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::data_model_draft::config::Config;
use harness_workflows::data_model_draft::data::state::DraftState;
use harness_workflows::data_model_draft::ports::Ports;
use harness_workflows::data_model_draft::run;
use harness_workflows::data_snapshot::action::capture;
use harness_workflows::data_snapshot::config::{ARCHIVE_BRANCH, Config as SnapshotConfig, SCRIPT};

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// The longest the dump may take.
const TIMEOUT: Duration = Duration::from_secs(300);

/// Drafts `data/model.json` for one branch and opens its pull request.
///
/// # Errors
/// The target cannot be named, the journal cannot be opened, or a stage
/// halted: the branch already has a model, its dump printed nothing, the
/// session failed, `git` or GitHub refused.
pub async fn run(args: &crate::cli::DataModelDraftArgs, here: &Path) -> Outcome<Verdict> {
    let slug = Slug::parse(&args.target_repo_url).ok_or_else(|| {
        Halt::Failed(format!(
            "{:?} is not a repository the draft can clone: set TARGET_REPO_URL",
            args.target_repo_url
        ))
    })?;
    let workdir = here.join(".llocal").join("data-model-draft");
    let clone = capture::clone_dir(&SnapshotConfig {
        slug: slug.clone(),
        workdir: workdir.clone(),
        archive_branch: ARCHIVE_BRANCH.to_string(),
        script: SCRIPT.to_string(),
        timeout: TIMEOUT,
    });
    let run_id = spending::run_id();
    let workspace = Workspace::new(here).at(&clone);
    let sink = Rc::new(
        Both::new(
            &workspace
                .log_dir("data-model-draft")
                .join(&run_id)
                .join("run.log"),
        )
        .map_err(|e| Halt::Failed(format!("log file cannot be opened: {e}")))?,
    );
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);
    // Where this run's journal is, for whoever follows its output: the
    // view's build log reads the session's own log from there, live.
    if let Some(dir) = sink.path().parent() {
        log.say(&format!("journal: {}", dir.display()));
    }

    let built = shared::adapters(
        &workspace,
        &run_id,
        &args.permission_mode,
        false,
        &log,
        Some(shared::trace_dir(sink.path())),
    )
    .await;
    let ports = Ports {
        gh: Rc::new(GhCli::for_slug(&slug)) as Rc<dyn GitHub>,
        repos: Rc::new(GitRepos),
        disk: Rc::new(RealDisk),
        process: Rc::new(Processes),
        sessions: Rc::clone(&built.sessions),
        spending: Rc::clone(&built.spending),
        now: spending::now,
    };
    // Reading a codebase to say what every field is for is the strongest
    // model's work; it runs once per branch, on a human's click.
    let config = Config {
        slug,
        base: args.branch.clone(),
        workdir,
        script: SCRIPT.to_string(),
        timeout: TIMEOUT,
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
        DraftState::default(),
        log,
    );
    Guarded::execute(&workflow, &mut ctx).await
}
