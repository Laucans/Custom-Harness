//! What a re-study's stages pass to each other.

use harness_core::domain::Issue;

use crate::common::decision::Question;
use crate::restudy::data::reply::Studied;

/// The state of one re-study.
#[derive(Debug, Clone, Default)]
pub struct StudyState {
    /// The task, set by precheck.
    pub issue: Option<Issue>,
    /// Its `## Decision`, the streak due a re-study.
    pub question: Option<Question>,
    /// The milestone the slices open under.
    pub milestone: Option<Issue>,
    /// The milestone's branch, the checkout's.
    pub branch: String,
    /// The milestone's other sub-issues, open and closed: what is built or
    /// being built already.
    pub siblings: Vec<Issue>,
    /// The checkout's commit before the session: what it must still be
    /// after it.
    pub head: Option<String>,
    /// What the session answered, read.
    pub studied: Option<Studied>,
    /// What the publish came to, for the summary.
    pub published: Option<String>,
}

impl StudyState {
    /// The task, or a programming failure if reached before `precheck`.
    ///
    /// # Panics
    /// If called before `precheck` sets it — something no path in
    /// [`Workflow::execute`](harness_core::execution::Workflow::execute)
    /// allows.
    #[must_use]
    pub const fn issue(&self) -> &Issue {
        self.issue
            .as_ref()
            .expect("precheck must set the task before the stages run")
    }
}
