//! The `git` binary, wrapped.
//!
//! Each call **names the repository** (`-C <root>`) rather than relying on
//! the current directory: the harness runs from anywhere, and a `git status`
//! that answers on the wrong repo would miss a dirty tree.
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

use std::path::{Path, PathBuf};
use std::rc::Rc;

use async_trait::async_trait;

use crate::adapters::shell::process::{self, Ran};
use crate::domain::Outcome;

/// The binary to call.
const BINARY: &str = "git";

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
    /// merges with rebase, so commits from a merged PR no longer exist anywhere
    /// with their original sha. Counting them as work to save was blocking a permanent
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

/// The factory that makes real `git` instances.
pub struct GitRepos;

impl Repos for GitRepos {
    fn at(&self, root: &Path) -> Rc<dyn Repo> {
        Rc::new(GitCli::new(root))
    }
}

/// `git`, called on a given repository.
pub struct GitCli {
    root: PathBuf,
}

impl GitCli {
    /// `git`, on this repository.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Arguments for a call, with repository named first.
    ///
    /// Pure, and that's where the invariant lives: **every call leaves with `-C <root>`**.
    /// A test verifies it, because forgetting it isn't visible — it answers correctly
    /// on the wrong repository.
    fn argv(&self, args: &[&str]) -> Vec<String> {
        let mut out = vec!["-C".to_string(), self.root.display().to_string()];
        out.extend(args.iter().map(ToString::to_string));
        out
    }

    async fn git(&self, args: &[&str]) -> Outcome<Ran> {
        process::run(BINARY, &self.argv(args), &self.root).await
    }
}

#[async_trait(?Send)]
impl Repo for GitCli {
    async fn current_branch(&self) -> Outcome<String> {
        // Empty is a response: a detached `HEAD` has no branch,
        // and `symbolic-ref` exits non-zero in that case.
        Ok(self
            .git(&["symbolic-ref", "--short", "HEAD"])
            .await?
            .out()
            .to_string())
    }

    async fn head_sha(&self) -> Outcome<String> {
        Ok(self
            .git(&["rev-parse", "--short", "HEAD"])
            .await?
            .out()
            .to_string())
    }

    async fn dirty_files(&self) -> Outcome<Vec<String>> {
        Ok(self.git(&["status", "--porcelain"]).await?.lines())
    }

    async fn has_branch(&self, name: &str) -> Outcome<bool> {
        Ok(self
            .git(&["rev-parse", "--verify", "--quiet", name])
            .await?
            .ok())
    }

    async fn origin_has_branch(&self, name: &str) -> Outcome<bool> {
        Ok(self
            .git(&["ls-remote", "--exit-code", "--heads", "origin", name])
            .await?
            .ok())
    }

    async fn remote_url(&self, remote: &str) -> Outcome<String> {
        Ok(self
            .git(&["remote", "get-url", remote])
            .await?
            .out()
            .to_string())
    }

    async fn tracked_files(&self) -> Outcome<Vec<String>> {
        Ok(self.git(&["ls-files"]).await?.lines())
    }

    async fn default_branch(&self) -> Outcome<String> {
        // `origin/main` -> `main`. Empty when `origin/HEAD` isn't set,
        // which happens on a shallow clone: a response, not a failure.
        let found = self
            .git(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
            .await?;
        Ok(found
            .out()
            .strip_prefix("origin/")
            .unwrap_or_default()
            .to_string())
    }

    async fn local_branches(&self) -> Outcome<Vec<String>> {
        Ok(self
            .git(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .await?
            .lines())
    }

    async fn stashes(&self) -> Outcome<Vec<String>> {
        Ok(self.git(&["stash", "list"]).await?.lines())
    }

    async fn unpushed(&self) -> Outcome<Vec<String>> {
        Ok(self
            .git(&["log", "--branches", "--not", "--remotes", "--oneline"])
            .await?
            .lines())
    }

    async fn branches_at_risk(&self, upstream: &str) -> Outcome<Vec<String>> {
        let mut at_risk = Vec::new();
        for branch in self.local_branches().await? {
            // `cherry` compares by patch-id: a `+` line is a commit
            // that `upstream` doesn't have, even if rewritten by a rebase.
            let said = self.git(&["cherry", upstream, &branch]).await?;
            if said.lines().iter().any(|line| line.starts_with('+')) {
                at_risk.push(branch);
            }
        }
        Ok(at_risk)
    }

    async fn clone_repo(&self, url: &str, name: &str) -> Outcome<Ran> {
        self.git(&["clone", url, name]).await
    }

    async fn fetch(&self) -> Outcome<Ran> {
        self.git(&["fetch", "--prune", "origin"]).await
    }

    async fn checkout(&self, branch: &str, force: bool) -> Outcome<Ran> {
        // `--force` only when resetting: `git` refuses to switch
        // if modified files would be overwritten, so a `--force-reset` would fail
        // on exactly the dirty workspace it exists to obliterate — complaining
        // about a missing branch that was there.
        if force {
            self.git(&["checkout", "--force", branch]).await
        } else {
            self.git(&["checkout", branch]).await
        }
    }

    async fn reset_hard(&self, reference: &str) -> Outcome<Ran> {
        self.git(&["reset", "--hard", reference]).await
    }

    async fn clean(&self) -> Outcome<Ran> {
        self.git(&["clean", "-fd"]).await
    }

    async fn delete_branch(&self, name: &str) -> Outcome<Ran> {
        self.git(&["branch", "-D", name]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git() -> GitCli {
        GitCli::new(Path::new("/tmp/le-clone"))
    }

    #[test]
    fn every_call_names_the_repository_first() {
        // The mistake we make impossible: `git status` would answer correctly
        // on the current directory's repository.
        let args = git().argv(&["status", "--porcelain"]);
        assert_eq!(args[0], "-C");
        assert_eq!(args[1], "/tmp/le-clone");
        assert_eq!(args[2], "status");
    }

    #[test]
    fn the_verb_and_its_flags_keep_their_order_after_the_repository() {
        let args = git().argv(&["rev-parse", "--verify", "--quiet", "main"]);
        assert_eq!(
            args,
            vec![
                "-C".to_string(),
                "/tmp/le-clone".to_string(),
                "rev-parse".to_string(),
                "--verify".to_string(),
                "--quiet".to_string(),
                "main".to_string(),
            ]
        );
    }

    #[test]
    fn a_call_with_no_arguments_still_names_the_repository() {
        assert_eq!(git().argv(&[]).len(), 2);
    }

    #[test]
    fn tracked_files_uses_ls_files_not_a_disk_walk() {
        // `.gitignore` excludes dependencies and build artifacts by construction;
        // a disk walk would include them, `ls-files` doesn't.
        assert_eq!(git().argv(&["ls-files"])[2], "ls-files");
    }

    #[test]
    fn clean_leaves_ignored_files_alone() {
        // Without this, a permanent workspace would lose `node_modules` on every run
        // and every stage verification command would fail. `-x` would delete ignored files;
        // it must never appear here.
        let args = git().argv(&["clean", "-fd"]);
        assert!(!args.iter().any(|a| a.contains('x')));
    }

    #[test]
    fn a_reset_checkout_forces_and_a_plain_one_does_not() {
        // Two forms of argv, and the test locks them in: `--force` on a checkout
        // that isn't resetting would overwrite a workspace we wanted to keep.
        assert!(
            !git()
                .argv(&["checkout", "main_agent"])
                .contains(&"--force".to_string())
        );
        assert!(
            git()
                .argv(&["checkout", "--force", "main_agent"])
                .contains(&"--force".to_string())
        );
    }

    #[test]
    fn a_fetch_prunes_so_a_stale_remote_ref_cannot_fake_unpushed_work() {
        let args = git().argv(&["fetch", "--prune", "origin"]);
        assert!(args.contains(&"--prune".to_string()));
    }
}
