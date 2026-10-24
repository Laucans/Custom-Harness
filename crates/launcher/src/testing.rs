//! The launcher's own test doubles, shared by its test modules: what is
//! tested in this crate is what remains specific to it — the wiring, the
//! arguments, the console — never a workflow, which `harness-workflows`
//! tests with its own fakes.

use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::github::GitHub;

/// A GitHub no call should reach: every method refuses, so a test that
/// takes it proves the code under test read nothing from the board.
pub struct NoGitHub;

#[async_trait::async_trait(?Send)]
impl GitHub for NoGitHub {
    async fn authenticated(&self) -> Outcome<bool> {
        Err(refused())
    }
    async fn repo(&self) -> Outcome<String> {
        Err(refused())
    }
    async fn labels(&self) -> Outcome<Vec<String>> {
        Err(refused())
    }
    async fn issue(&self, _number: u64) -> Outcome<harness_core::domain::Issue> {
        Err(refused())
    }
    async fn issues_labelled(
        &self,
        _label: &str,
        _state: &str,
    ) -> Outcome<Vec<harness_core::domain::Issue>> {
        Err(refused())
    }
    async fn activity(
        &self,
        _etag: Option<&str>,
    ) -> Outcome<harness_core::ports::shell::github::Activity> {
        Err(refused())
    }
    async fn sub_issues(&self, _number: u64) -> Outcome<Vec<harness_core::domain::Issue>> {
        Err(refused())
    }
    async fn blocked_by(&self, _number: u64) -> Outcome<Vec<harness_core::domain::Issue>> {
        Err(refused())
    }
    async fn with_blockers(
        &self,
        _tasks: Vec<harness_core::domain::Issue>,
    ) -> Outcome<Vec<harness_core::domain::Issue>> {
        Err(refused())
    }
    async fn merged_prs(&self, _base: &str) -> Outcome<Vec<harness_core::domain::Issue>> {
        Err(refused())
    }
    async fn issue_comments(&self, _number: u64) -> Outcome<Vec<String>> {
        Err(refused())
    }
    async fn add_label(&self, _number: u64, _label: &str) -> Outcome<()> {
        Err(refused())
    }
    async fn remove_label(&self, _number: u64, _label: &str) -> Outcome<()> {
        Err(refused())
    }
    async fn set_body(&self, _number: u64, _body: &str) -> Outcome<()> {
        Err(refused())
    }
    async fn post_issue_comment(&self, _number: u64, _body: &str) -> Outcome<()> {
        Err(refused())
    }
    async fn close_issue(&self, _number: u64) -> Outcome<()> {
        Err(refused())
    }
    async fn pr(&self, _pr_ref: &str) -> Outcome<harness_core::domain::Pr> {
        Err(refused())
    }
    async fn pr_files(&self, _pr_ref: &str) -> Outcome<Vec<String>> {
        Err(refused())
    }
    async fn pr_comments(&self, _num: &str) -> Outcome<String> {
        Err(refused())
    }
    async fn post_pr_comment(&self, _num: &str, _body_file: &std::path::Path) -> Outcome<()> {
        Err(refused())
    }
    async fn create_label(&self, _name: &str, _color: &str, _description: &str) -> Outcome<()> {
        Err(refused())
    }
    async fn branch_sha(&self, _branch: &str) -> Outcome<Option<String>> {
        Err(refused())
    }
    async fn branches(&self, _prefix: &str) -> Outcome<Vec<String>> {
        Err(refused())
    }
    async fn create_branch(&self, _branch: &str, _sha: &str) -> Outcome<()> {
        Err(refused())
    }
    async fn merge_branch(&self, _base: &str, _head: &str) -> Outcome<()> {
        Err(refused())
    }
    async fn changed_files(&self, _base: &str, _head: &str) -> Outcome<Option<Vec<String>>> {
        Err(refused())
    }
    async fn default_branch(&self) -> Outcome<String> {
        Err(refused())
    }
    async fn set_default_branch(&self, _branch: &str) -> Outcome<()> {
        Err(refused())
    }
    async fn protect_branch(&self, _branch: &str) -> Outcome<()> {
        Err(refused())
    }
    async fn require_squash_merges(&self) -> Outcome<()> {
        Err(refused())
    }
    async fn can_push(&self) -> Outcome<bool> {
        Err(refused())
    }
    async fn file_text(&self, _path: &str, _git_ref: &str) -> Outcome<Option<String>> {
        Err(refused())
    }
    async fn put_file(
        &self,
        _path: &str,
        _branch: &str,
        _content: &str,
        _message: &str,
    ) -> Outcome<()> {
        Err(refused())
    }
    async fn create_issue(&self, _title: &str, _body: &str, _labels: &[&str]) -> Outcome<u64> {
        Err(refused())
    }
    async fn create_sub_issue_link(&self, _parent: u64, _child: u64) -> Outcome<()> {
        Err(refused())
    }
    async fn add_blocked_by(&self, _number: u64, _blocker: u64) -> Outcome<()> {
        Err(refused())
    }
    async fn create_pr(
        &self,
        _head: &str,
        _base: &str,
        _title: &str,
        _body: &str,
    ) -> Outcome<String> {
        Err(refused())
    }
    async fn merge_pr(&self, _pr_ref: &str) -> Outcome<()> {
        Err(refused())
    }
    async fn pr_checks_green(&self, _pr_ref: &str) -> Outcome<bool> {
        Err(refused())
    }
    async fn open_prs_labelled(&self, _label: &str) -> Outcome<Vec<harness_core::domain::Pr>> {
        Err(refused())
    }
    async fn pr_failing_checks(&self, _pr_ref: &str) -> Outcome<Vec<String>> {
        Err(refused())
    }

    async fn pr_mergeable(&self, _pr_ref: &str) -> Outcome<Option<bool>> {
        Err(refused())
    }

    async fn branch_checks(&self, _branch: &str) -> Outcome<Option<bool>> {
        Ok(None)
    }

    async fn commit_checks(&self, _branch: &str, _sha: &str) -> Outcome<Option<bool>> {
        Ok(None)
    }
}

/// What every method of [`NoGitHub`] answers.
pub fn refused() -> Halt {
    Halt::Failed("no call should leave this test".to_string())
}
