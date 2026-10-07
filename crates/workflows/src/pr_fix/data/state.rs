//! What the repair's stages pass to each other.

use harness_core::domain::Pr;

/// The state of a repair attempt.
#[derive(Debug, Clone, Default)]
pub struct FixState {
    /// The PR being repaired, set by precheck.
    pub pr: Option<Pr>,
    /// The checks that have concluded in failure, one readable line each —
    /// set by the free context stage, and the reason the paid stage runs at
    /// all.
    pub failing: Vec<String>,
    /// The PR's comments, as one blob — a review, or a human, may already
    /// have said what is wrong.
    pub comments: String,
}

impl FixState {
    /// The PR, or a programming failure if reached before `precheck`.
    ///
    /// # Panics
    /// If called before `precheck` sets it — something no path in
    /// [`Workflow::execute`](harness_core::execution::Workflow::execute)
    /// allows.
    #[must_use]
    pub const fn pr(&self) -> &Pr {
        self.pr
            .as_ref()
            .expect("precheck must set the PR before the stages run")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_has_no_pr_and_nothing_broken_yet() {
        let state = FixState::default();
        assert!(state.pr.is_none());
        assert!(state.failing.is_empty());
    }
}
