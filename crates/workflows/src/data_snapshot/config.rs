//! What a data snapshot needs to know, by value.

use std::path::PathBuf;
use std::time::Duration;

use harness_core::domain::Slug;

/// The settings for a data snapshot.
pub struct Config {
    /// The target repository.
    pub slug: Slug,
    /// Where the snapshot clone lives — one folder per repository under it,
    /// reused — and where the default script is written.
    pub workdir: PathBuf,
    /// The archive branch: `harness/data-model`.
    pub archive_branch: String,
    /// The target's dump script, relative to its root.
    pub script: String,
    /// The longest a dump may take.
    pub timeout: Duration,
}

/// The archive branch's name.
pub const ARCHIVE_BRANCH: &str = "harness/data-model";

/// Where the target keeps its dump script.
pub const SCRIPT: &str = "scripts/dump-schema.sh";

#[cfg(test)]
pub(crate) mod fake {
    //! A test config: `o/r`, under `/tmp/snap`.

    use std::path::PathBuf;
    use std::time::Duration;

    use harness_core::domain::Slug;

    use super::{ARCHIVE_BRANCH, Config, SCRIPT};

    /// A test config.
    pub fn config() -> Config {
        Config {
            slug: Slug {
                owner: "o".to_string(),
                name: "r".to_string(),
            },
            workdir: PathBuf::from("/tmp/snap"),
            archive_branch: ARCHIVE_BRANCH.to_string(),
            script: SCRIPT.to_string(),
            timeout: Duration::from_secs(300),
        }
    }
}
