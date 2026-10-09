//! The draft's state, stage to stage.

use std::path::PathBuf;

/// What `prepare` sets up and `publish` reads.
#[derive(Debug, Clone, Default)]
pub struct DraftState {
    /// The draft clone.
    pub clone: PathBuf,
    /// The base branch's head, the draft starts from it.
    pub sha: String,
    /// The branch the draft is committed on.
    pub branch: String,
    /// Where the dump was written, outside the clone.
    pub schema_path: PathBuf,
    /// The dump.
    pub schema: String,
    /// The pull request opened, once it is.
    pub pr: Option<String>,
    /// Where the drafted model still disagrees with the dump.
    pub divergences: Vec<String>,
}

/// The draft branch for `base`, one per base: `harness/model-draft/<base>`.
#[must_use]
pub fn draft_branch(base: &str) -> String {
    format!("harness/model-draft/{base}")
}

/// Where the dump of `base` is written: under the workdir, slashes flattened.
#[must_use]
pub fn dump_name(base: &str) -> String {
    format!("{}.schema.sql", base.replace('/', "__"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_is_named_after_its_base() {
        assert_eq!(draft_branch("main_agent"), "harness/model-draft/main_agent");
        assert_eq!(dump_name("milestone/3-x"), "milestone__3-x.schema.sql");
    }
}
