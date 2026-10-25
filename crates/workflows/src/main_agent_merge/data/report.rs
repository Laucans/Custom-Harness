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
    /// A PR exists, but a check is still running: nothing done, tried
    /// again at a later tick.
    WaitingOnChecks,
    /// A PR exists and a check concluded in failure: nothing done, and
    /// nothing will be until a push turns it green — told, so a red
    /// milestone PR is not mistaken for one still running.
    Red {
        /// The pull request, `#99`.
        pr: String,
        /// The failing checks, one readable line each.
        checks: Vec<String>,
    },
    /// A PR red past its repairs: nothing merged, no repair asked — a
    /// decision on the milestone was asked (`milestone_merge::escalate`), or
    /// a human holds it. Told once, when asked.
    Escalated {
        /// The pull request, `#99`.
        pr: String,
        /// The failing checks, one readable line each.
        checks: Vec<String>,
        /// Whether the decision was asked now (`false`: asked already, or a
        /// human's, or the owner answered, or a wait stands).
        asked: bool,
        /// The issues its decided waits named, landed: the PR was resumed
        /// on them now instead (`milestone_merge::Escalation::Resumed`).
        resumed: Vec<u64>,
    },
    /// A PR's checks were green, but GitHub says it conflicts with the base,
    /// and the base could not be merged into the milestone branch: nothing
    /// merged, told every tick it holds — a catch-up resolves it, never a
    /// halt.
    Conflicting {
        /// The pull request, `#99`.
        pr: String,
        /// Why the base could not be merged in, in GitHub's own words — a
        /// conflict, or a failure to ask that the line must not hide.
        why: String,
    },
    /// A PR's checks were green, and whether it can merge could not be read:
    /// nothing merged, told every tick it holds, never a halt — a read that
    /// keeps failing must not leave a green milestone waiting in silence.
    MergeabilityUnread {
        /// The pull request, `#99`.
        pr: String,
        /// The failed read, in its own words.
        why: String,
    },
    /// A PR was green and mergeable, and GitHub refused the merge all the
    /// same: nothing merged, told as a warning, tried again at a later tick.
    MergeRefused {
        /// The pull request, `#99`.
        pr: String,
        /// GitHub's refusal, in its own words.
        why: String,
    },
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
        /// Why the milestone's native GitHub milestone could not be closed
        /// with it — a warning, the landing done all the same.
        native: Option<String>,
    },
}
