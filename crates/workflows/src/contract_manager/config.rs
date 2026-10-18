//! What this run changes for an evolution, by value.

use std::path::PathBuf;
use std::rc::Rc;

use harness_core::domain::prompt_book::PromptBook;
use harness_core::ports::agent::SessionSpec;

/// The settings of an evolution.
pub struct Config {
    /// The requests' lock directory — one evolution per request at a time.
    pub lock_dir: PathBuf,
    /// The integration branch the contracts and the graph are written on.
    pub base_branch: String,
    /// Model and effort for the session that evolves the contract.
    pub evolve: SessionSpec,
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
    pub fn in_dir(lock_dir: PathBuf) -> Config {
        Config {
            prompts: std::rc::Rc::default(),
            lock_dir,
            base_branch: "main_agent".to_string(),
            evolve: SessionSpec {
                model: "opus".to_string(),
                effort: "high".to_string(),
            },
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/contract-manager"))
    }
}
