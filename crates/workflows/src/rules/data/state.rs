//! What the rules' stages pass to each other.

use harness_core::domain::Issue;

use crate::rules::data::request::Ask;

/// The state of one rules' task.
#[derive(Debug, Clone, Default)]
pub struct RulesState {
    /// The rules' issue, set by precheck.
    pub issue: Option<Issue>,
    /// What it asks — set by the free stage, and the reason the paid stages
    /// run at all.
    pub ask: Option<Ask>,
    /// Why what it asks could not be read, when it could not.
    pub unreadable: Option<String>,
    /// What the issue's comments say — a fidelity review a former attempt
    /// left, the human's answer — oldest first.
    pub comments: String,
}

impl RulesState {
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
