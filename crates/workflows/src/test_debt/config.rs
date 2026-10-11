//! What this run changes for a test-debt attempt, by value.
//!
//! Separated from the core's `Settings` on purpose: `Settings` carries what
//! **every** workflow has — `--dry-run`, `--stages` — and the core reads it
//! from the `Context`. What is here belongs only to the test debt.
//!
//! Also separated from [`crate::test_debt::ports::Ports`]: this is only
//! values, never an injected capability.

use std::path::PathBuf;
use std::rc::Rc;

use harness_core::domain::prompt_book::PromptBook;
use harness_core::ports::agent::SessionSpec;

/// The settings for a test-debt attempt.
pub struct Config {
    /// The issues' lock directory — one attempt per issue at a time.
    pub debt_dir: PathBuf,
    /// The integration branch: what the checkout is on, and what the tests'
    /// pull request targets.
    pub branch: String,
    /// Model and effort for the session that writes the tests.
    pub cover: SessionSpec,
    /// The human's prompt edits, read once per run — a stage sends
    /// `prompts.text(KEY, DEFAULT)` (see [`crate::prompts`]).
    pub prompts: Rc<PromptBook>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config, disposable folder.

    use std::path::PathBuf;

    use harness_core::ports::agent::SessionSpec;

    use super::Config;

    /// A test config, under this folder.
    pub fn in_dir(debt_dir: PathBuf) -> Config {
        Config {
            prompts: std::rc::Rc::default(),
            debt_dir,
            branch: "main_agent".to_string(),
            cover: SessionSpec {
                model: "sonnet".to_string(),
                effort: "high".to_string(),
            },
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/test-debt"))
    }
}
