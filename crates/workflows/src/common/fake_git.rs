//! An in-memory `git`, shared by workflows that need one for their tests.
//!
//! A fake adapter, not a mock: it answers what a test told it and records
//! every verb it was asked, with the repository it was asked on. `init-repo`
//! uses it to prove a clone was made, files committed, a branch pushed —
//! and that a dry run asked for none of it.

use std::cell::RefCell;
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
}

impl FakeRepos {
    /// A `git` whose verbs starting with one of these prefixes fail.
    pub fn failing(prefixes: &[&'static str]) -> Self {
        Self {
            calls: Rc::default(),
            failing: Rc::new(prefixes.to_vec()),
        }
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
        Ok(Vec::new())
    }
    async fn has_branch(&self, _name: &str) -> Outcome<bool> {
        Ok(true)
    }
    async fn origin_has_branch(&self, _name: &str) -> Outcome<bool> {
        Ok(true)
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
        Ok(self.note(&format!("commit {message}")))
    }
    async fn push(&self, branch: &str) -> Outcome<Ran> {
        Ok(self.note(&format!("push {branch}")))
    }
}
