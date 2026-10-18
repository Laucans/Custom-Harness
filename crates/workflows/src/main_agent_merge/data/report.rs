//! What one merge attempt resolves to.

/// What happened on one attempt.
///
/// Distinct from [`harness_core::domain::Verdict`] (no `Skip`/`NothingLeft`
/// — those don't apply to this idempotent, two-tick command). Never
/// re-exported past `main_agent_merge::data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Tasks remain open — nothing attempted.
    NotReady,
    /// Every task delivered, but the producers of what this milestone reads
    /// (the plan's graph) have not landed yet: the train keeps its order.
    WaitingOnProducers(Vec<u64>),
    /// No PR existed yet for the milestone branch; one was opened, at this
    /// URL. CI has not had a chance to run; merging waits for a later tick.
    OpenedPr(String),
    /// A PR exists, but its checks haven't all succeeded yet.
    WaitingOnChecks,
    /// A PR's checks were green; it was merged, and the milestone is now
    /// `harness:waiting-merge`.
    Merged {
        /// The pull request merged, `#99`.
        pr: String,
        /// The roadmap item every milestone of which has now landed, if this
        /// one was its last.
        roadmap: Option<u64>,
        /// The ids the landing built, now marked so in the plan's graph.
        built: Vec<String>,
        /// The milestones still building whose branch took the landing.
        caught_up: Vec<u64>,
    },
}
