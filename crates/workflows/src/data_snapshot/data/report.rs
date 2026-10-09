//! What one data snapshot did, branch by branch.

/// One branch's head, as the snapshot left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snapped {
    /// A new version, archived in this folder.
    Archived {
        /// The branch.
        branch: String,
        /// Its head.
        sha: String,
        /// The folder on the archive branch.
        dir: String,
        /// Both sides were there to compare.
        compared: bool,
        /// Where the dump and the agents' model disagree; empty when they
        /// agree, or when one side is missing.
        divergences: Vec<String>,
    },
    /// The same dump and model as the branch's last version: only the head
    /// moved in the index.
    Unchanged {
        /// The branch.
        branch: String,
        /// Its head.
        sha: String,
    },
    /// Not archived; tried again when the board moves.
    Failed {
        /// The branch.
        branch: String,
        /// Its head.
        sha: String,
        /// Why.
        reason: String,
    },
}

/// Everything one run did; empty when every head was already archived.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// One line per branch whose head was new.
    pub snapped: Vec<Snapped>,
}
