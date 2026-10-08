//! A round: pick the task, run the sequence, confirm delivery.
//!
//! **The sequence is not here.** It's in `stages.rs`, one entry per stage.
//! This module carries what surrounds it: picking the task and the
//! postcondition that follows.
//!
//! A hand-written type, not a generic [`Round`](harness_core::execution::Round)
//! — not because this round branches (it no longer does; the roadmap/planner
//! branch is a separate, independently-triggered workflow now), but because
//! `pick` and `delivered` are `Action`s that must run unconditionally, outside
//! any `--stages`/resume guard a `Stage` would carry. Wrapping them as stages
//! in `Round<S>`'s table would subject picking the task and marking it
//! delivered to filters meant for the paid work in between.
//!
//! ```text
//! PickTask
//!   ├── no task, none open .......... nothing left: NothingLeft
//!   ├── no task, some open .......... halt: say what gesture unblocks
//!   └── a task ....................... the sequence, then delivery
//! ```

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context, Executable, Gate, Guarded, Stage};

use crate::dev_loop::action::actions::{MarkWaitingMerge, PickTask};
use crate::dev_loop::data::state::Loop;

/// A round of the development loop.
pub struct TaskRound {
    /// The turn number. Received, never counted here: the workflow counts
    /// turns (decision #13), and a round need not know it's the 3rd of 5.
    pub turn: u32,
    /// What picks the task.
    pub pick: PickTask,
    /// The sequence, in order. Comes from `stages::table`.
    pub stages: Vec<Stage<Loop>>,
    /// What marks the task delivered when a merged PR proves it.
    pub delivered: MarkWaitingMerge,
    /// What the round must achieve.
    pub post: Gate<Loop>,
}

#[async_trait(?Send)]
impl Executable<Loop> for TaskRound {
    fn post(&self) -> Option<&Gate<Loop>> {
        Some(&self.post)
    }

    async fn perform(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        self.pick.run(ctx).await?;
        if !ctx.state.has_task() {
            // `NothingLeft`, not success: the workflow must stop launching
            // rounds, not pay for another to relearn the milestone is done.
            // Opening the next roadmap item is a separate workflow's job now,
            // triggered on its own rather than chained from here.
            return Ok(Verdict::NothingLeft(
                "milestone has no runnable task left".to_string(),
            ));
        }
        for stage in &self.stages {
            stage.execute(ctx).await?;
        }
        // Last thing a round does: mark. The following gate judges.
        self.delivered.run(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::common::labels;
    use crate::dev_loop::checks::gates;
    use crate::dev_loop::config::fake as config_fake;
    use crate::dev_loop::orchestration::stages;
    use crate::dev_loop::ports::fake as ports_fake;
    use harness_core::domain::{Halt, Issue, Resumable};
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;
    use std::rc::Rc;

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("issue {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn ctx(stages: &str, dry_run: bool) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run,
                stages: stages.to_string(),
            },
            Loop::default(),
            Logbook::null(),
        )
    }

    /// A round built against this GitHub.
    fn round(gh: &Rc<FakeGitHub>) -> TaskRound {
        let ports = ports_fake::with(Rc::clone(gh));
        let config = config_fake::config();
        let port = Rc::clone(&ports.gh);
        TaskRound {
            turn: 1,
            pick: PickTask {
                gh: Rc::clone(&port),
                resuming: None,
                wanted: None,
            },
            stages: stages::table(&ports, &config, 1),
            delivered: MarkWaitingMerge {
                gh: Rc::clone(&port),
                integration_branch: config.integration_branch.clone(),
            },
            post: Gate {
                name: "the round must deliver",
                checks: vec![Box::new(gates::AMergedPrClosesTheTask {
                    gh: port,
                    integration_branch: config.integration_branch.clone(),
                    code_runs: true,
                    stages: String::new(),
                })],
            },
        }
    }

    #[tokio::test]
    async fn a_dry_run_names_the_task_it_would_pick_and_opens_nothing() {
        // Test wiring refuses to open a session: if one opened, this test
        // would fail on `Halt::Failed`.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![issue(11, &[labels::AGENT, labels::READY])])],
            ..FakeGitHub::default()
        });
        // Empty `--stages` would run all three; a real dry-run wires a repeat
        // factory. Here we verify the pick alone.
        let mut context = ctx("unknown-stage", true);
        let verdict = round(&gh)
            .execute(&mut context)
            .await
            .expect("a dry-run round");
        assert_eq!(verdict, Verdict::Continue);
        assert_eq!(context.state.task_key, "11");
        assert!(gh.writes().is_empty(), "a dry-run writes nothing");
        assert!(context.state.done().is_empty(), "and marks nothing");
    }

    #[tokio::test]
    async fn a_finished_milestone_says_there_is_nothing_left() {
        let mut done = issue(11, &[labels::AGENT]);
        done.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![done])],
            ..FakeGitHub::default()
        });
        let mut context = ctx("", false);
        let verdict = round(&gh)
            .execute(&mut context)
            .await
            .expect("nothing left, not a failure");
        let Verdict::NothingLeft(why) = verdict else {
            panic!("the workflow must stop launching, not chain");
        };
        assert!(why.contains("no runnable task"));
        assert!(!context.state.has_task());
    }

    #[tokio::test]
    async fn open_but_unplayable_tasks_stop_the_round_before_any_stage() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![issue(11, &[labels::AGENT])])],
            ..FakeGitHub::default()
        });
        let mut context = ctx("", false);
        let err = round(&gh)
            .execute(&mut context)
            .await
            .expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(!context.state.has_task(), "nothing picked");
    }

    #[tokio::test]
    async fn a_round_whose_every_stage_is_filtered_out_still_demands_the_proof() {
        // Real case: `--stages technical-refinement` delivers nothing, so nothing
        // marks the task. Without the postcondition, the next round would
        // repick it and repay the same SPEC write.
        let gh = Rc::new(FakeGitHub {
            issues: vec![
                issue(4, &[labels::MILESTONE]),
                issue(11, &[labels::AGENT, labels::READY]),
            ],
            subs: vec![(4, vec![issue(11, &[labels::AGENT, labels::READY])])],
            ..FakeGitHub::default()
        });
        let mut context = ctx("unknown-stage", false);
        let err = round(&gh)
            .execute(&mut context)
            .await
            .expect_err("must stop");
        assert!(err.reason().contains("Closes #11"));
        assert!(err.reason().contains("Stopping rather than looping"));
    }

    #[tokio::test]
    async fn a_task_a_merged_pr_closes_is_closed() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![
                issue(4, &[labels::MILESTONE]),
                issue(11, &[labels::AGENT, labels::READY]),
            ],
            subs: vec![(4, vec![issue(11, &[labels::AGENT, labels::READY])])],
            merged: vec![Issue {
                number: 99,
                body: "Closes #11".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        let mut context = ctx("unknown-stage", false);
        round(&gh)
            .execute(&mut context)
            .await
            .expect("a delivered round");
        assert_eq!(
            gh.writes(),
            vec![
                Wrote::Comment(
                    11,
                    crate::common::delivery::merged_note("main_agent", "#99")
                ),
                Wrote::Closed(11),
            ]
        );
    }
}
