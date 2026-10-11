//! What one arbitration carries from the precheck to its stages.

use harness_core::domain::Issue;

use crate::arbiter::data::board::Resolution;
use crate::common::decision::Question;

/// The state of one arbitration.
#[derive(Debug, Clone, Default)]
pub struct ArbiterState {
    /// The task, as read by the precheck.
    pub issue: Option<Issue>,
    /// The question under its `## Decision`.
    pub question: Option<Question>,
    /// The milestone it sits under.
    pub milestone: Option<u64>,
    /// The milestone's other tasks, bodies included.
    pub siblings: Vec<Issue>,
    /// The ids the repository already holds, so nobody has to produce them.
    pub existing: Vec<String>,
    /// What the harness settled on its own, before any session.
    pub resolution: Resolution,
    /// What the run concluded, for the summary.
    pub outcome: Option<String>,
}

impl ArbiterState {
    /// The task.
    ///
    /// # Panics
    /// Before the precheck set it: a stage that runs without one is a
    /// sequencing bug, not a case to handle.
    #[must_use]
    pub const fn issue(&self) -> &Issue {
        self.issue
            .as_ref()
            .expect("precheck must set the issue before the stages run")
    }

    /// The question.
    ///
    /// # Panics
    /// Before the precheck set it, for the same reason as [`Self::issue`].
    #[must_use]
    pub const fn question(&self) -> &Question {
        self.question
            .as_ref()
            .expect("precheck must set the question before the stages run")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_has_nothing_yet() {
        let state = ArbiterState::default();
        assert!(state.issue.is_none());
        assert!(state.question.is_none());
        assert!(state.siblings.is_empty());
        assert!(state.resolution.unresolved.is_empty());
    }
}
