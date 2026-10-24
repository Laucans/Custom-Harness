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

use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use harness_adapters::agent::claude_cli::ClaudeCliFactory;
use harness_adapters::agent::rehearsal::Rehearsal;
use harness_adapters::shell::disk::RealDisk;
use harness_adapters::shell::git::GitRepos;
use harness_adapters::shell::github::GhCli;
use harness_adapters::store::lock::DirLocks;
use harness_adapters::store::prompt_book::{DraftFile, PromptFile, record_used};
use harness_core::domain::Outcome;
use harness_core::domain::prompt_book::PromptBook;
use harness_core::domain::workspace::{Strategy, Wanted, Workspace};
use harness_core::execution::provisioning::{Mount, Provisioner, Run};
use harness_core::ports::agent::SessionFactory;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repos;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;
use harness_core::ports::store::prompts::{DraftStore, PromptStore};
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
/// A read-only tenant passes `false`, so commits no remote has still stop
/// the run and name themselves — uncommitted leftovers do not (see
/// [`wanted`]).
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
    let mount = mount_bare(
        here,
        target_repo_url,
        branch,
        id,
        force_reset,
        run_id,
        dry_run,
        log,
    )
    .await?;
    // The run's start: its journal says the tools its processes and its
    // sessions get, the cargo the checkout's own pin names.
    crate::toolchain::at_run_start(here, mount.workspace.root(), log).await;
    Ok(mount)
}

/// [`mount_named`] without the run's start: the toolchain the watch took
/// stays the one every adapter gets — a catch-up's mount, inside the
/// watch's own tick, resolves the cargo its merged tree pins itself.
///
/// # Errors
/// Whatever [`Provisioner::mount`] propagates.
#[allow(clippy::too_many_arguments)] // the same facts as `mount_named`'s
pub async fn mount_bare(
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
    let wanted = wanted(target_repo_url, id, force_reset);
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

/// The checkout a router-dispatched workflow asks for: permanent, under the
/// name the harness chose, and **only the harness ever works in it**.
///
/// That last fact is `drop_uncommitted`: whatever a session leaves
/// uncommitted on it — a merge it never finished, files it never committed
/// — is a leftover, dropped at the next mount rather than halted on. A lane
/// that halts on a leftover halts for every job after it (`ForgeDesMondes`,
/// 23:09Z: a repair's conflicted merge on `router-lane-1` stopped the review
/// that followed there, and `router-lane-3` was in the same state). Commits
/// no remote has are not leftovers: they still stop the mount, named.
fn wanted(target_repo_url: &str, id: &str, force_reset: bool) -> Wanted {
    Wanted {
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
        drop_uncommitted: true,
    }
}

/// Gives a checkout mounted here back, whatever the run did on it —
/// called no matter what, like `dev_loop`'s own unmount.
///
/// [`Provisioner::release`] does the work: an unfinished merge aborted, the
/// branch put back at `origin`'s state, untracked files and scratch branches
/// gone — so the next tenant of the lane, read-only or not, finds it clean.
/// Commits no remote has are the exception: kept whole and said in the
/// journal (`Event::WorkKept`), never deleted; the next mount then stops and
/// names them.
pub async fn release_checkout(mount: &Mount, log: &Logbook) {
    let repos: Rc<dyn Repos> = Rc::new(GitRepos);
    let disk: Rc<dyn Disk> = Rc::new(RealDisk);
    Provisioner { repos, disk }.release(mount, log).await;
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
/// `release_checkout` is the matching teardown, whichever mount built it.
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
    adapters_caching(
        workspace,
        run_id,
        permission_mode,
        dry_run,
        log,
        trace,
        false,
    )
    .await
}

/// [`adapters`], whose fresh sessions write a five-minute prompt cache when
/// `short_cache`: for a workflow that never forks a session (a repair).
pub async fn adapters_caching(
    workspace: &Workspace,
    run_id: &str,
    permission_mode: &str,
    dry_run: bool,
    log: &Logbook,
    trace: Option<std::path::PathBuf>,
    short_cache: bool,
) -> Adapters {
    let gh: Rc<dyn GitHub> = Rc::new(GhCli::new(workspace.root()));
    let sessions: Rc<dyn SessionFactory> = if dry_run {
        Rc::new(Rehearsal::new(log.clone()))
    } else {
        let factory = ClaudeCliFactory::new(workspace.root(), permission_mode, trace)
            .with_env(crate::toolchain::environment());
        Rc::new(if short_cache {
            factory.with_short_cache()
        } else {
            factory
        })
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

/// The prompt book, read once for this run — what every stage's
/// `prompts.text(KEY, DEFAULT)` answers from.
///
/// The book changes only when the watch starts and applies the drafts (see
/// [`apply_drafts`]), so every run between two starts sends the same texts.
/// The edited prompts are named in the journal, so a run that behaves
/// differently says why; and the text of every prompt this run's line could
/// send is recorded in `run_dir`, which is what a dry-run replays.
///
/// # Errors
///
/// [`Halt::Unreadable`](harness_core::domain::Halt::Unreadable) when the book
/// exists and cannot be read: sending the defaults while a human believes
/// their edit runs is the failure this refuses.
pub fn prompts(workspace: &Workspace, log: &Logbook, run_dir: &Path) -> Outcome<Rc<PromptBook>> {
    let book = PromptFile::new(workspace.state_root()).load()?;
    let edited: Vec<&str> = book.edited().collect();
    if !edited.is_empty() {
        log.say(&format!(
            "prompts edited from the view: {}",
            edited.join(", ")
        ));
    }
    let line = run_dir
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let texts: BTreeMap<String, String> = harness_workflows::prompts::of_line(&line)
        .iter()
        .map(|prompt| {
            (
                prompt.key.to_string(),
                book.text(prompt.key, prompt.default),
            )
        })
        .collect();
    if !texts.is_empty() {
        // A record lost is a dry-run that falls back on the defaults, never
        // a run that stops.
        if let Err(halt) = record_used(run_dir, &texts) {
            log.debug(halt.reason());
        }
    }
    Ok(Rc::new(book))
}

/// Applies the drafts the human left in the view: called once, when the
/// watch starts, so a change reaches the runs only at a start — and only the
/// harness ever writes the book a run reads.
///
/// Answers the keys it changed.
///
/// # Errors
///
/// [`Halt::Unreadable`](harness_core::domain::Halt::Unreadable) when the
/// drafts or the book cannot be read; [`Halt::Failed`](harness_core::domain::Halt::Failed)
/// when the book cannot be written — the drafts are then put back, to wait
/// for the next start.
pub fn apply_drafts(here: &Path, now: u64) -> Outcome<Vec<String>> {
    let workspace = Workspace::new(here);
    let drafts_store = DraftFile::new(workspace.state_root());
    let book_store = PromptFile::new(workspace.state_root());
    let drafts = drafts_store.take()?;
    if drafts.is_empty() {
        return Ok(Vec::new());
    }
    let mut book = match book_store.load() {
        Ok(book) => book,
        Err(halt) => {
            let _ = drafts_store.save(&drafts);
            return Err(halt);
        }
    };
    let applied = book.apply(&drafts, harness_workflows::prompts::default_of, now);
    if let Err(halt) = book_store.save(&book) {
        let _ = drafts_store.save(&drafts);
        return Err(halt);
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_adapters::shell::process;

    #[test]
    fn a_router_checkout_is_permanent_harness_named_and_drops_a_session_s_leftovers() {
        // The two facts the lanes' live failure turned on: the checkout is
        // the harness's own (so a missing one is cloned, a dirty one swept),
        // and `force_reset` is the tenant's to pass — a read-only one does
        // not, so commits no remote has still stop it.
        let asked = wanted("git@github.com:o/r.git", "router-lane-1", false);
        assert_eq!(asked.strategy, Strategy::Permanent);
        assert_eq!(asked.url, "git@github.com:o/r.git");
        assert_eq!(asked.id, "router-lane-1");
        assert!(asked.base.is_empty(), "asked.base should be empty");
        assert!(!asked.keep);
        assert!(!asked.force_reset);
        assert!(asked.create_if_missing);
        assert!(asked.drop_uncommitted);
        assert!(wanted("", "router-prfix", true).force_reset);
    }

    /// Runs `git` in `dir` with a throwaway identity, so a commit needs no
    /// configuration of the machine's own.
    async fn git(dir: &Path, args: &[&str]) -> String {
        let with_identity: Vec<String> = ["-c", "user.name=t", "-c", "user.email=t@t"]
            .iter()
            .chain(args)
            .map(|arg| (*arg).to_string())
            .collect();
        let ran = process::run("git", &with_identity, dir)
            .await
            .expect("git runs");
        assert!(ran.ok(), "git {args:?}: {}", ran.stderr);
        ran.out().to_string()
    }

    #[tokio::test]
    async fn a_released_checkout_is_given_back_clean_by_the_real_git() {
        // The one teardown every router-dispatched workflow ends on, built
        // against the real `git`: a release that did nothing would leave
        // the lane as the session left it — the live failure it exists to
        // end. A clone of a local origin, left with an untracked file and a
        // branch of its own, as a session leaves them.
        let root = std::env::temp_dir().join(format!("harness-release-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("mkdir");
        git(&root, &["init", "-q", "-b", "main", "origin"]).await;
        let origin = root.join("origin");
        std::fs::write(origin.join("a"), "base").expect("write");
        git(&origin, &["add", "a"]).await;
        git(&origin, &["commit", "-q", "-m", "base"]).await;
        let origin_url = origin.display().to_string();
        git(&root, &["clone", "-q", &origin_url, "lane"]).await;
        let lane = root.join("lane");
        std::fs::write(lane.join("scratch"), "left behind").expect("write");
        git(&lane, &["branch", "scratch"]).await;
        let mount = Mount {
            workspace: Workspace::new(&root).at(&lane),
            name: "router-lane-1".to_string(),
            path: Some(lane.clone()),
            branch: "main".to_string(),
            disposable: false,
        };
        release_checkout(&mount, &Logbook::null()).await;
        assert!(
            !lane.join("scratch").exists(),
            "the untracked file is dropped"
        );
        assert!(
            lane.join("a").exists(),
            "the checkout itself stays: permanent"
        );
        assert_eq!(git(&lane, &["status", "--porcelain"]).await, "", "clean");
        assert_eq!(
            git(
                &lane,
                &["for-each-ref", "--format=%(refname:short)", "refs/heads"]
            )
            .await,
            "main",
            "the scratch branch is gone, the run's branch stays"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
