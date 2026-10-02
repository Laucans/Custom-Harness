//! What this run changes for a review, by value.
//!
//! Separated from the core's `Settings` on purpose: `Settings` carries what
//! **every** workflow has — `--dry-run`, `--stages` — and the core reads it
//! from the `Context`. What is here belongs only to the review.
//!
//! Also separated from [`crate::pr_review::ports::Ports`]: this is only
//! values, never an injected capability.

use std::path::PathBuf;

use harness_core::adapters::agent::SessionSpec;

/// The settings for a review.
pub struct Config {
    /// A review's folder — kept comment, registry, lock.
    pub review_dir: PathBuf,
    /// The level of `/code-review` for the "inline" pass.
    pub level: String,
    /// `--no-inline`: the summary comment, and nothing else.
    pub no_inline: bool,
    /// Model and effort for the "inline" pass.
    pub inline: SessionSpec,
    /// Model and effort for the "brief" pass.
    pub brief: SessionSpec,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config: both passes in `sonnet`, level `medium`.

    use std::path::PathBuf;

    use harness_core::adapters::agent::SessionSpec;

    use super::Config;

    /// A test config.
    pub fn config() -> Config {
        let spec = SessionSpec {
            model: "sonnet".to_string(),
            effort: "medium".to_string(),
        };
        Config {
            review_dir: PathBuf::from("/tmp/review"),
            level: "medium".to_string(),
            no_inline: false,
            inline: spec.clone(),
            brief: spec,
        }
    }
}
