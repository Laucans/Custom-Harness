//! A round: pick the task, run the sequence, confirm delivery.
//!
//! **The sequence is not here.** It's in `stages.rs`, one entry per stage.
//! This module carries what surrounds it: the rollover branch and the
//! postcondition that follows.
//!
//! A hand-written type, not a generic [`Round`](harness_core::execution::Round)
//! — variant B from `docs/ROUND-DRAFT.md`. The reason: this round **branches**:
//! a declarative table should carry a router to say so, and the repo already
//! ripped out a graph engine for exactly that. What remains of the graph is
//! an `if/else`:
//!
//! ```text
//! PickTask
//!   ├── no task, none open ........... rollover: /planner
//!   ├── no task, some open .......... halt: say what gesture unblocks
//!   └── a task ....................... the sequence, then delivery
//! ```
//!
//! The old router had three exits, one silent, because that was the shortest
//! way to say "this round goes nowhere" to a router that would otherwise chain.
//! An `Err(Halt)` and a `Verdict::NothingLeft` say it without a router.

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
    /// What picks the task, or flips to rollover.
    pub pick: PickTask,
    /// The sequence, in order. Comes from `stages::table`.
    pub stages: Vec<Stage<Loop>>,
    /// The rollover stage, if wired.
    ///
    /// `None` is the default, and it's a choice: chaining unsupervised spends
    /// an opus run and commits the project to a roadmap item nobody read.
    pub rollover: Option<Stage<Loop>>,
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
        if ctx.state.rollover {
            return self.roll(ctx).await;
        }
        for stage in &self.stages {
            stage.execute(ctx).await?;
        }
        // Last thing a round does: mark. The following gate judges.
        self.delivered.run(ctx).await
    }
}

impl TaskRound {
    /// The rollover branch: open the next roadmap item, or stop.
    async fn roll(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        let Some(planner) = &self.rollover else {
            // `NothingLeft`, not success: the workflow must stop launching
            // rounds, not pay for another to relearn there's nothing.
            return Ok(Verdict::NothingLeft(
                "no rollover stage wired — the milestone is finished and \
                 nothing is set to open the next roadmap item"
                    .to_string(),
            ));
        };
        planner.execute(ctx).await?;
        Ok(Verdict::Continue)
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

    /// A round built against this GitHub, with or without rollover wired.
    fn round(gh: &Rc<FakeGitHub>, with_rollover: bool) -> TaskRound {
        let ports = ports_fake::with(Rc::clone(gh));
        let config = config_fake::config();
        let port = Rc::clone(&ports.gh);
        TaskRound {
            turn: 1,
            pick: PickTask {
                gh: Rc::clone(&port),
                resuming: None,
            },
            stages: stages::table(&ports, &config, 1),
            rollover: with_rollover.then(|| stages::planner(&ports, &config, 1)),
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
        let verdict = round(&gh, false)
            .execute(&mut context)
            .await
            .expect("a dry-run round");
        assert_eq!(verdict, Verdict::Continue);
        assert_eq!(context.state.task_key, "11");
        assert!(gh.writes().is_empty(), "a dry-run writes nothing");
        assert!(context.state.done().is_empty(), "and marks nothing");
    }

    #[tokio::test]
    async fn a_finished_milestone_without_a_rollover_says_there_is_nothing_left() {
        let mut done = issue(11, &[labels::AGENT]);
        done.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![done])],
            ..FakeGitHub::default()
        });
        let mut context = ctx("", false);
        let verdict = round(&gh, false)
            .execute(&mut context)
            .await
            .expect("a rollover");
        let Verdict::NothingLeft(why) = verdict else {
            panic!("the workflow must stop launching, not chain");
        };
        assert!(why.contains("no rollover stage wired"));
        assert!(context.state.rollover);
    }

    #[tokio::test]
    async fn a_rollover_round_passes_the_delivery_gate_having_no_task() {
        // Failure mode avoided: the round's postcondition demands a merged PR
        // for a task that doesn't exist, and rollover never succeeds.
        let mut done = issue(11, &[labels::AGENT]);
        done.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![done])],
            ..FakeGitHub::default()
        });
        let mut context = ctx("", false);
        assert!(round(&gh, false).execute(&mut context).await.is_ok());
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
        let err = round(&gh, false)
            .execute(&mut context)
            .await
            .expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(!context.state.rollover, "not a rollover");
    }

    #[tokio::test]
    async fn a_round_whose_every_stage_is_filtered_out_still_demands_the_proof() {
        // Real case: `--stages business-analyst` delivers nothing, so nothing
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
        let err = round(&gh, false)
            .execute(&mut context)
            .await
            .expect_err("must stop");
        assert!(err.reason().contains("Closes #11"));
        assert!(err.reason().contains("Stopping rather than looping"));
    }

    #[tokio::test]
    async fn a_task_a_merged_pr_closes_is_marked_waiting_merge_not_closed() {
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
        let mut context = ctx("aucun-stage-connu", false);
        round(&gh, false)
            .execute(&mut context)
            .await
            .expect("un round livré");
        assert_eq!(
            gh.writes(),
            vec![Wrote::Label(11, labels::WAITING_MERGE.to_string())]
        );
    }
}
