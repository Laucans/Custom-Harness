//! What the refinement stages pass to each other.

use std::collections::HashMap;

use harness_core::domain::{Issue, prompts};

use crate::common::explore::Explored;

/// The state of a refinement round.
#[derive(Debug, Clone, Default)]
pub struct RefinementState {
    /// The refined issue, set by precheck.
    pub issue: Option<Issue>,
    /// The current round, counted from the comments.
    pub round_no: u32,
    /// The sections already in the body, by key — re-read at `precheck`.
    pub found: HashMap<String, String>,
    /// The keys this round writes.
    pub wanted: Vec<String>,
    /// What the free stage read from the repo. Only the explorer reads it.
    pub brief: String,
}

impl RefinementState {
    /// The issue, or a programming failure if reached before `precheck`.
    ///
    /// # Panics
    /// If called before `precheck` sets the issue — something no path in
    /// [`Workflow::execute`](harness_core::execution::Workflow::execute)
    /// allows.
    #[must_use]
    pub const fn issue(&self) -> &Issue {
        self.issue
            .as_ref()
            .expect("precheck must set the issue before the stages run")
    }
}

impl Explored for RefinementState {
    fn brief_mut(&mut self) -> &mut String {
        &mut self.brief
    }

    fn subject(&self) -> String {
        let Some(issue) = &self.issue else {
            return String::new();
        };
        let body = issue.body.trim();
        format!(
            "GitHub issue #{} — {}\n\n{}",
            issue.number,
            issue.title,
            if body.is_empty() {
                prompts::EMPTY_BODY
            } else {
                body
            }
        )
    }

    fn round_no(&self) -> u32 {
        self.round_no
    }

    fn task(&self) -> String {
        format!("#{}", self.issue.as_ref().map_or(0, |i| i.number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_has_no_issue_yet() {
        assert!(RefinementState::default().issue.is_none());
    }

    #[test]
    fn the_subject_names_the_issue_and_carries_its_body() {
        let state = RefinementState {
            issue: Some(Issue {
                number: 25,
                title: "Schema, seed & first DB-backed page".to_string(),
                body: "le corps".to_string(),
                ..Issue::default()
            }),
            ..RefinementState::default()
        };
        let said = state.subject();
        assert!(said.contains("#25"));
        assert!(said.contains("le corps"));
    }

    #[test]
    fn an_empty_body_is_said_in_words_in_the_subject() {
        let state = RefinementState {
            issue: Some(Issue::default()),
            ..RefinementState::default()
        };
        assert!(state.subject().contains(prompts::EMPTY_BODY));
    }
}
