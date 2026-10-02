//! An in-memory `GitHub`, shared by workflows that need it for their tests.
//!
//! A fake adapter, not a mock: it responds from `Issue`/`Pr` given to it,
//! and writes re-read. This is what `CLAUDE.md` asks for — "inject a fake
//! adapter; nothing mocks at the call site" — and what makes it possible to
//! exercise rules without network.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use harness_core::adapters::shell::github::GitHub;
use harness_core::domain::{Halt, Issue, Outcome, Pr};

/// What the fake recorded as a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wrote {
    /// A label placed.
    Label(u64, String),
    /// A label removed.
    Unlabelled(u64, String),
    /// A body rewritten.
    Body(u64, String),
    /// A comment posted on an issue.
    Comment(u64, String),
    /// An issue closed.
    Closed(u64),
    /// A comment posted on a PR, and the file it comes from.
    PrComment(String, String),
    /// A label created: name, color, description.
    CreatedLabel(String, String, String),
    /// A branch created: name, sha.
    CreatedBranch(String, String),
}

/// An in-memory GitHub.
#[derive(Default)]
pub struct FakeGitHub {
    /// The issues it knows, including milestones.
    pub issues: Vec<Issue>,
    /// Sub-issues, by parent number.
    pub subs: Vec<(u64, Vec<Issue>)>,
    /// Merged PRs it will return for `merged_prs`.
    pub merged: Vec<Issue>,
    /// The labels the repository carries.
    pub labels: Vec<String>,
    /// Issue comments it will return for `issue_comments`.
    pub issue_comments: Vec<String>,
    /// PRs that `pr()` can read, by number or URL requested.
    pub prs: Vec<(String, Pr)>,
    /// What `pr_comments` returns, by PR number.
    pub pr_comment_bodies: Vec<(String, String)>,
    /// What it wrote.
    pub wrote: RefCell<Vec<Wrote>>,
    /// When filled, **every** read fails with this halt.
    pub broken: Option<Halt>,
    /// `branch_sha` answers, by branch name. An absent key refuses — the
    /// test must set up exactly what it reads.
    pub branch_shas: HashMap<String, Option<String>>,
    /// `file_text` answers, by `(path, git_ref)`. An absent key is `None` —
    /// a 404, not a refusal, since "the file is absent" is itself a valid
    /// test scenario.
    pub files: HashMap<(String, String), String>,
    /// `default_branch`'s answer. `None` refuses — the test must set it up.
    pub default_branch_name: Option<String>,
    /// `can_push`'s answer. `None` refuses — the test must set it up.
    pub can_push_answer: Option<bool>,
}

impl FakeGitHub {
    fn ok(&self) -> Outcome<()> {
        self.broken
            .as_ref()
            .map_or(Ok(()), |halt| Err(halt.clone()))
    }

    /// The writes, in order.
    pub fn writes(&self) -> Vec<Wrote> {
        self.wrote.borrow().clone()
    }
}

#[async_trait(?Send)]
impl GitHub for FakeGitHub {
    async fn authenticated(&self) -> Outcome<bool> {
        self.ok()?;
        Ok(true)
    }

    async fn repo(&self) -> Outcome<String> {
        self.ok()?;
        Ok("owner/repo".to_string())
    }

    async fn labels(&self) -> Outcome<Vec<String>> {
        self.ok()?;
        Ok(self.labels.clone())
    }

    async fn issue(&self, number: u64) -> Outcome<Issue> {
        self.ok()?;
        self.issues
            .iter()
            .find(|issue| issue.number == number)
            .cloned()
            .ok_or_else(|| Halt::Unreadable(format!("no issue #{number}")))
    }

    async fn issues_labelled(&self, label: &str, _state: &str) -> Outcome<Vec<Issue>> {
        self.ok()?;
        Ok(self
            .issues
            .iter()
            .filter(|issue| issue.has(label))
            .cloned()
            .collect())
    }

    async fn sub_issues(&self, number: u64) -> Outcome<Vec<Issue>> {
        self.ok()?;
        Ok(self
            .subs
            .iter()
            .find(|(parent, _)| *parent == number)
            .map(|(_, subs)| subs.clone())
            .unwrap_or_default())
    }

    async fn blocked_by(&self, number: u64) -> Outcome<Vec<Issue>> {
        self.ok()?;
        // Blockers are already carried by the issue in the fake: what we want
        // to exercise is the rule, not the API shape.
        Ok(self
            .subs
            .iter()
            .flat_map(|(_, subs)| subs.iter())
            .find(|issue| issue.number == number)
            .map(|issue| issue.blocked_by.clone())
            .unwrap_or_default())
    }

    async fn with_blockers(&self, tasks: Vec<Issue>) -> Outcome<Vec<Issue>> {
        self.ok()?;
        Ok(tasks)
    }

    async fn merged_prs(&self, _base: &str) -> Outcome<Vec<Issue>> {
        self.ok()?;
        Ok(self.merged.clone())
    }

    async fn issue_comments(&self, _number: u64) -> Outcome<Vec<String>> {
        self.ok()?;
        Ok(self.issue_comments.clone())
    }

    async fn add_label(&self, number: u64, label: &str) -> Outcome<()> {
        self.wrote
            .borrow_mut()
            .push(Wrote::Label(number, label.to_string()));
        Ok(())
    }

    async fn remove_label(&self, number: u64, label: &str) -> Outcome<()> {
        self.wrote
            .borrow_mut()
            .push(Wrote::Unlabelled(number, label.to_string()));
        Ok(())
    }

    async fn set_body(&self, number: u64, body: &str) -> Outcome<()> {
        self.wrote
            .borrow_mut()
            .push(Wrote::Body(number, body.to_string()));
        Ok(())
    }

    async fn post_issue_comment(&self, number: u64, body: &str) -> Outcome<()> {
        self.wrote
            .borrow_mut()
            .push(Wrote::Comment(number, body.to_string()));
        Ok(())
    }

    async fn close_issue(&self, number: u64) -> Outcome<()> {
        self.wrote.borrow_mut().push(Wrote::Closed(number));
        Ok(())
    }

    async fn pr(&self, pr_ref: &str) -> Outcome<Pr> {
        self.ok()?;
        self.prs
            .iter()
            .find(|(asked, _)| asked == pr_ref)
            .map(|(_, pr)| pr.clone())
            .ok_or_else(|| Halt::Failed(format!("no PR {pr_ref}")))
    }

    async fn pr_comments(&self, num: &str) -> Outcome<String> {
        self.ok()?;
        Ok(self
            .pr_comment_bodies
            .iter()
            .find(|(pr, _)| pr == num)
            .map(|(_, body)| body.clone())
            .unwrap_or_default())
    }

    async fn post_pr_comment(&self, num: &str, body_file: &Path) -> Outcome<()> {
        self.ok()?;
        self.wrote.borrow_mut().push(Wrote::PrComment(
            num.to_string(),
            body_file.display().to_string(),
        ));
        Ok(())
    }

    async fn create_label(&self, name: &str, color: &str, description: &str) -> Outcome<()> {
        self.wrote.borrow_mut().push(Wrote::CreatedLabel(
            name.to_string(),
            color.to_string(),
            description.to_string(),
        ));
        Ok(())
    }

    async fn branch_sha(&self, branch: &str) -> Outcome<Option<String>> {
        self.ok()?;
        self.branch_shas
            .get(branch)
            .cloned()
            .ok_or_else(|| Halt::Failed(format!("branch_sha({branch}) not set up in this test")))
    }

    async fn create_branch(&self, branch: &str, sha: &str) -> Outcome<()> {
        self.wrote
            .borrow_mut()
            .push(Wrote::CreatedBranch(branch.to_string(), sha.to_string()));
        Ok(())
    }

    async fn default_branch(&self) -> Outcome<String> {
        self.ok()?;
        self.default_branch_name
            .clone()
            .ok_or_else(|| Halt::Failed("default_branch not set up in this test".to_string()))
    }

    async fn can_push(&self) -> Outcome<bool> {
        self.ok()?;
        self.can_push_answer
            .ok_or_else(|| Halt::Failed("can_push not set up in this test".to_string()))
    }

    async fn file_text(&self, path: &str, git_ref: &str) -> Outcome<Option<String>> {
        self.ok()?;
        Ok(self
            .files
            .get(&(path.to_string(), git_ref.to_string()))
            .cloned())
    }
}
