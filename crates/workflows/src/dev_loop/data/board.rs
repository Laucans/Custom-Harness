//! The board: the current milestone, its tasks, and which one comes next.
//!
//! The **read** side of the issue model, in one place. The round needs it to
//! choose, preflight to verify the board is reachable, a status report to say
//! where we are. Many readers, one way to read.
//!
//! Here, not in `harness-core::adapters`: assembling "the lowest-numbered open
//! milestone, then its sub-issues, then their blockers" is a policy, and an
//! adapter decides nothing.
//!
//! **`Board` is pure data**, unlike Python, where it carried the `gh` client
//! to avoid reopening one. In Rust the client is passed, and a board without
//! client is tested by building it by hand.

use harness_core::adapters::shell::github::GitHub;
use harness_core::domain::{Halt, Issue, Outcome};

use crate::common::labels;
use crate::dev_loop::data::tasks;

/// What the loop sees of the board, at a given moment.
#[derive(Debug, Clone, Default)]
pub struct Board {
    /// The current milestone.
    pub milestone: Issue,
    /// Its sub-issues, blockers included.
    pub tasks: Vec<Issue>,
    /// The task the resumption point designates, if any.
    ///
    /// It **overrides** the board's choice, and this is what lets a round
    /// finish when `/code` already merged: the issue is closed, so the board
    /// wouldn't pick it, and `/create-test` would be lost.
    pub resuming: Option<Issue>,
}

impl Board {
    /// Open agent tasks — what decides rollover.
    #[must_use]
    pub fn open_agents(&self) -> Vec<&Issue> {
        tasks::open_agent_tasks(&self.tasks)
    }

    /// The next task by the four rules, if any.
    #[must_use]
    pub fn next(&self) -> Option<&Issue> {
        tasks::next_task(&self.tasks)
    }

    /// The sub-issue of this milestone with this number, **open or not**.
    ///
    /// Closed included, by design: a round interrupted after `/code` merged
    /// must be able to finish, and the issue it finishes is already closed.
    #[must_use]
    pub fn find(&self, key: &str) -> Option<&Issue> {
        let number: u64 = key.parse().ok()?;
        self.tasks.iter().find(|task| task.number == number)
    }

    /// The stop message when tasks remain but none can run.
    ///
    /// This case is **not** rollover, and confusing them costs an opus run:
    /// the milestone is not done, it awaits a human. It remains to say **which**
    /// gesture it awaits, because they're not the same — delivering everything
    /// and awaiting a merge is not the same answer as an unchecked `harness:ready` box.
    #[must_use]
    pub fn stuck(&self) -> String {
        let open = self.open_agents();
        let head = format!(
            "milestone {} has {} open task(s) but none can run:\n{}\n",
            self.milestone.reference(),
            open.len(),
            tasks::stuck_report(&self.tasks)
        );
        if !open.is_empty() && open.iter().all(|task| tasks::waiting_merge(task)) {
            return head
                + "Everything is delivered on the integration branch and \
                   waiting for you to merge it and close these issues. Nothing \
                   here is the harness's to do.";
        }
        head + &format!(
            "Add {} to the one to work on next, or close what blocks it, then \
             re-run.",
            labels::READY
        )
    }
}

/// The board, read now.
///
/// # Errors
///
/// - [`Halt::Halted`] if there is no open milestone: nothing to work on,
///   and saying so names the unblocking gesture;
/// - adapter failure, propagated as-is. A read that fails never becomes
///   an empty board, which would read "milestone done" and trigger `/planner`.
pub async fn read(gh: &dyn GitHub) -> Outcome<Board> {
    let found = gh.issues_labelled(labels::MILESTONE, "open").await?;
    let Some(milestone) = tasks::current_milestone(&found) else {
        return Err(Halt::Halted(format!(
            "no open {} issue — there is nothing to work from. Open one (or let \
             /planner open one from a {} issue) before running the harness.",
            labels::MILESTONE,
            labels::ROADMAP
        )));
    };
    let milestone = milestone.clone();
    let subs = gh.sub_issues(milestone.number).await?;
    let held = gh.with_blockers(subs).await?;
    Ok(Board {
        milestone,
        tasks: held,
        resuming: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("issue {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn task(number: u64) -> Issue {
        issue(number, &[labels::AGENT, labels::READY])
    }

    #[tokio::test]
    async fn the_board_reads_the_lowest_open_milestone_and_its_tasks() {
        let gh = FakeGitHub {
            issues: vec![
                issue(9, &[labels::MILESTONE]),
                issue(4, &[labels::MILESTONE]),
            ],
            subs: vec![(4, vec![task(11), task(12)])],
            ..FakeGitHub::default()
        };
        let board = read(&gh).await.expect("a board");
        assert_eq!(board.milestone.number, 4);
        assert_eq!(board.next().expect("a task").number, 11);
    }

    #[tokio::test]
    async fn no_open_milestone_halts_and_names_the_gesture() {
        let gh = FakeGitHub::default();
        let err = read(&gh).await.expect_err("must stop");
        let Halt::Halted(said) = err else {
            panic!("a voluntary stop, not a failure");
        };
        assert!(said.contains(labels::MILESTONE));
        assert!(said.contains("/planner"), "say what to do");
    }

    #[tokio::test]
    async fn an_unreadable_read_propagates_and_never_becomes_an_empty_board() {
        // Failure mode avoided: an empty board reads "milestone done" and
        // triggers a paid /planner.
        let gh = FakeGitHub {
            broken: Some(Halt::Unreadable("expired token".to_string())),
            ..FakeGitHub::default()
        };
        let err = read(&gh).await.expect_err("must fail");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[test]
    fn find_returns_a_closed_task_so_an_interrupted_round_can_finish() {
        let mut done = task(11);
        done.state = "closed".to_string();
        let board = Board {
            milestone: issue(4, &[labels::MILESTONE]),
            tasks: vec![done],
            resuming: None,
        };
        // The resumed round works on an issue already closed by /code's merge:
        // excluding it would lose /create-test.
        assert_eq!(board.find("11").expect("found").number, 11);
        assert!(board.find("99").is_none());
        assert!(board.find("not a number").is_none());
    }

    #[test]
    fn stuck_tells_you_to_merge_when_everything_is_delivered() {
        let board = Board {
            milestone: issue(4, &[labels::MILESTONE]),
            tasks: vec![issue(11, &[labels::AGENT, labels::WAITING_MERGE])],
            resuming: None,
        };
        let said = board.stuck();
        assert!(said.contains("waiting for you to merge"));
        assert!(!said.contains(labels::READY), "not the gesture here");
    }

    #[test]
    fn stuck_tells_you_what_to_tick_when_nothing_is_ready() {
        let board = Board {
            milestone: issue(4, &[labels::MILESTONE]),
            tasks: vec![issue(11, &[labels::AGENT])],
            resuming: None,
        };
        let said = board.stuck();
        assert!(said.contains(labels::READY));
        assert!(!said.contains("waiting for you to merge"));
    }
}
