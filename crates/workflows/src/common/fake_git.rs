//! An in-memory `git`, shared by workflows that need one for their tests.
//!
//! A fake adapter, not a mock: it answers what a test told it and records
//! every verb it was asked, with the repository it was asked on. `init-repo`
//! uses it to prove a clone was made, files committed, a branch pushed —
//! and that a dry run asked for none of it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::Outcome;
use harness_core::ports::shell::git::{Repo, Repos};
use harness_core::ports::shell::process::Ran;

/// Every repository the test opened, sharing one call log.
///
/// Cloning it clones the handle, not the log: the test keeps one clone to
/// read what the ports' copy was asked.
#[derive(Default, Clone)]
pub struct FakeRepos {
    /// The verbs asked, in order, as `<root>: <verb>`.
    calls: Rc<RefCell<Vec<String>>>,
    /// Verbs that fail, by prefix (`"push"`).
    failing: Rc<Vec<&'static str>>,
    /// What `changed_between` answers, whatever the references.
    changed: Rc<Vec<String>>,
    /// The files a `merge` leaves conflicted, each with its status code:
    /// when there are some, the merge fails and `dirty_files` answers them
    /// as `<code> <path>`.
    conflicts: Rc<Vec<(String, String)>>,
    /// Whether `push` is refused as a non-fast-forward: the remote branch
    /// moved since the fetch.
    rejects_push: bool,
    /// The files `dirty_files` answers as staged changes while nothing
    /// conflicts: what a generator rewrote.
    rewritten: Rc<Vec<String>>,
    /// The branches that exist, local and on `origin`; `None`: every branch
    /// asked about exists.
    branches: Option<Rc<(Vec<String>, Vec<String>)>>,
    /// One conflict set per `merge`, in order, replacing `conflicts` when
    /// there are some: past the last, a merge is clean.
    rounds: Rc<Vec<Vec<(String, String)>>>,
    /// How many merges were asked.
    merges: Rc<Cell<usize>>,
    /// Whether a merge is left in progress: a conflicting merge until it
    /// is aborted or committed.
    in_merge: Rc<Cell<bool>>,
    /// What `show` answers, by spec (`:2:<path>`, `<sha>:<path>`).
    shown: Rc<HashMap<String, String>>,
    /// What `merge_base` answers.
    base: Option<String>,
    /// What each `merge_file` answers, in order; past the end, a clean
    /// merge printing `merged <ours>`.
    merged_files: Rc<RefCell<Vec<Ran>>>,
}

impl FakeRepos {
    /// A `git` whose verbs starting with one of these prefixes fail.
    pub fn failing(prefixes: &[&'static str]) -> Self {
        Self {
            failing: Rc::new(prefixes.to_vec()),
            ..Self::default()
        }
    }

    /// A `git` whose diff between any two references is these files.
    pub fn changing(files: &[&str]) -> Self {
        Self {
            changed: Rc::new(files.iter().map(|f| (*f).to_string()).collect()),
            ..Self::default()
        }
    }

    /// A `git` whose merge conflicts on these files, and whose verbs
    /// starting with one of `failing` fail.
    pub fn conflicting(files: &[&str], failing: &[&'static str]) -> Self {
        let both: Vec<(&str, &str)> = files.iter().map(|file| ("UU", *file)).collect();
        Self::conflicting_as(&both, failing)
    }

    /// A `git` whose merge conflicts on these files, each with its status
    /// code (`UD`: deleted by them), and whose verbs starting with one of
    /// `failing` fail.
    pub fn conflicting_as(files: &[(&str, &str)], failing: &[&'static str]) -> Self {
        Self {
            failing: Rc::new(failing.to_vec()),
            conflicts: Rc::new(
                files
                    .iter()
                    .map(|(code, file)| ((*code).to_string(), (*file).to_string()))
                    .collect(),
            ),
            ..Self::default()
        }
    }

    /// The same `git`, its `push` refused as a non-fast-forward.
    #[must_use]
    pub fn rejecting_push(self) -> Self {
        Self {
            rejects_push: true,
            ..self
        }
    }

    /// The same `git`, these files staged as rewritten while nothing
    /// conflicts.
    #[must_use]
    pub fn rewriting(self, files: &[&str]) -> Self {
        Self {
            rewritten: Rc::new(files.iter().map(|f| (*f).to_string()).collect()),
            ..self
        }
    }

    /// The same `git`, holding exactly these branches, local and on
    /// `origin`.
    #[must_use]
    pub fn with_branches(self, local: &[&str], origin: &[&str]) -> Self {
        let owned = |names: &[&str]| names.iter().map(|n| (*n).to_string()).collect();
        Self {
            branches: Some(Rc::new((owned(local), owned(origin)))),
            ..self
        }
    }

    /// A `git` whose successive merges conflict on these sets, each file
    /// with its status code: the first merge on `first`, the second on
    /// `second` (empty: clean), any later one clean.
    pub fn conflicting_then(first: &[(&str, &str)], second: &[(&str, &str)]) -> Self {
        let owned = |files: &[(&str, &str)]| -> Vec<(String, String)> {
            files
                .iter()
                .map(|(code, file)| ((*code).to_string(), (*file).to_string()))
                .collect()
        };
        Self {
            rounds: Rc::new(vec![owned(first), owned(second)]),
            ..Self::default()
        }
    }

    /// The same `git`, its verbs starting with one of these prefixes
    /// failing.
    #[must_use]
    pub fn failing_too(self, prefixes: &[&'static str]) -> Self {
        Self {
            failing: Rc::new(prefixes.to_vec()),
            ..self
        }
    }

    /// The same `git`, whose merge base of any two commits is `sha`.
    #[must_use]
    pub fn with_merge_base(self, sha: &str) -> Self {
        Self {
            base: Some(sha.to_string()),
            ..self
        }
    }

    /// The same `git`, whose `show` answers these texts by spec.
    #[must_use]
    pub fn showing(self, blobs: &[(&str, &str)]) -> Self {
        Self {
            shown: Rc::new(
                blobs
                    .iter()
                    .map(|(spec, text)| ((*spec).to_string(), (*text).to_string()))
                    .collect(),
            ),
            ..self
        }
    }

    /// The same `git`, whose three-way merges of files answer these, in
    /// order.
    #[must_use]
    pub fn merging_files(self, answers: Vec<Ran>) -> Self {
        Self {
            merged_files: Rc::new(RefCell::new(answers)),
            ..self
        }
    }

    /// The conflicts of the merge asked last.
    fn current_conflicts(&self) -> Vec<(String, String)> {
        if self.rounds.is_empty() {
            return self.conflicts.as_ref().clone();
        }
        self.merges
            .get()
            .checked_sub(1)
            .and_then(|at| self.rounds.get(at))
            .cloned()
            .unwrap_or_default()
    }

    /// The verbs asked so far, in order.
    pub fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }

    /// True if a verb starting with `prefix` was asked on any repository.
    pub fn asked(&self, prefix: &str) -> bool {
        self.calls.borrow().iter().any(|call| {
            call.split_once(": ")
                .is_some_and(|(_, verb)| verb.starts_with(prefix))
        })
    }
}

impl Repos for FakeRepos {
    fn at(&self, root: &Path) -> Rc<dyn Repo> {
        Rc::new(FakeRepo {
            repos: self.clone(),
            root: root.to_path_buf(),
        })
    }
}

/// One repository of a [`FakeRepos`].
pub struct FakeRepo {
    repos: FakeRepos,
    root: PathBuf,
}

impl FakeRepo {
    fn pushed(&self, verb: &str, branch: &str) -> Ran {
        let mut ran = self.note(verb);
        if self.repos.rejects_push {
            ran.code = Some(1);
            ran.stderr = format!(
                " ! [rejected]        {branch} -> {branch} (fetch first)\n\
                 error: failed to push some refs\n\
                 hint: Updates were rejected because the remote contains work that you do not"
            );
        }
        ran
    }

    fn note(&self, verb: &str) -> Ran {
        self.repos
            .calls
            .borrow_mut()
            .push(format!("{}: {verb}", self.root.display()));
        let broke = self.repos.failing.iter().any(|f| verb.starts_with(f));
        Ran {
            code: Some(i32::from(broke)),
            stdout: String::new(),
            stderr: if broke {
                format!("fatal: {verb} failed")
            } else {
                String::new()
            },
        }
    }
}

#[async_trait(?Send)]
impl Repo for FakeRepo {
    async fn current_branch(&self) -> Outcome<String> {
        Ok("main_agent".to_string())
    }
    async fn head_sha(&self) -> Outcome<String> {
        Ok("abc1234".to_string())
    }
    async fn dirty_files(&self) -> Outcome<Vec<String>> {
        let conflicts = self.repos.current_conflicts();
        if conflicts.is_empty() {
            return Ok(self
                .repos
                .rewritten
                .iter()
                .map(|path| format!("M  {path}"))
                .collect());
        }
        Ok(conflicts
            .iter()
            .map(|(code, path)| format!("{code} {path}"))
            .collect())
    }
    async fn has_branch(&self, name: &str) -> Outcome<bool> {
        Ok(self
            .repos
            .branches
            .as_ref()
            .is_none_or(|known| known.0.iter().any(|b| b == name)))
    }
    async fn origin_has_branch(&self, name: &str) -> Outcome<bool> {
        Ok(self
            .repos
            .branches
            .as_ref()
            .is_none_or(|known| known.1.iter().any(|b| b == name)))
    }
    async fn remote_url(&self, _remote: &str) -> Outcome<String> {
        Ok(String::new())
    }
    async fn tracked_files(&self) -> Outcome<Vec<String>> {
        Ok(Vec::new())
    }
    async fn default_branch(&self) -> Outcome<String> {
        Ok("main".to_string())
    }
    async fn local_branches(&self) -> Outcome<Vec<String>> {
        Ok(Vec::new())
    }
    async fn stashes(&self) -> Outcome<Vec<String>> {
        Ok(Vec::new())
    }
    async fn unpushed(&self) -> Outcome<Vec<String>> {
        Ok(Vec::new())
    }
    async fn branches_at_risk(&self, _upstream: &str) -> Outcome<Vec<String>> {
        Ok(Vec::new())
    }
    async fn clone_repo(&self, url: &str, name: &str) -> Outcome<Ran> {
        Ok(self.note(&format!("clone {url} {name}")))
    }
    async fn fetch(&self) -> Outcome<Ran> {
        Ok(self.note("fetch"))
    }
    async fn checkout(&self, branch: &str, force: bool) -> Outcome<Ran> {
        Ok(self.note(&format!(
            "checkout {branch}{}",
            if force { " --force" } else { "" }
        )))
    }
    async fn reset_hard(&self, reference: &str) -> Outcome<Ran> {
        Ok(self.note(&format!("reset_hard {reference}")))
    }
    async fn changed_between(&self, from: &str, to: &str) -> Outcome<Vec<String>> {
        self.note(&format!("diff {from}..{to}"));
        Ok(self.repos.changed.as_ref().clone())
    }
    async fn sparse_checkout(&self, patterns: &[String]) -> Outcome<Ran> {
        Ok(self.note(&format!("sparse-checkout set {}", patterns.len())))
    }
    async fn sparse_checkout_off(&self) -> Outcome<Ran> {
        Ok(self.note("sparse-checkout disable"))
    }
    async fn clean(&self) -> Outcome<Ran> {
        Ok(self.note("clean"))
    }
    async fn delete_branch(&self, name: &str) -> Outcome<Ran> {
        Ok(self.note(&format!("delete_branch {name}")))
    }
    async fn create_local_branch(&self, name: &str, from: &str) -> Outcome<Ran> {
        Ok(self.note(&format!("create_local_branch {name} {from}")))
    }
    async fn stage_all(&self) -> Outcome<Ran> {
        Ok(self.note("stage_all"))
    }
    async fn commit(&self, message: &str) -> Outcome<Ran> {
        self.repos.in_merge.set(false);
        Ok(self.note(&format!("commit {message}")))
    }
    async fn push(&self, branch: &str) -> Outcome<Ran> {
        Ok(self.pushed(&format!("push {branch}"), branch))
    }
    async fn push_head(&self, branch: &str) -> Outcome<Ran> {
        Ok(self.pushed(&format!("push HEAD:{branch}"), branch))
    }
    async fn create_orphan_branch(&self, name: &str) -> Outcome<Ran> {
        Ok(self.note(&format!("create_orphan_branch {name}")))
    }
    async fn merge(&self, reference: &str) -> Outcome<Ran> {
        let mut ran = self.note(&format!("merge {reference}"));
        self.repos
            .merges
            .set(self.repos.merges.get().saturating_add(1));
        if !self.repos.current_conflicts().is_empty() {
            ran.code = Some(1);
            ran.stdout = "CONFLICT (content): Merge conflict".to_string();
            self.repos.in_merge.set(true);
        }
        Ok(ran)
    }
    async fn take_theirs(&self, path: &str) -> Outcome<Ran> {
        Ok(self.note(&format!("take_theirs {path}")))
    }
    async fn remove(&self, path: &str) -> Outcome<Ran> {
        Ok(self.note(&format!("remove {path}")))
    }
    async fn merge_in_progress(&self) -> Outcome<bool> {
        Ok(self.repos.in_merge.get())
    }
    async fn abort_merge(&self) -> Outcome<Ran> {
        self.repos.in_merge.set(false);
        Ok(self.note("abort_merge"))
    }
    async fn merge_base(&self, a: &str, b: &str) -> Outcome<Option<String>> {
        self.note(&format!("merge_base {a} {b}"));
        Ok(self.repos.base.clone())
    }
    async fn show(&self, spec: &str) -> Outcome<Option<String>> {
        let ran = self.note(&format!("show {spec}"));
        Ok(ran
            .ok()
            .then(|| self.repos.shown.get(spec).cloned())
            .flatten())
    }
    async fn stage_paths(&self, paths: &[String]) -> Outcome<Ran> {
        Ok(self.note(&format!("stage_paths {}", paths.join(" "))))
    }
    async fn merge_file(&self, ours: &Path, base: &Path, theirs: &Path) -> Outcome<Ran> {
        let noted = self.note(&format!(
            "merge_file {} {} {}",
            ours.display(),
            base.display(),
            theirs.display()
        ));
        if !noted.ok() {
            // An error, never a count of conflicts.
            return Ok(Ran {
                code: Some(255),
                ..noted
            });
        }
        let mut answers = self.repos.merged_files.borrow_mut();
        Ok(if answers.is_empty() {
            Ran {
                code: Some(0),
                stdout: format!("merged {}", ours.display()),
                stderr: String::new(),
            }
        } else {
            answers.remove(0)
        })
    }
}
