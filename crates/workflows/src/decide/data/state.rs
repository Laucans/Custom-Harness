//! What a decision's stages pass to each other.

use harness_core::domain::{Issue, Pr};

use crate::common::decision::Question;

/// The state of one decision.
#[derive(Debug, Clone, Default)]
pub struct DecideState {
    /// The issue to decide on, set by precheck.
    pub issue: Option<Issue>,
    /// Its `## Decision`, when it has one.
    pub question: Option<Question>,
    /// What kind of decision: `stop`, `breaker`, `review`, `graph`, `spec`…
    pub kind: String,
    /// The issue's comments, the latest last, cut to what a prompt holds.
    pub comments: String,
    /// The task's open pull request, for a task waiting on its review.
    pub pr: Option<Pr>,
    /// That pull request's comments — its review among them.
    pub pr_comments: String,
    /// A human answered under `## Decision` already: nothing to pay for.
    pub owner_decided: bool,
    /// The integration branch, set by the context stage: a PR landing on
    /// it is never merged nor cut by a decision, and a decision never tells
    /// a session to sync a branch with it.
    pub integration: String,
}

impl DecideState {
    /// What a decision may do here: the issue, its PR, the integration
    /// branch.
    ///
    /// # Panics
    /// If called before `precheck` sets the issue — as [`Self::issue`].
    #[must_use]
    pub fn scope(&self) -> crate::decide::data::decision::Scope<'_> {
        crate::decide::data::decision::Scope {
            number: self.issue().number,
            pr: self.pr.as_ref(),
            integration: &self.integration,
            rules: self.issue().has(crate::common::labels::RULES),
        }
    }

    /// The issue, or a programming failure if reached before `precheck`.
    ///
    /// # Panics
    /// If called before `precheck` sets it — something no path in
    /// [`Workflow::execute`](harness_core::execution::Workflow::execute)
    /// allows.
    #[must_use]
    pub const fn issue(&self) -> &Issue {
        self.issue
            .as_ref()
            .expect("precheck must set the issue before the stages run")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_has_nothing_to_decide_yet() {
        let state = DecideState::default();
        assert!(state.issue.is_none() && state.question.is_none() && state.pr.is_none());
        assert!(!state.owner_decided);
    }
}
