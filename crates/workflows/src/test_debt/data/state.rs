//! What the test-debt stages pass to each other.

use harness_core::domain::Issue;

/// The state of one attempt at a file's test debt.
#[derive(Debug, Clone, Default)]
pub struct DebtState {
    /// The test-debt issue, set by precheck.
    pub issue: Option<Issue>,
    /// The source file its title names — set by the free context stage, and
    /// the reason the paid stage runs at all.
    pub file: Option<String>,
    /// What the mutation runs found, oldest first: the body, then each later
    /// run's comment.
    pub findings: String,
}

impl DebtState {
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
    fn a_fresh_state_has_no_issue_and_no_file_yet() {
        let state = DebtState::default();
        assert!(state.issue.is_none());
        assert!(state.file.is_none());
        assert!(state.findings.is_empty());
    }
}
