//! What this run changes for a decision, by value.

use std::path::PathBuf;
use std::rc::Rc;

use harness_core::domain::prompt_book::PromptBook;
use harness_core::ports::agent::SessionSpec;

/// The settings of a decision.
pub struct Config {
    /// The issues' lock directory — one decision per issue at a time.
    pub decide_dir: PathBuf,
    /// Model and effort for the session that decides.
    pub decide: SessionSpec,
    /// The human's prompt edits, read once per run (see [`crate::prompts`]).
    pub prompts: Rc<PromptBook>,
    /// The integration branch (`main_agent`): what a decision never tells a
    /// session to merge into its branch, and the base of the PRs it never
    /// merges nor cuts.
    pub integration: String,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config, disposable folder.

    use std::path::PathBuf;

    use harness_core::ports::agent::SessionSpec;

    use super::Config;

    /// A test config, under this folder.
    pub fn in_dir(decide_dir: PathBuf) -> Config {
        Config {
            prompts: std::rc::Rc::default(),
            integration: "main_agent".to_string(),
            decide_dir,
            decide: SessionSpec {
                model: "opus".to_string(),
                effort: "high".to_string(),
            },
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/decide"))
    }
}
