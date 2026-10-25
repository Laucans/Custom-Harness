//! Assemble an entire re-study from ports and a config already built.

use std::cell::Cell;
use std::rc::Rc;

use harness_core::domain::{Halt, Outcome};
use harness_core::execution::Gate;
use harness_core::ports::shell::github::GitHub;

use crate::common::{branching, restudy};
use crate::restudy::config::Config;
use crate::restudy::data::state::StudyState;
use crate::restudy::orchestration::round;
use crate::restudy::orchestration::workflow::RestudyRun;
use crate::restudy::ports::Ports;

/// What an invocation requests.
pub struct Request {
    /// The task to re-study.
    pub task: u64,
}

/// Assemble an entire re-study from its wiring.
#[must_use]
pub fn build(
    ports: &Ports,
    config: &Config,
    request: &Request,
    pre: Gate<StudyState>,
) -> RestudyRun {
    RestudyRun {
        pre,
        // One re-study per run.
        remaining: Cell::new(1),
        gh: Rc::clone(&ports.gh),
        locks: Rc::clone(&ports.locks),
        restudy_dir: config.restudy_dir.clone(),
        task: request.task,
        lock_name: request.task.to_string(),
        round: round::build(ports, config),
    }
}

/// The branch a re-study of `task` checks out: its milestone's, as the
/// task's `## Decision` names it — where the slices being built are.
///
/// # Errors
/// The GitHub reads', and a task whose `## Decision` names no milestone.
pub async fn branch_of(gh: &dyn GitHub, task: u64) -> Outcome<String> {
    let issue = gh.issue(task).await?;
    let milestone = restudy::question_of(&issue)
        .and_then(|question| question.milestone)
        .ok_or_else(|| {
            Halt::Failed(format!(
                "{}'s ## Decision names no milestone to re-study it on",
                issue.reference()
            ))
        })?;
    let milestone = gh.issue(milestone).await?;
    Ok(branching::milestone_branch(
        milestone.number,
        &milestone.title,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::restudy::config::fake as config_fake;
    use crate::restudy::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_and_the_round_straight_through() {
        let run = build(
            &ports_fake::ports(),
            &config_fake::config(),
            &Request { task: 219 },
            Gate::empty("tooling"),
        );
        assert_eq!((run.task, run.lock_name.as_str()), (219, "219"));
        assert_eq!(run.round.stages.len(), 3);
        assert_eq!(run.remaining.get(), 1);
    }

    #[tokio::test]
    async fn a_re_study_checks_out_the_milestone_its_decision_names() {
        use crate::common::fake_github::FakeGitHub;
        use harness_core::domain::Issue;
        let gh = FakeGitHub {
            issues: vec![
                Issue {
                    number: 219,
                    body: "## Decision\n\nblocks: x\nmilestone: #28\ndiscovered: 2".to_string(),
                    ..Issue::default()
                },
                Issue {
                    number: 28,
                    title: "Session prep".to_string(),
                    ..Issue::default()
                },
                Issue {
                    number: 220,
                    body: "## Decision\n\nblocks: x".to_string(),
                    ..Issue::default()
                },
            ],
            ..FakeGitHub::default()
        };
        assert_eq!(
            branch_of(&gh, 219).await.expect("a branch"),
            "milestone/28-session-prep"
        );
        assert!(
            branch_of(&gh, 220)
                .await
                .expect_err("none")
                .reason()
                .contains("names no milestone")
        );
        assert!(branch_of(&gh, 9).await.is_err());
    }
}
