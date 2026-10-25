//! The `git` binary, wrapped — the implementation of the [`Repo`] port.
//!
//! Each call **names the repository** (`-C <root>`) rather than relying on
//! the current directory: the harness runs from anywhere, and a `git status`
//! that answers on the wrong repo would miss a dirty tree.
//!
//! Nothing here decides: what the port promises, and why reads and setup
//! verbs return different things, is documented with it in
//! [`ports::shell::git`](harness_core::ports::shell::git).

use std::path::{Path, PathBuf};
use std::rc::Rc;

use async_trait::async_trait;

use crate::shell::process;
use harness_core::domain::Outcome;
use harness_core::ports::shell::git::{Repo, Repos};
use harness_core::ports::shell::process::Ran;

/// The binary to call.
const BINARY: &str = "git";

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

    /// Whether `branch` holds a patch `upstream` has neither commit by
    /// commit (a rebase merge) nor as their sum (a squash merge).
    async fn carries_unmerged_work(&self, upstream: &str, branch: &str) -> Outcome<bool> {
        // `cherry` compares by patch-id: a `+` line is a commit that
        // `upstream` doesn't have, even if rewritten by a rebase.
        let cherry = self.git(&["cherry", upstream, branch]).await?;
        if !cherry.lines().iter().any(|line| line.starts_with('+')) {
            return Ok(false);
        }
        // A squash merge keeps none of the branch's commits, only their sum:
        // merging the branch into `upstream` then changes nothing, and the
        // merged tree is `upstream`'s own. A conflict is work to keep.
        let merged = self
            .git(&["merge-tree", "--write-tree", upstream, branch])
            .await?;
        if !merged.ok() {
            return Ok(true);
        }
        let upstream_tree = self
            .git(&["rev-parse", &format!("{upstream}^{{tree}}")])
            .await?;
        Ok(!upstream_tree.ok() || merged.out() != upstream_tree.out())
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
            // A branch some remote holds whole is no work to lose, whatever
            // `upstream` has of it: a task's PR branch fetched to repair it
            // read as "work origin does not have" and refused the checkout.
            let unpushed = self
                .git(&["log", &branch, "--not", "--remotes", "--oneline"])
                .await?;
            if unpushed.lines().is_empty() {
                continue;
            }
            if self.carries_unmerged_work(upstream, &branch).await? {
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

    async fn create_local_branch(&self, name: &str, from: &str) -> Outcome<Ran> {
        self.git(&["switch", "-c", name, from]).await
    }

    async fn stage_all(&self) -> Outcome<Ran> {
        self.git(&["add", "-A"]).await
    }

    async fn commit(&self, message: &str) -> Outcome<Ran> {
        self.git(&["commit", "-m", message]).await
    }

    async fn push(&self, branch: &str) -> Outcome<Ran> {
        self.git(&["push", "-u", "origin", branch]).await
    }

    async fn changed_between(&self, from: &str, to: &str) -> Outcome<Vec<String>> {
        Ok(self
            .git(&["diff", "--name-only", &format!("{from}..{to}")])
            .await?
            .lines())
    }

    async fn sparse_checkout(&self, patterns: &[String]) -> Outcome<Ran> {
        let args = sparse_set_args(patterns);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.git(&refs).await
    }

    async fn sparse_checkout_off(&self) -> Outcome<Ran> {
        self.git(&["sparse-checkout", "disable"]).await
    }

    async fn create_orphan_branch(&self, name: &str) -> Outcome<Ran> {
        self.git(&["switch", "--orphan", name]).await
    }

    async fn merge_in_progress(&self) -> Outcome<bool> {
        // `MERGE_HEAD` exists from the first conflict to the merge commit or
        // its abort: a merge whose conflicts were all resolved but never
        // committed is still in progress.
        Ok(self
            .git(&["rev-parse", "--quiet", "--verify", "MERGE_HEAD"])
            .await?
            .ok())
    }

    async fn merge(&self, reference: &str) -> Outcome<Ran> {
        self.git(&["merge", "--no-edit", reference]).await
    }

    async fn take_theirs(&self, path: &str) -> Outcome<Ran> {
        self.git(&["checkout", "--theirs", "--", path]).await
    }

    async fn remove(&self, path: &str) -> Outcome<Ran> {
        self.git(&["rm", "--quiet", "--", path]).await
    }

    async fn merge_base(&self, a: &str, b: &str) -> Outcome<Option<String>> {
        let ran = self.git(&["merge-base", a, b]).await?;
        Ok(base_of(&ran))
    }

    async fn show(&self, spec: &str) -> Outcome<Option<String>> {
        // As-is, never trimmed: the text is written back byte for byte.
        let ran = self.git(&["show", spec]).await?;
        Ok(ran.ok().then_some(ran.stdout))
    }

    async fn stage_paths(&self, paths: &[String]) -> Outcome<Ran> {
        let mut args = vec!["add", "-A", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.git(&args).await
    }

    async fn merge_file(&self, ours: &Path, base: &Path, theirs: &Path) -> Outcome<Ran> {
        let (ours, base, theirs) = (
            ours.display().to_string(),
            base.display().to_string(),
            theirs.display().to_string(),
        );
        self.git(&["merge-file", "-p", &ours, &base, &theirs]).await
    }

    async fn push_head(&self, branch: &str) -> Outcome<Ran> {
        self.git(&["push", "origin", &format!("HEAD:refs/heads/{branch}")])
            .await
    }

    async fn abort_merge(&self) -> Outcome<Ran> {
        self.git(&["merge", "--abort"]).await
    }

    async fn clear_stashes(&self) -> Outcome<Ran> {
        self.git(&["stash", "clear"]).await
    }
}

/// `sparse-checkout set --no-cone -- <patterns>`: the patterns are
/// gitignore-style lines, which `--no-cone` takes as they are; `--` keeps a
/// pattern that starts with `!` or `-` from reading as a flag.
fn sparse_set_args(patterns: &[String]) -> Vec<String> {
    let mut args = vec![
        "sparse-checkout".to_string(),
        "set".to_string(),
        "--no-cone".to_string(),
        "--".to_string(),
    ];
    args.extend(patterns.iter().cloned());
    args
}

/// The commit `git merge-base` answered: only from a call that succeeded
/// and said something — a failure's words, or a silent success, name none.
fn base_of(ran: &Ran) -> Option<String> {
    (ran.ok() && !ran.out().is_empty()).then(|| ran.out().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ran(code: i32, stdout: &str) -> Ran {
        Ran {
            code: Some(code),
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    #[test]
    fn a_merge_base_is_read_only_from_a_success_that_names_one() {
        assert_eq!(base_of(&ran(0, "abc\n")).as_deref(), Some("abc"));
        assert_eq!(base_of(&ran(0, "  \n")), None, "a silent success");
        assert_eq!(base_of(&ran(1, "abc\n")), None, "a failure's words");
        assert_eq!(base_of(&ran(1, "")), None);
    }

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

    #[test]
    fn create_local_branch_switches_in_one_step() {
        let args = git().argv(&["switch", "-c", "milestone/4-cities", "main_agent"]);
        assert_eq!(
            args,
            vec![
                "-C".to_string(),
                "/tmp/le-clone".to_string(),
                "switch".to_string(),
                "-c".to_string(),
                "milestone/4-cities".to_string(),
                "main_agent".to_string(),
            ]
        );
    }

    #[test]
    fn narrowing_sets_the_patterns_in_non_cone_mode_behind_a_double_dash() {
        let args = sparse_set_args(&[
            "/*".to_string(),
            "!/*/".to_string(),
            "/crates/s/".to_string(),
        ]);
        assert_eq!(
            args,
            [
                "sparse-checkout",
                "set",
                "--no-cone",
                "--",
                "/*",
                "!/*/",
                "/crates/s/"
            ]
        );
    }

    #[test]
    fn the_files_of_a_round_are_the_diff_between_two_references_by_name() {
        let args = git().argv(&["diff", "--name-only", "abc..origin/milestone/3-x"]);
        assert_eq!(
            &args[2..],
            ["diff", "--name-only", "abc..origin/milestone/3-x"]
        );
    }

    #[test]
    fn widening_disables_the_sparse_checkout() {
        let args = git().argv(&["sparse-checkout", "disable"]);
        assert_eq!(&args[2..], ["sparse-checkout", "disable"]);
    }

    #[test]
    fn stage_all_is_add_dash_a_not_dash_dot() {
        // `-A` picks up deletions too, not just new/modified files like `.`.
        let args = git().argv(&["add", "-A"]);
        assert_eq!(&args[2..], &["add".to_string(), "-A".to_string()]);
    }

    #[test]
    fn commit_carries_the_message_as_one_argument() {
        let args = git().argv(&["commit", "-m", "feat: deux lignes\n\nplus de détail"]);
        assert_eq!(args[2], "commit");
        assert_eq!(args[3], "-m");
        assert_eq!(args[4], "feat: deux lignes\n\nplus de détail");
    }

    #[test]
    fn push_sets_upstream_on_origin() {
        let args = git().argv(&["push", "-u", "origin", "feat/x"]);
        assert!(args.contains(&"-u".to_string()));
        assert!(args.contains(&"origin".to_string()));
        assert_eq!(args.last(), Some(&"feat/x".to_string()));
    }

    /// A real repository: a remote, a clone, one branch pushed, one not.
    async fn sh(dir: &Path, args: &[&str]) {
        let out = tokio::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .await
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn a_branch_a_remote_holds_whole_is_not_at_risk_an_unpushed_one_is() {
        let root = std::env::temp_dir().join(format!("harness-git-at-risk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let remote = root.join("remote.git");
        let clone = root.join("clone");
        std::fs::create_dir_all(&remote).expect("mkdir");
        sh(&remote, &["init", "-q", "--bare", "-b", "main"]).await;
        sh(
            &root,
            &["clone", "-q", remote.to_str().expect("utf8"), "clone"],
        )
        .await;
        std::fs::write(clone.join("a"), "a").expect("write");
        sh(&clone, &["add", "a"]).await;
        sh(&clone, &["commit", "-q", "-m", "base"]).await;
        sh(&clone, &["push", "-q", "-u", "origin", "HEAD:main"]).await;
        sh(&clone, &["checkout", "-q", "-b", "pushed"]).await;
        std::fs::write(clone.join("b"), "b").expect("write");
        sh(&clone, &["add", "b"]).await;
        sh(&clone, &["commit", "-q", "-m", "pushed work"]).await;
        sh(&clone, &["push", "-q", "-u", "origin", "pushed"]).await;
        sh(&clone, &["checkout", "-q", "-b", "local-only", "main"]).await;
        std::fs::write(clone.join("c"), "c").expect("write");
        sh(&clone, &["add", "c"]).await;
        sh(&clone, &["commit", "-q", "-m", "local work"]).await;
        let at_risk = GitCli::new(&clone)
            .branches_at_risk("origin/main")
            .await
            .expect("judged");
        assert_eq!(at_risk, vec!["local-only".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_conflicted_merge_is_seen_in_progress_and_aborted_whole() {
        // The lane's state on ForgeDesMondes (23:09Z): a repair merged the
        // milestone branch, hit a conflict, and left `UU` entries behind.
        let root = std::env::temp_dir().join(format!("harness-git-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("mkdir");
        let repo = root.join("repo");
        sh(&root, &["init", "-q", "-b", "main", "repo"]).await;
        // The merge below runs through the port, without the helper's
        // identity: a runner with no global git user refuses it otherwise.
        sh(&repo, &["config", "user.name", "t"]).await;
        sh(&repo, &["config", "user.email", "t@t"]).await;
        std::fs::write(repo.join("a"), "base").expect("write");
        sh(&repo, &["add", "a"]).await;
        sh(&repo, &["commit", "-q", "-m", "base"]).await;
        sh(&repo, &["checkout", "-q", "-b", "feat"]).await;
        std::fs::write(repo.join("a"), "feat").expect("write");
        sh(&repo, &["commit", "-q", "-am", "feat"]).await;
        sh(&repo, &["checkout", "-q", "main"]).await;
        std::fs::write(repo.join("a"), "main").expect("write");
        sh(&repo, &["commit", "-q", "-am", "main"]).await;
        sh(&repo, &["checkout", "-q", "feat"]).await;
        let git = GitCli::new(&repo);
        assert!(!git.merge_in_progress().await.expect("asked"));
        // The merge conflicts: a non-zero exit, the merge left in progress.
        let merged = git.merge("main").await.expect("ran");
        assert!(!merged.ok(), "the merge conflicts");
        assert!(git.merge_in_progress().await.expect("asked"));
        assert!(
            git.dirty_files()
                .await
                .expect("read")
                .iter()
                .any(|line| line.starts_with("UU ")),
            "an unmerged entry"
        );
        assert!(git.abort_merge().await.expect("ran").ok());
        assert!(!git.merge_in_progress().await.expect("asked"));
        assert!(
            git.dirty_files().await.expect("read").is_empty(),
            "the abort leaves a clean tree"
        );
        // Again, resolved with the side merged in, and committed.
        assert!(!git.merge("main").await.expect("ran").ok());
        assert!(git.take_theirs("a").await.expect("ran").ok());
        assert_eq!(
            std::fs::read_to_string(repo.join("a")).expect("read"),
            "main"
        );
        assert!(git.stage_all().await.expect("ran").ok());
        assert!(git.commit("merge main").await.expect("ran").ok());
        assert!(!git.merge_in_progress().await.expect("asked"));
        assert!(
            git.merge("main").await.expect("ran").ok(),
            "nothing left to bring in"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_file_the_side_merged_in_deleted_is_removed_where_theirs_cannot_be_taken() {
        // A component removed on the base while the branch rebuilt its
        // bundle: `UD`, deleted by them.
        let root = std::env::temp_dir().join(format!("harness-git-rm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("mkdir");
        let repo = root.join("repo");
        sh(&root, &["init", "-q", "-b", "main", "repo"]).await;
        sh(&repo, &["config", "user.name", "t"]).await;
        sh(&repo, &["config", "user.email", "t@t"]).await;
        std::fs::write(repo.join("gen.js"), "base").expect("write");
        sh(&repo, &["add", "gen.js"]).await;
        sh(&repo, &["commit", "-q", "-m", "base"]).await;
        sh(&repo, &["checkout", "-q", "-b", "feat"]).await;
        std::fs::write(repo.join("gen.js"), "feat").expect("write");
        sh(&repo, &["commit", "-q", "-am", "feat"]).await;
        sh(&repo, &["checkout", "-q", "main"]).await;
        sh(&repo, &["rm", "-q", "gen.js"]).await;
        sh(&repo, &["commit", "-q", "-m", "gone"]).await;
        sh(&repo, &["checkout", "-q", "feat"]).await;
        let git = GitCli::new(&repo);
        assert!(!git.merge("main").await.expect("ran").ok());
        assert_eq!(
            git.dirty_files().await.expect("read"),
            ["UD gen.js"],
            "deleted by them"
        );
        assert!(
            !git.take_theirs("gen.js").await.expect("ran").ok(),
            "theirs has no such file"
        );
        assert!(git.remove("gen.js").await.expect("ran").ok());
        assert!(!repo.join("gen.js").exists());
        assert!(git.commit("merge main").await.expect("ran").ok());
        assert!(!git.merge_in_progress().await.expect("asked"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_split_handler_merges_three_ways_on_a_base_read_from_git() {
        // The handler-split catch-up's verbs on a real repository: the merge
        // base, a conflicted path's two sides by stage, a blob of a commit,
        // a three-way merge of files on the disk, staging some paths only.
        let root = std::env::temp_dir().join(format!("harness-git-3way-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("mkdir");
        let repo = root.join("repo");
        sh(&root, &["init", "-q", "-b", "main", "repo"]).await;
        sh(&repo, &["config", "user.name", "t"]).await;
        sh(&repo, &["config", "user.email", "t@t"]).await;
        std::fs::write(repo.join("other"), "x").expect("write");
        sh(&repo, &["add", "other"]).await;
        sh(&repo, &["commit", "-q", "-m", "base"]).await;
        let git = GitCli::new(&repo);
        let base = git.merge_base("HEAD", "HEAD").await.expect("asked");
        assert_eq!(base.as_deref().map(str::len), Some(40), "{base:?}");
        sh(&repo, &["checkout", "-q", "-b", "feat"]).await;
        std::fs::write(repo.join("h.js"), "a\nB\nc\nd\n").expect("write");
        sh(&repo, &["add", "h.js"]).await;
        sh(&repo, &["commit", "-q", "-m", "feat adds h"]).await;
        sh(&repo, &["checkout", "-q", "main"]).await;
        std::fs::write(repo.join("h.js"), "a\nb\nc\nD\n").expect("write");
        sh(&repo, &["add", "h.js"]).await;
        sh(&repo, &["commit", "-q", "-m", "main adds h"]).await;
        assert_eq!(git.merge_base("HEAD", "feat").await.expect("asked"), base);
        assert_eq!(
            git.merge_base("HEAD", "no-such-ref").await.expect("asked"),
            None
        );
        assert!(!git.merge("feat").await.expect("ran").ok());
        assert_eq!(git.dirty_files().await.expect("read"), ["AA h.js"]);
        let ours = git.show(":2:h.js").await.expect("read").expect("ours");
        let theirs = git.show(":3:h.js").await.expect("read").expect("theirs");
        assert_eq!(
            (ours.as_str(), theirs.as_str()),
            ("a\nb\nc\nD\n", "a\nB\nc\nd\n")
        );
        assert_eq!(
            git.show(":1:h.js").await.expect("read"),
            None,
            "no base stage"
        );
        assert_eq!(
            git.show("HEAD:other").await.expect("read").as_deref(),
            Some("x")
        );
        // On the base both sides came from, each changed its own line.
        let scratch = root.join("merge");
        std::fs::create_dir_all(&scratch).expect("mkdir");
        let (o, b, t) = (scratch.join("o"), scratch.join("b"), scratch.join("t"));
        std::fs::write(&o, &ours).expect("write");
        std::fs::write(&b, "a\nb\nc\nd\n").expect("write");
        std::fs::write(&t, &theirs).expect("write");
        let merged = git.merge_file(&o, &b, &t).await.expect("ran");
        assert_eq!(merged.code, Some(0));
        assert_eq!(merged.stdout, "a\nB\nc\nD\n");
        assert_eq!(
            std::fs::read_to_string(&o).expect("read"),
            ours,
            "-p writes nothing"
        );
        // On an empty base, both added everything: one conflict.
        std::fs::write(&b, "").expect("write");
        let conflicted = git.merge_file(&o, &b, &t).await.expect("ran");
        assert_eq!(conflicted.code, Some(1));
        assert!(conflicted.stdout.contains("<<<<<<<"));
        let failed = git
            .merge_file(&scratch.join("absent"), &b, &t)
            .await
            .expect("ran");
        assert!(
            !matches!(failed.code, Some(0..=127)),
            "an error is no count of conflicts: {failed:?}"
        );
        // Resolved, staged by path alone: the other change stays unstaged.
        std::fs::write(repo.join("h.js"), &merged.stdout).expect("write");
        std::fs::write(repo.join("other"), "changed").expect("write");
        assert!(
            git.stage_paths(&["h.js".to_string()])
                .await
                .expect("ran")
                .ok()
        );
        assert!(git.commit("merge feat").await.expect("ran").ok());
        assert_eq!(git.dirty_files().await.expect("read"), [" M other"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_detached_head_pushed_as_a_branch_leaves_no_local_branch_ahead() {
        // The catch-up's shape: work on a detached `HEAD`, push it as the
        // branch — a kill before the push leaves a commit no branch holds.
        let root = std::env::temp_dir().join(format!("harness-git-head-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("mkdir");
        sh(&root, &["init", "-q", "--bare", "-b", "main", "origin.git"]).await;
        sh(&root, &["clone", "-q", "origin.git", "repo"]).await;
        let repo = root.join("repo");
        sh(&repo, &["config", "user.name", "t"]).await;
        sh(&repo, &["config", "user.email", "t@t"]).await;
        sh(&repo, &["checkout", "-q", "-b", "milestone/1"]).await;
        std::fs::write(repo.join("a"), "1").expect("write");
        sh(&repo, &["add", "a"]).await;
        sh(&repo, &["commit", "-q", "-m", "one"]).await;
        sh(&repo, &["push", "-q", "-u", "origin", "milestone/1"]).await;
        let git = GitCli::new(&repo);
        assert!(
            git.checkout("origin/milestone/1", true)
                .await
                .expect("ran")
                .ok()
        );
        assert_eq!(git.current_branch().await.expect("read"), "", "detached");
        std::fs::write(repo.join("a"), "2").expect("write");
        sh(&repo, &["commit", "-q", "-am", "two"]).await;
        assert!(
            git.branches_at_risk("origin/milestone/1")
                .await
                .expect("read")
                .is_empty(),
            "the unpushed commit is on no branch"
        );
        assert!(git.push_head("milestone/1").await.expect("ran").ok());
        let head = git.head_sha().await.expect("read");
        sh(&repo, &["fetch", "-q", "origin"]).await;
        assert_eq!(
            git.git(&["rev-parse", "--short", "origin/milestone/1"])
                .await
                .expect("ran")
                .out(),
            head
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
