//! Whether a milestone is ready to merge.
//!
//! Pure: takes issues already read, decides nothing about the filesystem
//! or the network. Re-read by [`crate::milestone_merge::action::apply::run`]
//! immediately before acting on it — the acknowledged price of "judging is
//! not doing", same split the rest of the harness already pays.

use harness_core::domain::Issue;

use crate::common::labels;

/// Every sub-issue delivered, and there is at least one.
///
/// **Delivered is not closed**, and conflating the two deadlocked this
/// workflow: a task's PR merges into the *milestone* branch, and GitHub's
/// `Closes #n` only closes an issue when the PR merges into the repository's
/// **default** branch. So a delivered task stays open carrying
/// `harness:waiting-merge` — which is precisely what that label is for — and
/// a rule reading `is_closed` alone would wait for a state the flow never
/// reaches: the milestone would never merge, however many tasks were done.
///
/// `dev_loop`'s own `tasks::blockers_pending` already treats a
/// `waiting-merge` task as done for the same reason. This is that notion,
/// applied one level up.
///
/// A milestone with **zero** tasks is deliberately **not** ready: it never
/// had a branch pushed, so there is no PR a check could ever find — the next
/// step would simply fail trying to read one. Vacuous truth over an empty
/// list is the wrong answer here.
#[must_use]
pub fn all_tasks_delivered(tasks: &[Issue]) -> bool {
    !tasks.is_empty()
        && tasks
            .iter()
            .all(|task| task.is_closed() || task.has(labels::WAITING_MERGE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64, state: &str) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            ..Issue::default()
        }
    }

    fn delivered(number: u64) -> Issue {
        Issue {
            number,
            state: "open".to_string(),
            labels: vec![labels::WAITING_MERGE.to_string()],
            ..Issue::default()
        }
    }

    #[test]
    fn all_closed_is_ready() {
        assert!(all_tasks_delivered(&[
            issue(1, "closed"),
            issue(2, "closed")
        ]));
    }

    #[test]
    fn a_task_delivered_but_still_open_counts_as_done() {
        // The case the whole flow actually produces: a task PR merged into
        // the milestone branch leaves the issue open under
        // `harness:waiting-merge`, because closing keywords only fire on the
        // default branch. Requiring `closed` here deadlocked the milestone.
        assert!(all_tasks_delivered(&[delivered(1), issue(2, "closed")]));
    }

    #[test]
    fn one_open_task_nobody_delivered_is_not_ready() {
        assert!(!all_tasks_delivered(&[
            issue(1, "closed"),
            issue(2, "open")
        ]));
    }

    #[test]
    fn no_tasks_at_all_is_not_ready() {
        assert!(!all_tasks_delivered(&[]));
    }
}
