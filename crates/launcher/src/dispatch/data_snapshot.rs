//! Launcher wiring for the `data_snapshot` deterministic command: the
//! product's data model archived, version by version, on the target's
//! `harness/data-model` branch.
//!
//! Reached by the router on every tick that read the board: a quiet pass is
//! one `gh` read per branch, and nothing is cloned.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use harness_adapters::shell::disk::RealDisk;
use harness_adapters::shell::git::GitRepos;
use harness_adapters::shell::github::GhCli;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome, Slug};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook, Sink, Verbosity};
use harness_workflows::data_diagnosis::run::Request as DiagnosisRequest;
use harness_workflows::data_snapshot::action::capture;
use harness_workflows::data_snapshot::config::{ARCHIVE_BRANCH, Config, SCRIPT};
use harness_workflows::data_snapshot::data::report::{Report, Snapped};
use harness_workflows::data_snapshot::ports::Ports;
use harness_workflows::data_snapshot::run as workflow;

use crate::adapters::sink::Both;
use crate::adapters::spending;

/// The longest a dump may take: a scratch database, every migration, one
/// `pg_dump`.
const TIMEOUT: Duration = Duration::from_secs(300);

/// Archives the data model of every branch whose head moved.
///
/// # Errors
/// Whatever the command propagates: GitHub, `git` or the disk unreachable.
pub async fn run(
    gh: Rc<dyn GitHub>,
    slug: &Slug,
    here: &Path,
    branches: Vec<String>,
) -> Outcome<Report> {
    let ports = Ports {
        gh,
        repos: Rc::new(GitRepos),
        disk: Rc::new(RealDisk),
        process: Rc::new(crate::toolchain::processes()),
        now: spending::now,
    };
    let config = config(here, slug);
    workflow::build(&ports, &config, workflow::Request { branches })
        .run()
        .await
}

/// The snapshot's settings: its clone under `.llocal/data-snapshot/`.
#[must_use]
pub fn config(here: &Path, slug: &Slug) -> Config {
    Config {
        slug: slug.clone(),
        workdir: here.join(".llocal").join("data-snapshot"),
        archive_branch: ARCHIVE_BRANCH.to_string(),
        script: SCRIPT.to_string(),
        timeout: TIMEOUT,
    }
}

/// The snapshot clone — where the archive branch is left checked out, and
/// where a diagnosis reads it.
#[must_use]
pub fn clone_dir(here: &Path, slug: &Slug) -> std::path::PathBuf {
    capture::clone_dir(&config(here, slug))
}

/// The release branch: what a human merges the integration branch into.
pub const RELEASE_BRANCH: &str = "main";

/// What a snapshot pass needs beyond the GitHub it reads.
pub struct Pass<'a> {
    /// The harness checkout.
    pub here: &'a Path,
    /// The target repository.
    pub slug: &'a Slug,
    /// The integration branch: with `main`, the one whose versions are checked.
    pub integration: &'a str,
    /// Passed to the diagnosis's `claude`.
    pub permission_mode: &'a str,
}

/// Archives the data model of `branches` whose head moved, and checks the
/// versions of `main` and the integration branch: agreeing clears their
/// alert, diverging sends Opus to say why. Everything it learns is an event
/// in `log`; nothing here fails its caller.
pub async fn snapshot_and_check(
    pass: &Pass<'_>,
    gh: Rc<dyn GitHub>,
    branches: Vec<String>,
    log: &Logbook,
) {
    let report = match run(gh, pass.slug, pass.here, branches).await {
        Ok(report) => report,
        Err(halt) => {
            log.event(&Event::DataSnapshotFailed {
                branch: "every branch".to_string(),
                sha: String::new(),
                reason: halt.reason().to_string(),
            });
            return;
        }
    };
    for snapped in report.snapped {
        match snapped {
            Snapped::Archived {
                branch,
                sha,
                dir,
                compared,
                divergences,
            } => {
                log.event(&Event::DataSnapshotTaken {
                    branch: branch.clone(),
                    sha: sha.clone(),
                    dir: dir.clone(),
                });
                // Only what a human released or the agents integrated is
                // checked: a milestone's branch is work in progress.
                if compared && (branch == RELEASE_BRANCH || branch == pass.integration) {
                    check(pass, (branch, sha), dir, divergences, log).await;
                }
            }
            Snapped::Unchanged { branch, sha } => log.event(&Event::DataSnapshotTaken {
                branch,
                sha,
                dir: String::new(),
            }),
            Snapped::Failed {
                branch,
                sha,
                reason,
            } => {
                log.event(&Event::DataSnapshotFailed {
                    branch,
                    sha,
                    reason,
                });
            }
        }
    }
}

/// A version whose two sides were compared: agreeing clears the branch's
/// alert, diverging sends Opus to say why — and either way the human hears
/// of it as a notification.
async fn check(
    pass: &Pass<'_>,
    head: (String, String),
    dir: String,
    divergences: Vec<String>,
    log: &Logbook,
) {
    let (branch, sha) = head.clone();
    if divergences.is_empty() {
        log.event(&Event::DataModelAgreed { branch, sha });
        return;
    }
    let clone = clone_dir(pass.here, pass.slug);
    let request = DiagnosisRequest {
        head,
        dir: dir.clone(),
        divergences: divergences.clone(),
    };
    let (diagnosis, actions, failure) = match crate::dispatch::data_diagnosis::run(
        pass.here,
        &clone,
        request,
        pass.permission_mode,
    )
    .await
    {
        Ok(d) => (d.markdown(), d.actions, String::new()),
        Err(halt) => (String::new(), Vec::new(), halt.reason().to_string()),
    };
    log.event(&Event::DataModelDiverged {
        branch,
        sha,
        dir,
        divergences,
        diagnosis,
        actions,
        failure,
    });
}

/// `harness data-snapshot`: one pass, on the branches the human named, its
/// journal under `.llocal/logs/data-snapshot/<run>/run.log` and its events
/// in the store — so the view hears of it as it hears of the watch's.
///
/// # Errors
/// The target is not a repository the harness can name, or the journal
/// cannot be opened.
pub async fn by_hand(args: &crate::cli::DataSnapshotArgs, here: &Path) -> Outcome<()> {
    let slug = Slug::parse(&args.target_repo_url).ok_or_else(|| {
        Halt::Failed(format!(
            "{:?} is not a repository the snapshot can clone: set TARGET_REPO_URL",
            args.target_repo_url
        ))
    })?;
    let journal = Workspace::new(here)
        .log_dir("data-snapshot")
        .join(spending::run_id())
        .join("run.log");
    let sink = Rc::new(
        Both::new(&journal)
            .map_err(|e| Halt::Failed(format!("the journal cannot be opened: {e}")))?,
    );
    let log = Logbook::new(sink as Rc<dyn Sink>, Verbosity::Normal);
    let branches = if args.branches.is_empty() {
        vec![RELEASE_BRANCH.to_string(), args.integration_branch.clone()]
    } else {
        args.branches.clone()
    };
    log.say(&format!(
        "data snapshot: {} on {}",
        branches.join(", "),
        args.target_repo_url
    ));
    let pass = Pass {
        here,
        slug: &slug,
        integration: &args.integration_branch,
        permission_mode: &args.permission_mode,
    };
    let gh: Rc<dyn GitHub> = Rc::new(GhCli::for_slug(&slug));
    snapshot_and_check(&pass, gh, branches, &log).await;
    Ok(())
}
