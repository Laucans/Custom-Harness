//! The product's files: on GitHub at the integration branch, or in a local
//! folder the human names (`--data-dir`, and the sample `--demo` shows).

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::github::GitHub;

use crate::ports::Product;

/// The product's repository on GitHub, read at one branch.
pub struct GhProduct {
    gh: Rc<dyn GitHub>,
    slug: String,
    branch: String,
}

impl GhProduct {
    /// `slug` names the repository in the origin line; `branch` is where the
    /// agents' work lands.
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
            format!("this repository @ {}", self.branch)
        } else {
            format!("{} @ {}", self.slug, self.branch)
        }
    }

    async fn file(&self, path: &str) -> Outcome<Option<String>> {
        self.gh.file_text(path, &self.branch).await
    }
}

/// The product's files in a folder: `<dir>/data/schema.sql` and its sibling.
pub struct DirProduct {
    dir: PathBuf,
}

impl DirProduct {
    /// Reads under `dir`, the product's root.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }
}

#[async_trait(?Send)]
impl Product for DirProduct {
    fn origin(&self) -> String {
        self.dir.display().to_string()
    }

    async fn file(&self, path: &str) -> Outcome<Option<String>> {
        let rel = Path::new(path);
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(Halt::Unreadable(format!(
                "{path}: not a path under the product"
            )));
        }
        let full = self.dir.join(rel);
        match fs::read_to_string(&full) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Halt::Unreadable(format!("{}: {e}", full.display()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_folder_answers_present_absent_and_refuses_to_leave() {
        let dir = std::env::temp_dir().join(format!("harness-product-{}", std::process::id()));
        fs::create_dir_all(dir.join("data")).expect("temp dir");
        fs::write(dir.join("data/schema.sql"), "CREATE TABLE t (a int);").expect("written");
        let product = DirProduct::new(&dir);
        assert_eq!(
            product
                .file("data/schema.sql")
                .await
                .expect("read")
                .as_deref(),
            Some("CREATE TABLE t (a int);")
        );
        assert_eq!(product.file("data/model.json").await.expect("read"), None);
        assert!(product.file("../secret").await.is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
