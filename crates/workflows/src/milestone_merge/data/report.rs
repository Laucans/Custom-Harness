//! What one merge attempt resolves to.

/// What happened on one attempt.
///
/// Distinct from [`harness_core::domain::Verdict`] (no `Skip`/`NothingLeft`
/// — those don't apply to this idempotent, two-tick command). Never
/// re-exported past `milestone_merge::data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Tasks remain open — nothing attempted.
    NotReady,
    /// No PR existed yet for the milestone branch; one was opened, at this
    /// URL. CI has not had a chance to run; merging waits for a later tick.
    OpenedPr(String),
    /// A PR exists, but its checks haven't all succeeded yet.
    WaitingOnChecks,
    /// A PR's checks were green; it was merged, and the milestone is now
    /// `harness:waiting-merge`.
    Merged,
}
