//! What `planner`, `split`, `refinement` and `pr_review` share: a read-only
//! checkout, refreshed only when one of them is actually triggered, and the
//! adapters built against it.
//!
//! **Never `dev_loop`'s own workspace.** The four share one checkout among
//! themselves, distinct from `dev_loop`'s exclusive one (a different `Wanted`
//! id) — none of the four ever commits, so nothing here can conflict with
//! a `dev_loop` run in progress.
//!
//! `pr_fix` is the exception that proves the rule: it *does* commit, so it
//! gets a third workspace of its own ([`mount_named`], a different id again)
//! rather than leaving commits in a checkout the read-only four would then
//! refuse to reuse.

use std::path::Path;
use std::rc::Rc;

use harness_core::adapters::agent::claude_cli::ClaudeCliFactory;
use harness_core::adapters::agent::rehearsal::Rehearsal;
use harness_core::adapters::shell::disk::RealDisk;
use harness_core::adapters::shell::git::GitRepos;
use harness_core::adapters::shell::github::GhCli;
use harness_core::adapters::store::lock::DirLocks;
use harness_core::domain::Outcome;
use harness_core::domain::workspace::{Strategy, Wanted, Workspace};
use harness_core::execution::provisioning::{Mount, Provisioner, Run};
use harness_core::ports::agent::SessionFactory;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repos;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;
use harness_core::ports::store::spending::Spending;
use harness_core::traces::Logbook;

use crate::adapters::spending::{self, LedgerSpending};

/// The fixed id of the checkout `planner`/`split`/`refinement` share —
/// distinct from `dev_loop`'s own, which is named after the repo by default.
const SHARED_WORKSPACE_ID: &str = "router-readonly";

/// Which checkout a router-dispatched workflow reads the repository from.
pub struct Checkout<'a> {
    /// The target repository.
    pub target_repo_url: &'a str,
    /// The branch the checkout sits on.
    pub branch: &'a str,
    /// A checkout of its own, by id — a lane of a parallel `watch`, so two
    /// children never run `git` in the same clone at once. `None`: the
    /// shared read-only checkout.
    pub workspace: Option<&'a str>,
}

/// Mounts the checkout `checkout` names: its own when it has an id, the
/// shared read-only one otherwise.
///
/// # Errors
/// Whatever [`mount_named`] propagates.
pub async fn mount_checkout(
    here: &Path,
    checkout: &Checkout<'_>,
    run_id: &str,
    dry_run: bool,
    log: &Logbook,
) -> Outcome<Mount> {
    mount_named(
        here,
        checkout.target_repo_url,
        checkout.branch,
        checkout.workspace.unwrap_or(SHARED_WORKSPACE_ID),
        false,
        run_id,
        dry_run,
        log,
    )
    .await
}

/// Mounts, or refreshes, the shared read-only checkout.
///
/// # Errors
/// Whatever [`Provisioner::mount`] propagates — a reused workspace carrying
/// local work it did not expect, a clone that failed.
pub async fn mount_shared(
    here: &Path,
    target_repo_url: &str,
    branch: &str,
    run_id: &str,
    dry_run: bool,
    log: &Logbook,
) -> Outcome<Mount> {
    mount_named(
        here,
        target_repo_url,
        branch,
        SHARED_WORKSPACE_ID,
        false,
        run_id,
        dry_run,
        log,
    )
    .await
}

/// Mounts, or refreshes, a checkout under a name the **harness** chose.
///
/// `force_reset` is what separates a read-only tenant from one that commits:
/// a workflow that pushes leaves commits behind when it fails, and the next
/// run of it wants them gone rather than to be told the workspace is dirty.
/// A read-only tenant passes `false`, so an unexpected local change still
/// stops the run and names itself.
///
/// # Errors
/// Whatever [`Provisioner::mount`] propagates — a workspace cloned from
/// another repository, a clone that failed.
#[allow(clippy::too_many_arguments)] // Every one is a distinct fact of the
// mount, and the two callers each pass all of them; a struct here would be
// a parameter object read once and never stored.
pub async fn mount_named(
    here: &Path,
    target_repo_url: &str,
    branch: &str,
    id: &str,
    force_reset: bool,
    run_id: &str,
    dry_run: bool,
    log: &Logbook,
) -> Outcome<Mount> {
    let source = Workspace::new(here);
    let lender: Rc<dyn Disk> = Rc::new(RealDisk);
    let repos: Rc<dyn Repos> = Rc::new(GitRepos);
    let wanted = Wanted {
        strategy: Strategy::Permanent,
        url: target_repo_url.to_string(),
        id: id.to_string(),
        base: String::new(),
        keep: false,
        force_reset,
        // The harness chose this name, not a human: nobody could have cloned
        // it beforehand, so refusing a missing one would make the whole
        // router path unreachable on a fresh machine.
        create_if_missing: true,
    };
    let mount = Provisioner {
        repos,
        disk: Rc::clone(&lender),
    }
    .mount(
        &Run {
            source: &source,
            wanted: &wanted,
            branch,
            run_id,
            dry_run,
        },
        log,
    )
    .await?;
    // After the mount, every time: the clone's own `clean` runs inside it,
    // and a copy refreshed each run is a copy that cannot drift from the
    // harness that owns it.
    if mount.mounted() {
        crate::dispatch::skills::lend(here, mount.workspace.root(), lender.as_ref(), log)?;
    }
    Ok(mount)
}

/// Unmounts a checkout mounted here — a no-op for a named `Permanent`
/// workspace, called anyway for symmetry with `dev_loop`'s own "unmount
/// happens no matter what".
///
/// **And drops what the run left on it.** These checkouts are read-only:
/// nothing on them is ever pushed, so a branch a session made there — a pull
/// request fetched to be reviewed — is scratch. Left behind, it read as work
/// origin does not have, and the next mount refused the whole checkout.
pub async fn unmount_shared(mount: &Mount, log: &Logbook) {
    let repos: Rc<dyn Repos> = Rc::new(GitRepos);
    if let Some(path) = &mount.path
        && !mount.branch.is_empty()
    {
        let git = repos.at(path);
        if git
            .checkout(&mount.branch, true)
            .await
            .is_ok_and(|ran| ran.ok())
        {
            for branch in git.local_branches().await.unwrap_or_default() {
                if branch != mount.branch
                    && git.delete_branch(&branch).await.is_ok_and(|ran| ran.ok())
                {
                    log.say(&format!("workspace: dropped the scratch branch {branch}"));
                }
            }
        }
    }
    let disk: Rc<dyn Disk> = Rc::new(RealDisk);
    Provisioner { repos, disk }.unmount(mount, log).await;
}

/// The adapters every router-dispatched workflow needs, built against a
/// mounted workspace.
pub struct Adapters {
    /// What reads and writes issues.
    pub gh: Rc<dyn GitHub>,
    /// What opens a paid session — or runs it dry.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where a step's spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// The real disk.
    pub disk: Rc<dyn Disk>,
}

/// Builds the adapters for one run against this mounted workspace.
///
/// `unmount_shared` is the matching teardown, whichever mount built it.
/// `trace` is the folder where paid sessions append what they did, live — see
/// [`trace_dir`].
/// The run's own folder, derived from where its `run.log` is.
///
/// Derived rather than passed a second time, so the harness's journal
/// (`run.log`) and what its sessions did (`session.log`, `stream.jsonl`,
/// `prompts.md`) cannot land in different places.
#[must_use]
pub fn trace_dir(run_log: &std::path::Path) -> std::path::PathBuf {
    run_log.parent().map_or_else(
        || std::path::PathBuf::from("."),
        std::path::Path::to_path_buf,
    )
}

pub async fn adapters(
    workspace: &Workspace,
    run_id: &str,
    permission_mode: &str,
    dry_run: bool,
    log: &Logbook,
    trace: Option<std::path::PathBuf>,
) -> Adapters {
    let gh: Rc<dyn GitHub> = Rc::new(GhCli::new(workspace.root()));
    let sessions: Rc<dyn SessionFactory> = if dry_run {
        Rc::new(Rehearsal::new(log.clone()))
    } else {
        Rc::new(ClaudeCliFactory::new(
            workspace.root(),
            permission_mode,
            trace,
        ))
    };
    let spend: Rc<dyn Spending> = Rc::new(LedgerSpending::new(
        &workspace.ledger(),
        run_id,
        &spending::hostname().await,
    ));
    Adapters {
        gh,
        sessions,
        spending: spend,
        locks: Rc::new(DirLocks),
        disk: Rc::new(RealDisk),
    }
}
