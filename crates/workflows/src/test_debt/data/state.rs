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
    /// The branch this attempt pushes (`landing::branch`): one per attempt,
    /// never a name a dropped PR's branch still holds on the remote.
    pub head: String,
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
