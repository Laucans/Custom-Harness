//! The board, read through core's `GitHub` port.
//!
//! One read is the roadmap, every milestone, and the tasks of the open ones
//! with their blockers — the same reads the router makes, so the sign shows
//! what the plant will actually pick next. Closed milestones keep their tasks
//! but not their blockers: a delivered milestone's dependencies are history.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Issue, Outcome};
use harness_core::ports::shell::github::GitHub;
use harness_workflows::common::labels;

use crate::ports::{Board, BoardReading, Milestone};

/// How many closed milestones the store keeps on its shelf.
const CLOSED_KEPT: usize = 3;

/// The board on GitHub.
pub struct GhBoard {
    gh: Rc<dyn GitHub>,
}

impl GhBoard {
    /// Reads through this `GitHub`.
    #[must_use]
    pub fn new(gh: Rc<dyn GitHub>) -> Self {
        Self { gh }
    }
}

#[async_trait(?Send)]
impl Board for GhBoard {
    async fn read(&self) -> Outcome<BoardReading> {
        let slug = self.gh.repo().await?;
        let roadmap = self.gh.issues_labelled(labels::ROADMAP, "open").await?;
        let mut all = self.gh.issues_labelled(labels::MILESTONE, "all").await?;
        all.sort_by_key(|issue| issue.number);
        let (open, closed): (Vec<Issue>, Vec<Issue>) = all.into_iter().partition(Issue::is_open);
        let mut milestones = Vec::with_capacity(open.len() + CLOSED_KEPT);
        for issue in open {
            let tasks = self.gh.sub_issues(issue.number).await?;
            let tasks = self.gh.with_blockers(tasks).await?;
            milestones.push(Milestone { issue, tasks });
        }
        for issue in closed.into_iter().rev().take(CLOSED_KEPT) {
            let tasks = self.gh.sub_issues(issue.number).await?;
            milestones.push(Milestone { issue, tasks });
        }
        Ok(BoardReading {
            slug,
            roadmap,
            milestones,
        })
    }
}
