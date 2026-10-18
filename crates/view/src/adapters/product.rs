//! The product's files: on GitHub, or in a local folder the human names
//! (`--data-dir`, and the sample `--demo` shows).

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Issue, Outcome};
use harness_core::ports::shell::github::{GitHub, Kept};
use harness_workflows::common::labels;
use harness_workflows::data_snapshot::config::ARCHIVE_BRANCH;
use harness_workflows::dev_loop::data::tasks::current_milestone;

use crate::ports::Product;

/// The product's repository on GitHub.
pub struct GhProduct {
    gh: Rc<dyn GitHub>,
    slug: String,
    branch: String,
}

impl GhProduct {
    /// `slug` names the repository in the origin line; `branch` is where the
    /// agents' work lands, read for the live files.
    #[must_use]
    pub fn new(gh: Rc<dyn GitHub>, slug: &str, branch: &str) -> Self {
        Self {
            gh,
            slug: slug.to_string(),
            branch: branch.to_string(),
        }
    }
}

impl GhProduct {
    /// Where an issue of the product is.
    fn url_of(&self, number: u64) -> String {
        if self.slug.is_empty() {
            format!("#{number}")
        } else {
            format!("https://github.com/{}/issues/{number}", self.slug)
        }
    }
}

#[async_trait(?Send)]
impl Product for GhProduct {
    fn origin(&self) -> String {
        if self.slug.is_empty() {
            "this repository".to_string()
        } else {
            self.slug.clone()
        }
    }

    fn live_ref(&self) -> String {
        self.branch.clone()
    }

    async fn file(&self, path: &str, git_ref: &str) -> Outcome<Option<String>> {
        self.gh.file_text(path, git_ref).await
    }

    async fn branches(&self, prefix: &str) -> Outcome<Vec<String>> {
        self.gh.branches(prefix).await
    }

    async fn open_issue(&self, title: &str, body: &str, labels: &[&str]) -> Outcome<String> {
        let number = self.gh.create_issue(title, body, labels).await?;
        Ok(self.url_of(number))
    }

    async fn rules_status(&self) -> Outcome<Option<Kept>> {
        self.gh
            .artifact_file(
                "gates.yml",
                &self.branch,
                "rules-status",
                "rules-status.json",
            )
            .await
    }

    async fn current_milestone(&self) -> Outcome<Option<u64>> {
        let open = self.gh.issues_labelled(labels::MILESTONE, "open").await?;
        Ok(current_milestone(&open).map(|issue| issue.number))
    }

    async fn open_task(
        &self,
        milestone: u64,
        title: &str,
        body: &str,
        labels: &[&str],
    ) -> Outcome<String> {
        let number = self.gh.create_issue(title, body, labels).await?;
        self.gh.create_sub_issue_link(milestone, number).await?;
        Ok(self.url_of(number))
    }

    async fn human_tasks(&self) -> Outcome<Vec<Issue>> {
        self.gh.issues_labelled(labels::HUMAN, "all").await
    }

    async fn close_issue(&self, number: u64, comment: &str) -> Outcome<()> {
        self.gh.post_issue_comment(number, comment).await?;
        self.gh.close_issue(number).await
    }

    async fn to_decide(&self, number: u64, comment: &str) -> Outcome<()> {
        self.gh.post_issue_comment(number, comment).await?;
        self.gh.add_label(number, labels::NEEDS_DECISION).await
    }
}

/// The product's files in a folder — `<dir>/data/schema.sql` and its
/// sibling — and, when given, an archive laid out as the archive branch is.
pub struct DirProduct {
    dir: PathBuf,
    archive: Option<PathBuf>,
    integration: String,
}

impl DirProduct {
    /// Reads the live files under `dir`, the product's root; the archive
    /// branch under `archive`, when there is one. `integration` names the
    /// branch the folder stands for.
    #[must_use]
    pub fn new(dir: &Path, archive: Option<&Path>, integration: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            archive: archive.map(Path::to_path_buf),
            integration: integration.to_string(),
        }
    }
}

#[async_trait(?Send)]
impl Product for DirProduct {
    fn origin(&self) -> String {
        self.dir.display().to_string()
    }

    fn live_ref(&self) -> String {
        self.integration.clone()
    }

    async fn file(&self, path: &str, git_ref: &str) -> Outcome<Option<String>> {
        let rel = Path::new(path);
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(Halt::Unreadable(format!(
                "{path}: not a path under the product"
            )));
        }
        let root = if git_ref == ARCHIVE_BRANCH {
            match &self.archive {
                Some(archive) => archive,
                None => return Ok(None),
            }
        } else {
            &self.dir
        };
        let full = root.join(rel);
        match fs::read_to_string(&full) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Halt::Unreadable(format!("{}: {e}", full.display()))),
        }
    }

    /// A folder has one set of files: no branch beside it.
    async fn branches(&self, _prefix: &str) -> Outcome<Vec<String>> {
        Ok(Vec::new())
    }

    async fn open_issue(&self, _title: &str, _body: &str, _labels: &[&str]) -> Outcome<String> {
        Err(Halt::Failed(
            "no board here: the view reads a folder, not a GitHub repository".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_folder_answers_present_absent_and_refuses_to_leave() {
        let dir = std::env::temp_dir().join(format!("harness-product-{}", std::process::id()));
        fs::create_dir_all(dir.join("data")).expect("temp dir");
        fs::create_dir_all(dir.join("archive")).expect("temp dir");
        fs::write(dir.join("data/schema.sql"), "CREATE TABLE t (a int);").expect("written");
        fs::write(dir.join("archive/index.json"), "{}").expect("written");
        let product = DirProduct::new(&dir, Some(&dir.join("archive")), "main_agent");
        assert_eq!(
            product
                .file("data/schema.sql", "x")
                .await
                .expect("read")
                .as_deref(),
            Some("CREATE TABLE t (a int);")
        );
        assert_eq!(
            product.file("data/model.json", "x").await.expect("read"),
            None
        );
        assert_eq!(
            product
                .file("index.json", ARCHIVE_BRANCH)
                .await
                .expect("read")
                .as_deref(),
            Some("{}")
        );
        assert!(product.file("../secret", "x").await.is_err());
        let bare = DirProduct::new(&dir, None, "main_agent");
        assert_eq!(
            bare.file("index.json", ARCHIVE_BRANCH).await.expect("read"),
            None
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A GitHub that answers the Ports pane's calls and writes each down;
    /// every other call is refused.
    #[derive(Default)]
    struct Recorder(std::cell::RefCell<Vec<String>>);

    impl Recorder {
        fn note(&self, line: String) {
            self.0.borrow_mut().push(line);
        }
    }

    #[async_trait::async_trait(?Send)]
    impl GitHub for Recorder {
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
            label: &str,
            state: &str,
        ) -> Outcome<Vec<harness_core::domain::Issue>> {
            self.note(format!("list {label} {state}"));
            Ok(vec![harness_core::domain::Issue {
                number: 21,
                ..harness_core::domain::Issue::default()
            }])
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
        async fn add_label(&self, number: u64, label: &str) -> Outcome<()> {
            self.note(format!("label #{number} {label}"));
            Ok(())
        }
        async fn remove_label(&self, _number: u64, _label: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn set_body(&self, _number: u64, _body: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn post_issue_comment(&self, number: u64, body: &str) -> Outcome<()> {
            self.note(format!("comment #{number} {body}"));
            Ok(())
        }
        async fn close_issue(&self, number: u64) -> Outcome<()> {
            self.note(format!("close #{number}"));
            Ok(())
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
    }

    fn refused() -> Halt {
        Halt::Failed("no call should leave this test".to_string())
    }

    #[tokio::test]
    async fn the_ports_pane_reads_the_human_tasks_closes_and_hands_over_on_github() {
        let gh = Rc::new(Recorder::default());
        let product = GhProduct::new(Rc::clone(&gh) as Rc<dyn GitHub>, "me/alpha", "main_agent");
        let human = product.human_tasks().await.expect("read");
        assert_eq!(human.len(), 1);
        product.close_issue(21, "approved").await.expect("closed");
        product.to_decide(11, "refused").await.expect("handed over");
        assert_eq!(
            gh.0.borrow().clone(),
            vec![
                format!("list {} all", labels::HUMAN),
                "comment #21 approved".to_string(),
                "close #21".to_string(),
                "comment #11 refused".to_string(),
                format!("label #11 {}", labels::NEEDS_DECISION),
            ]
        );
        let folder = DirProduct::new(Path::new("/nowhere"), None, "main_agent");
        assert!(
            folder.human_tasks().await.is_err(),
            "no board behind a folder"
        );
        assert!(folder.close_issue(1, "x").await.is_err());
        assert!(folder.to_decide(1, "x").await.is_err());
    }
}
