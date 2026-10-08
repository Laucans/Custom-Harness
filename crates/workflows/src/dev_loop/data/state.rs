//! The state of a loop round: the `S` of `Context<S>`.
//!
//! It replaces the three overlapping objects on the Python side — `Ctx`,
//! `RoundCtx(Ctx)` and `RoundState`. One type, and the two bounds that
//! core requires of it: [`Resumable`] for idempotence, [`Scoped`] so no
//! session starts without knowing what it's working on.

use harness_core::domain::{Named, Resumable, Scope, Scoped, Sibling};

/// What the round knows, and what belongs only to it.
///
/// Serializable because it's **this** that a run resumes: the two-line
/// pointer says which task, this state says where it was.
// Four facts of the round, each true or false on its own: resumed, the two
// sections written, the side. A state machine would multiply their product.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Loop {
    /// The roadmap item the milestone came from, when it could be found.
    pub roadmap: Option<Named>,
    /// The current milestone.
    pub milestone: Named,
    /// The milestone's tasks, titles and status only.
    pub siblings: Vec<Sibling>,
    /// The chosen task. Empty until `pick-task` runs.
    pub task: Named,
    /// The task resume key — its number.
    pub task_key: String,
    /// `auto` or `human`, for the journal.
    pub kind: String,
    /// The task is on the write side of the architecture: its PR is opened
    /// for a human to merge, never merged by the session.
    pub write_side: bool,
    /// True when the resume point designated the task, rather than the board.
    ///
    /// What guards read that only apply on a resumed round: on a fresh
    /// round, `/code` never ran, and looking for a merged PR would cost one
    /// call per round for a known answer.
    pub resumed: bool,
    /// The business SPEC is already in the issue body.
    pub spec_written: bool,
    /// The technical sections are already in the issue body.
    pub tech_written: bool,
    /// The stages already done for this task, across all runs.
    pub stages_done: Vec<String>,
}

impl Resumable for Loop {
    fn done(&self) -> &[String] {
        &self.stages_done
    }

    fn mark(&mut self, stage: &str) {
        if !self.is_done(stage) {
            self.stages_done.push(stage.to_string());
        }
    }
}

impl Scoped for Loop {
    fn scope(&self) -> Scope {
        Scope {
            roadmap: self.roadmap.clone(),
            siblings: self.siblings.clone(),
            milestone: self.milestone.clone(),
            task: self.task.clone(),
        }
    }
}

impl Loop {
    /// True if a task was chosen.
    #[must_use]
    pub const fn has_task(&self) -> bool {
        !self.task_key.is_empty()
    }

    /// The state for the next turn: the task forgotten, the milestone kept.
    ///
    /// `stages_done` goes with the task, and that's the invariant: this list
    /// is "what already ran **for this task**". Keeping it from one turn to
    /// the next would skip the three stages of the next task.
    ///
    /// The milestone stays because it will be re-read anyway; keeping it just
    /// leaves a readable log between turns.
    #[must_use]
    pub fn turned(&self) -> Self {
        Self {
            milestone: self.milestone.clone(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Loop {
        Loop {
            milestone: Named {
                number: "12".to_string(),
                title: "The chat".to_string(),
                body: "the milestone".to_string(),
            },
            task: Named {
                number: "34".to_string(),
                title: "The grid".to_string(),
                body: "the SPEC".to_string(),
            },
            task_key: "34".to_string(),
            kind: "auto".to_string(),
            ..Loop::default()
        }
    }

    #[test]
    fn a_fresh_state_has_no_task() {
        let fresh = Loop::default();
        assert!(!fresh.has_task());
        assert!(fresh.done().is_empty());
    }

    #[test]
    fn marking_a_stage_twice_does_not_duplicate_it() {
        // Idempotence is in core, via this bound: without it we'd have to
        // loop through the list on every call, like Python did.
        let mut round = state();
        round.mark("code");
        round.mark("code");
        assert_eq!(round.done(), &["code".to_string()]);
        assert!(round.is_done("code"));
        assert!(!round.is_done("create-test"));
    }

    #[test]
    fn the_scope_carries_both_issues_so_no_session_starts_blind() {
        let scope = state().scope();
        assert_eq!(scope.milestone.number, "12");
        assert_eq!(scope.task.title, "The grid");
        assert_eq!(scope.task.body, "the SPEC");
    }
}
