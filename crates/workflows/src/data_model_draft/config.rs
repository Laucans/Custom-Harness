//! What a draft needs to know, by value.

use std::path::PathBuf;
use std::time::Duration;

use harness_core::domain::Slug;
use harness_core::ports::agent::SessionSpec;

/// The settings for a draft.
pub struct Config {
    /// The target repository.
    pub slug: Slug,
    /// The branch the model is drafted for, and the pull request's base.
    pub base: String,
    /// Where the draft clone lives — one folder per repository, reused —
    /// and where the dump is written, outside the clone.
    pub workdir: PathBuf,
    /// The target's dump script, relative to its root.
    pub script: String,
    /// The longest a dump may take.
    pub timeout: Duration,
    /// The model and effort the session opens with.
    pub spec: SessionSpec,
}
