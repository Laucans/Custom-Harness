//! What the harness asks a git repository — the port, not the binary.
//!
//! Nothing here decides. Knowing that a branch exists is a response; knowing
//! if it's the right branch is a gate, and gates live in the workflow.
//!
//! # Reads and writes return different things
//!
//! Reads return an already-interpreted **response** — a branch, lines, a bool.
//! Setup verbs return the raw [`Ran`], because a failing `reset --hard` must
//! be able to say *why*: the last line of its stderr is all a human will read.
//! Translating them to `Outcome<()>` would lose exactly that.
//!
//! The implementation — one `git -C <root>` call per method — lives in
//! `harness_adapters::shell::git`.

use std::path::Path;
use std::rc::Rc;

use async_trait::async_trait;

use crate::domain::Outcome;
use crate::ports::shell::process::Ran;

/// What the harness asks git, and nothing more.
#[async_trait(?Send)]
pub trait Repo {
    /// The current branch, or empty if `HEAD` is detached.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn current_branch(&self) -> Outcome<String>;

    /// The short sha of `HEAD`.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn head_sha(&self) -> Outcome<String>;

    /// Files that `status --porcelain` reports, one per line.
    ///
    /// Empty means a clean tree.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn dirty_files(&self) -> Outcome<Vec<String>>;

    /// Does this branch exist locally?
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn has_branch(&self, name: &str) -> Outcome<bool>;

    /// Does `origin` have this branch?
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn origin_has_branch(&self, name: &str) -> Outcome<bool>;

    /// The URL of a remote, or empty if none.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn remote_url(&self, remote: &str) -> Outcome<String>;

    /// Files that git tracks, one per line.
    ///
    /// `ls-files` and not a disk walk: what `.gitignore` excludes is excluded
    /// by construction — dependencies, build artifacts, logs —
    /// and the list is exactly what a repository reader would see.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn tracked_files(&self) -> Outcome<Vec<String>>;

    /// The branch that `origin/HEAD` points to, or empty.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn default_branch(&self) -> Outcome<String>;

    /// Local branches, by name.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn local_branches(&self) -> Outcome<Vec<String>>;

    /// Entries from `stash list`.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn stashes(&self) -> Outcome<Vec<String>>;

    /// Local commits that no remote has, one per line.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn unpushed(&self) -> Outcome<Vec<String>>;

    /// Local branches carrying a patch that `upstream` doesn't have.
    ///
    /// **By patch, not by sha**, and that's not a detail: the target repository
    /// merges by squash (`init-repo` leaves no other method), so commits from
    /// a merged PR no longer exist anywhere with their original sha, nor one
    /// by one. Counting them as work to save was blocking a permanent
    /// workspace on every run.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn branches_at_risk(&self, upstream: &str) -> Outcome<Vec<String>>;

    // --- setup verbs: they return what `git` said ---

    /// Clone `url` into `name`, under the repository this client names.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn clone_repo(&self, url: &str, name: &str) -> Outcome<Ran>;

    /// `fetch --prune`: add remote references, prune dead ones.
    ///
    /// **Doesn't destroy anything local**, and that's what lets it be called
    /// before guard rather than after.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn fetch(&self) -> Outcome<Ran>;

    /// Switch to this branch. `force` overwrites modified files.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn checkout(&self, branch: &str, force: bool) -> Outcome<Ran>;

    /// `reset --hard <reference>`.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn reset_hard(&self, reference: &str) -> Outcome<Ran>;

    /// `clean -fd`: remove what git doesn't track, ignored files excluded.
    ///
    /// Ignored files **stay**, and that's what lets a permanent workspace
    /// keep its installed dependencies.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn clean(&self) -> Outcome<Ran>;

    /// Delete this local branch, even if not merged.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn delete_branch(&self, name: &str) -> Outcome<Ran>;

    /// Create `name` from `from` and switch to it in one step.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn create_local_branch(&self, name: &str, from: &str) -> Outcome<Ran>;

    /// `add -A`: stage every change, tracked or new.
    ///
    /// Safe only because the tree is exclusive to one task at a time — see
    /// `dev_loop`'s `between` hook, which resets it before the next pick.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn stage_all(&self) -> Outcome<Ran>;

    /// `commit -m <message>`, against what's already staged.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn commit(&self, message: &str) -> Outcome<Ran>;

    /// `push -u origin <branch>`.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn push(&self, branch: &str) -> Outcome<Ran>;

    /// `switch --orphan <name>`: a new branch with no history and an empty
    /// tree — where a store that is not code (an archive) begins.
    ///
    /// Answers a refusal by default: only a real `git` and the fakes that
    /// Keeps on the disk only what `patterns` name — `git sparse-checkout`
    /// in its non-cone syntax, one pattern per line: `/dir/`, `/file`,
    /// `!/dir/`. The index and `.git` stay whole; only the working tree
    /// narrows, so a commit and a push are unchanged.
    ///
    /// **Refused by default**: a repository adapter that does not say how it
    /// narrows must not pretend it did, because a session would then read a
    /// tree the harness believes it cannot.
    ///
    /// # Errors
    /// If `git` couldn't be launched, or this adapter cannot narrow.
    async fn sparse_checkout(&self, patterns: &[String]) -> Outcome<Ran> {
        Ok(Ran {
            code: Some(1),
            stdout: String::new(),
            stderr: format!(
                "this git cannot narrow the working tree to {} pattern(s)",
                patterns.len()
            ),
        })
    }

    /// Puts the whole working tree back — the inverse of
    /// [`Self::sparse_checkout`]. A no-op by default: a repository that was
    /// never narrowed has nothing to widen, which is the honest answer for
    /// an adapter that does not narrow.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn sparse_checkout_off(&self) -> Outcome<Ran> {
        Ok(Ran {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    /// test an archive need it.
    ///
    /// # Errors
    /// If `git` couldn't be launched.
    async fn create_orphan_branch(&self, name: &str) -> Outcome<Ran> {
        Ok(Ran {
            code: Some(1),
            stdout: String::new(),
            stderr: format!("this git cannot create the orphan branch {name}"),
        })
    }
}

/// Open a [`Repo`] on a given repository.
///
/// Workspace setup talks to **three** repositories: the one the run launches from,
/// the parent folder where the clone lands, and the clone itself. A `Repo` is
/// tied to a root, so we need a way to open one elsewhere — and one test seam for all three.
pub trait Repos {
    /// A `git` on this repository.
    fn at(&self, root: &Path) -> Rc<dyn Repo>;
}
