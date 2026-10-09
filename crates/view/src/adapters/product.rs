//! The product's files: on GitHub, or in a local folder the human names
//! (`--data-dir`, and the sample `--demo` shows).

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::github::GitHub;
use harness_workflows::data_snapshot::config::ARCHIVE_BRANCH;

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
        Ok(if self.slug.is_empty() {
            format!("#{number}")
        } else {
            format!("https://github.com/{}/issues/{number}", self.slug)
        })
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
}
