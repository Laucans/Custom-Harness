//! What this run changes for a split round, by value.
//!
//! Separated from the core's `Settings` on purpose: `Settings` carries what
//! **every** workflow has — `--dry-run`, `--stages` — and the core reads it
//! from the `Context`. What is here belongs only to split.
//!
//! Also separated from [`crate::split::ports::Ports`]: this is only values,
//! never an injected capability.

use std::path::PathBuf;

use harness_core::ports::agent::SessionSpec;

/// The settings for a split round.
pub struct Config {
    /// This milestone's lock directory — one split run at a time.
    pub split_dir: PathBuf,
    /// The branch the milestone's own branch is created from, if it does
    /// not exist yet — `main_agent`.
    pub base_branch: String,
    /// Model and effort for the slice step.
    pub slice: SessionSpec,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config, disposable folder.

    use std::path::PathBuf;

    use harness_core::ports::agent::SessionSpec;

    use super::Config;

    /// A test config, under this folder.
    pub fn in_dir(split_dir: PathBuf) -> Config {
        Config {
            split_dir,
            base_branch: "main_agent".to_string(),
            slice: SessionSpec {
                model: "sonnet".to_string(),
                effort: "high".to_string(),
            },
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/split"))
    }
}
