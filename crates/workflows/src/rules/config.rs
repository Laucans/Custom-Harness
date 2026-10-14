//! What this run changes for the rules' task, by value.

use std::path::PathBuf;
use std::rc::Rc;

use harness_core::domain::prompt_book::PromptBook;
use harness_core::ports::agent::SessionSpec;

/// The settings of the rules' task.
pub struct Config {
    /// The issues' lock directory — one attempt per issue at a time.
    pub rules_dir: PathBuf,
    /// The integration branch: what the checkout is on, and what the rules'
    /// pull request targets.
    pub branch: String,
    /// Model and effort for the session that writes the rules' tests.
    pub write: SessionSpec,
    /// Model and effort for the session that judges them.
    pub fidelity: SessionSpec,
    /// The human's prompt edits, read once per run (see [`crate::prompts`]).
    pub prompts: Rc<PromptBook>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config, disposable folder.

    use std::path::PathBuf;

    use harness_core::ports::agent::SessionSpec;

    use super::Config;

    /// A test config, under this folder.
    pub fn in_dir(rules_dir: PathBuf) -> Config {
        let spec = SessionSpec {
            model: "sonnet".to_string(),
            effort: "high".to_string(),
        };
        Config {
            prompts: std::rc::Rc::default(),
            rules_dir,
            branch: "main_agent".to_string(),
            write: spec.clone(),
            fidelity: spec,
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/rules"))
    }
}
