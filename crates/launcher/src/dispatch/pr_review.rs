//! Launcher wiring for the `pr_review` workflow.
//!
//! **This is the first launcher entry point `pr_review` has ever had.** Its
//! `run::build` existed and was tested since the workflow was written, but
//! nothing in this crate called it: its documented trigger was a hook on
//! `gh pr create`, which would need a public URL this harness does not
//! have. The router triggers it on a label instead (`harness:to-review`),
//! the same gesture every other stage of the flow is opened by.
//!
//! Against the shared read-only checkout (`shared::mount_shared`), like
//! `planner.rs` — a review posts a comment and never commits. The passes
//! read the change through `gh pr diff` and `/code-review <pr>`, so the
//! checkout only has to be a checkout of the right repository, not of the
//! PR's own branch.

use std::path::Path;
use std::rc::Rc;

use harness_core::adapters::store::review_ledger::ReviewLedger;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::ports::store::review::ReviewCosts;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::pr_review::config::Config;
use harness_workflows::pr_review::data::state::ReviewState;
use harness_workflows::pr_review::ports::Ports;
use harness_workflows::pr_review::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::dispatch::shared;

/// Reviews one PR: mounts the shared checkout, runs the workflow, unmounts.
///
/// `base` is **the branch the PR itself targets**, read by the router from
/// the PR rather than taken from `--branch`: a task PR targets its
/// milestone's branch, and judging it against a fixed integration branch
/// would skip every one of them as "targets the wrong branch".
///
/// # Errors
/// Whatever the workflow propagates — a precheck refusal, a session
/// failure, a quota exhausted.
pub async fn run(
    pr_ref: &str,
    base: &str,
    here: &Path,
    target_repo_url: &str,
    branch: &str,
    permission_mode: &str,
    dry_run: bool,
) -> Outcome<Verdict> {
    let run_id = spending::run_id();
    let sink = Rc::new(
        Both::new(
            &Workspace::new(here)
                .log_dir("pr-review")
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

    let ports = Ports {
        gh: Rc::clone(&built.gh),
        sessions: Rc::clone(&built.sessions),
        spending: Rc::clone(&built.spending),
        // The only reader of the review ledger: what the footer of the posted
        // comment says this review cost.
        costs: Rc::new(ReviewLedger::new(&workspace.review_ledger())) as Rc<dyn ReviewCosts>,
        disk: Rc::clone(&built.disk),
        locks: Rc::clone(&built.locks),
        now: spending::now,
    };
    let config = Config {
        review_dir: workspace.review_dir(),
        level: "medium".to_string(),
        no_inline: false,
        // The hunt is the expensive half and the one worth the better model;
        // the summary pass is writing up what the hunt and the diff already
        // say.
        inline: SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        },
        brief: SessionSpec {
            model: "sonnet".to_string(),
            effort: "high".to_string(),
        },
    };

    let workflow = run::build(
        &ports,
        &config,
        run::Request {
            pr_ref: pr_ref.to_string(),
            base: base.to_string(),
            // The label asked for a review; the workflow's own skip rules
            // (draft, a test PR, one already reviewed) still apply, and
            // forcing past them would re-review what is already reviewed on
            // every poll.
            force: false,
        },
        Gate::empty("tooling"),
    );
    let mut ctx = Context::new(
        Settings {
            dry_run,
            stages: String::new(),
        },
        ReviewState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::unmount_shared(&mount, &log).await;
    verdict
}
