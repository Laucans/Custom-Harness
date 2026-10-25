//! The session facts: what a session's tools need of its task.
//!
//! What nothing in its checkout says — the base its pull request targets,
//! the branch it will open, the task's `## Architecture` declaration.
//!
//! Written by [`WriteFacts`](crate::dev_loop::action::facts::WriteFacts)
//! before the session opens, at the path the session's environment names
//! (`HARNESS_FACTS`). Read by the product's `scripts/gates-local.mjs` (the
//! base, the branch, the labels) and by `harness architecture-check` (the
//! declaration, the skeleton), so both judge the PR the session is about to
//! open, before it is open.

use crate::common::sections;
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::data::tasks;

/// What a session's tools know of its task: see the module.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Facts {
    /// The task's number.
    pub task: String,
    /// Its title.
    pub title: String,
    /// The branch its pull request targets, as `origin/<branch>`.
    pub base: String,
    /// The branch its body declares, the one the session opens.
    pub branch: String,
    /// The labels its pull request carries that a gate reads. Empty for a
    /// dev-loop task: the rules' task (`harness:rules`) is its own route.
    pub labels: Vec<String>,
    /// The checkout's head when the task was picked.
    pub round_base: String,
    /// The task's `## Architecture` section, as its body writes it.
    pub architecture: String,
    /// The files of the unit's skeleton the harness wrote.
    pub skeleton: Vec<String>,
}

impl Facts {
    /// The facts of the round's task, its pull request into `integration`.
    #[must_use]
    pub fn of(state: &Loop, integration: &str) -> Self {
        Self {
            task: state.task.number.clone(),
            title: state.task.title.clone(),
            base: format!("origin/{integration}"),
            branch: tasks::declared_branch(&state.task.body).unwrap_or_default(),
            labels: Vec::new(),
            round_base: state.round_base.clone(),
            architecture: architecture_of(&state.task.body),
            skeleton: state.skeleton.clone(),
        }
    }

    /// The facts a file holds, when it holds them.
    #[must_use]
    pub fn read(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }
}

/// The `## Architecture` section of a task's body, empty when it has none.
#[must_use]
pub fn architecture_of(body: &str) -> String {
    sections::parse(body)
        .get(sections::ARCHITECTURE)
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::prompts::Named;

    #[test]
    fn the_facts_carry_the_base_the_branch_and_the_declaration() {
        let state = Loop {
            task: Named {
                number: "42".to_string(),
                title: "ListScenes".to_string(),
                body: "## Scope\n\nbranch: `feat/list-scenes`\n\n## Architecture\n\nunit: capability\nsystem: session\n".to_string(),
            },
            round_base: "abc123".to_string(),
            skeleton: vec!["crates/session/capabilities/list-scenes/src/lib.rs".to_string()],
            ..Loop::default()
        };
        let facts = Facts::of(&state, "milestone/3-scenes");
        assert_eq!(facts.task, "42");
        assert_eq!(facts.base, "origin/milestone/3-scenes");
        assert_eq!(facts.branch, "feat/list-scenes");
        assert!(facts.architecture.contains("unit: capability"), "{facts:?}");
        assert_eq!(facts.labels, Vec::<String>::new());
        let text = serde_json::to_string(&facts).expect("json");
        assert_eq!(Facts::read(&text), Some(facts));
        assert_eq!(Facts::read("not json"), None);
    }
}
