//! What this run changes for an arbitration, by value.

use std::path::PathBuf;
use std::rc::Rc;

use harness_core::domain::prompt_book::PromptBook;
use harness_core::ports::agent::SessionSpec;

/// The settings of an arbitration.
pub struct Config {
    /// Where the locks live — one per task.
    pub arbiter_dir: PathBuf,
    /// The checkout's root: where the repository's manifests are read.
    pub root: PathBuf,
    /// Model and effort for the session.
    pub arbitrate: SessionSpec,
    /// The human's prompt edits, read once per run.
    pub prompts: Rc<PromptBook>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config at `sonnet`/`high`.

    use std::path::PathBuf;

    use harness_core::ports::agent::SessionSpec;

    use super::Config;

    /// A test config, locks under this folder.
    pub fn in_dir(arbiter_dir: PathBuf) -> Config {
        Config {
            prompts: std::rc::Rc::default(),
            root: PathBuf::from("/repo"),
            arbiter_dir,
            arbitrate: SessionSpec {
                model: "sonnet".to_string(),
                effort: "high".to_string(),
            },
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/arbiter"))
    }
}
