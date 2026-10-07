//! What the planner's stages pass to each other.

use harness_core::domain::{Issue, prompts};

use crate::common::explore::Explored;

/// The state of a planner run.
#[derive(Debug, Clone, Default)]
pub struct PlannerState {
    /// The roadmap issue being planned, set by precheck.
    pub roadmap: Option<Issue>,
    /// Milestones already open under this roadmap item, oldest first — the
    /// context that stops the plan from reopening what already exists.
    pub existing: Vec<Issue>,
    /// The combined business/technical grilling digests, if either exists.
    pub grounding: String,
    /// What the free `explore` entry read from the repo. Only the explorer
    /// reads it.
    pub brief: String,
}

impl PlannerState {
    /// The roadmap issue, or a programming failure if reached before
    /// `precheck`.
    ///
    /// # Panics
    /// If called before `precheck` sets it — something no path in
    /// [`Workflow::execute`](harness_core::execution::Workflow::execute)
    /// allows.
    #[must_use]
    pub const fn roadmap(&self) -> &Issue {
        self.roadmap
            .as_ref()
            .expect("precheck must set the roadmap issue before the stages run")
    }
}

impl Explored for PlannerState {
    fn brief_mut(&mut self) -> &mut String {
        &mut self.brief
    }

    fn subject(&self) -> String {
        let Some(issue) = &self.roadmap else {
            return String::new();
        };
        let body = issue.body.trim();
        format!(
            "GitHub roadmap issue #{} — {}\n\n{}",
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
        1
    }

    fn task(&self) -> String {
        format!("#{}", self.roadmap.as_ref().map_or(0, |i| i.number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_has_no_roadmap_issue_yet() {
        assert!(PlannerState::default().roadmap.is_none());
    }

    #[test]
    fn the_subject_names_the_issue_and_carries_its_body() {
        let state = PlannerState {
            roadmap: Some(Issue {
                number: 7,
                title: "Territory tooling".to_string(),
                body: "le corps".to_string(),
                ..Issue::default()
            }),
            ..PlannerState::default()
        };
        let said = state.subject();
        assert!(said.contains("#7"));
        assert!(said.contains("le corps"));
    }

    #[test]
    fn an_empty_body_is_said_in_words_in_the_subject() {
        let state = PlannerState {
            roadmap: Some(Issue::default()),
            ..PlannerState::default()
        };
        assert!(state.subject().contains(prompts::EMPTY_BODY));
    }

    #[test]
    fn with_no_roadmap_the_subject_is_empty_rather_than_panicking() {
        assert_eq!(PlannerState::default().subject(), "");
    }
}
