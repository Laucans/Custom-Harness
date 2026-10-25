//! What this run changes for a repair attempt, by value.
//!
//! Separated from the core's `Settings` on purpose: `Settings` carries what
//! **every** workflow has — `--dry-run`, `--stages` — and the core reads it
//! from the `Context`. What is here belongs only to the repair.
//!
//! Also separated from [`crate::pr_fix::ports::Ports`]: this is only values,
//! never an injected capability.

use std::path::PathBuf;

use harness_core::ports::agent::SessionSpec;

/// The settings for a repair attempt.
pub struct Config {
    /// This PR's lock directory — one repair per PR at a time.
    pub fix_dir: PathBuf,
    /// Model and effort for the repair step.
    pub fix: SessionSpec,
    /// The human's prompt edits, read once per run — a stage sends
    /// `prompts.text(KEY, DEFAULT)` (see [`crate::prompts`]).
    pub prompts: std::rc::Rc<harness_core::domain::prompt_book::PromptBook>,
    /// The integration branch: where the plan's graph is read, to tell a
    /// contract the PR may evolve from one another milestone produces.
    pub integration: String,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config, disposable folder.

    use std::path::PathBuf;

    use harness_core::ports::agent::SessionSpec;

    use super::Config;

    /// A test config, under this folder.
    pub fn in_dir(fix_dir: PathBuf) -> Config {
        Config {
            prompts: std::rc::Rc::default(),
            integration: "main_agent".to_string(),
            fix_dir,
            fix: SessionSpec {
                model: "sonnet".to_string(),
                effort: "high".to_string(),
            },
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/pr-fix"))
    }
}
