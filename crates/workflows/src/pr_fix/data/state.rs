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
    /// Its last agent review asks for changes, and the repairs it gets are
    /// not used up — a reason to repair even with every check green.
    pub review_blocking: bool,
    /// Its branch conflicts with its base — another PR landed there since —
    /// and the repairs are not used up: a reason to repair, by bringing the
    /// base in.
    pub conflicting: bool,
}

impl FixState {
    /// Whether anything asks for a repair at all.
    #[must_use]
    pub const fn asks_for_a_repair(&self) -> bool {
        !self.failing.is_empty() || self.review_blocking || self.conflicting
    }
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
        assert_eq!(state.failing, [] as [std::string::String; 0]);
    }
}
