//! What split's stages pass to each other.

use harness_core::domain::Issue;

/// The state of a split run.
#[derive(Debug, Clone, Default)]
pub struct SplitState {
    /// The milestone issue being split, set by precheck.
    pub milestone: Option<Issue>,
    /// Task slices already open under this milestone, oldest first — the
    /// context that stops the plan from reopening what already exists.
    pub existing: Vec<Issue>,
    /// What the checkout already holds of the architecture — systems,
    /// Concepts, Capabilities, aggregates, `DataCapabilities`, Micro-UIs —
    /// rendered for the prompt, so the plan names what exists.
    pub inventory: String,
}

impl SplitState {
    /// The milestone issue, or a programming failure if reached before
    /// `precheck`.
    ///
    /// # Panics
    /// If called before `precheck` sets it — something no path in
    /// [`Workflow::execute`](harness_core::execution::Workflow::execute)
    /// allows.
    #[must_use]
    pub const fn milestone(&self) -> &Issue {
        self.milestone
            .as_ref()
            .expect("precheck must set the milestone issue before the stages run")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_has_no_milestone_issue_yet() {
        assert!(SplitState::default().milestone.is_none());
    }
}
