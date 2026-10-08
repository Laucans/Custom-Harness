//! What the stages of a review hand each other.

use harness_core::domain::Pr;

/// The state of a review: the PR, set by the precheck.
#[derive(Debug, Clone, Default)]
pub struct ReviewState {
    /// The PR under review. `None` until `precheck` has read it.
    pub pr: Option<Pr>,
    /// The PR delivers a milestone's data layer (`harness:data-layer` on
    /// the task it closes): the review holds it to the write side's bar.
    pub data_layer: bool,
}

impl ReviewState {
    /// The PR, or the programming error of reaching it before `precheck`.
    ///
    /// # Panics
    /// If called before `precheck` ran — which no path through
    /// [`Workflow::execute`](harness_core::execution::Workflow::execute)
    /// allows: the stages only run after it.
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
    fn a_fresh_state_has_no_pr_yet() {
        assert!(ReviewState::default().pr.is_none());
    }
}
