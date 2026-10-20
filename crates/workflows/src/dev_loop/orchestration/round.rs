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
//!         └── a stage halts
//!               ├── the session was killed at its open PR ... delivered to the PR's cycle
//!               ├── a stop, a refusal ........................ a decision asked
//!               └── anything else ............................ the halt, as it was
//! ```

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context, Executable, Gate, Guarded, Stage};

use crate::dev_loop::action::actions::{MarkWaitingMerge, PickTask};
use crate::dev_loop::action::stop::RaiseDecision;
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
    /// What hands the task over when its session stopped or the breaker
    /// refused: a decision asked rather than a human waited on.
    pub raise: RaiseDecision,
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
            if let Err(halt) = stage.execute(ctx).await {
                // A session killed waiting on a CI the harness watches, its
                // PR open: the PR is the delivery, in the harness's own
                // cycle from here — not a failed round, and not a task the
                // scheduler may send a second session onto.
                if self.delivered.pr_stands(ctx, &halt).await? {
                    return Ok(Verdict::Continue);
                }
                return self.raise.on_halt(ctx, halt).await;
            }
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
        round_on(&ports_fake::with(Rc::clone(gh)))
    }

    /// A round built on these ports.
    fn round_on(ports: &crate::dev_loop::ports::Ports) -> TaskRound {
        let config = config_fake::config();
        let port = Rc::clone(&ports.gh);
        TaskRound {
            turn: 1,
            pick: PickTask {
                gh: Rc::clone(&port),
                branch: String::new(),
                spending: Rc::clone(&ports.spending),
                repo: Rc::clone(&ports.repo),
                resuming: None,
                wanted: None,
                data_layer: config.data_layer_spec(),
                chain: None,
            },
            stages: stages::table(ports, &config, 1),
            delivered: MarkWaitingMerge {
                gh: Rc::clone(&port),
                integration_branch: config.integration_branch.clone(),
            },
            raise: RaiseDecision {
                gh: Rc::clone(&port),
                autonomous: false,
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
        // Real case: `--stages pick` delivers nothing, so nothing
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

    // --- a session killed at its open PR ----------------------------------

    /// A session the idle kill takes: what the adapter says of it.
    struct Hung;

    #[async_trait(?Send)]
    impl harness_core::ports::agent::Session for Hung {
        async fn ask(
            &mut self,
            _prompt: &str,
        ) -> harness_core::domain::Outcome<harness_core::ports::agent::Reply> {
            Err(Halt::Failed(format!(
                "claude printed nothing for 1200s and {}",
                harness_core::domain::KILLED_AS_HUNG
            )))
        }
    }

    struct Hangs;

    #[async_trait(?Send)]
    impl harness_core::ports::agent::SessionFactory for Hangs {
        async fn open(
            &self,
            _spec: &harness_core::ports::agent::SessionSpec,
        ) -> harness_core::domain::Outcome<Box<dyn harness_core::ports::agent::Session>> {
            Ok(Box::new(Hung))
        }

        async fn fork(
            &self,
            spec: &harness_core::ports::agent::SessionSpec,
            _base: &harness_core::ports::agent::Base,
        ) -> harness_core::domain::Outcome<Box<dyn harness_core::ports::agent::Session>> {
            self.open(spec).await
        }
    }

    /// A ledger that takes every row and keeps none: the kill is what the
    /// round is judged on, not the row it leaves.
    struct Quiet;

    impl harness_core::ports::store::spending::Spending for Quiet {
        fn record(
            &self,
            _entry: &harness_core::ports::store::spending::Entry<'_>,
        ) -> harness_core::domain::Outcome<()> {
            Ok(())
        }
    }

    /// The board of a task whose session hangs, with its SPEC and its
    /// branch; `prs` is what `pr(<branch>)` answers.
    fn hanging(prs: Vec<(String, harness_core::domain::Pr)>) -> (Rc<FakeGitHub>, TaskRound) {
        let mut task = issue(11, &[labels::AGENT, labels::READY]);
        task.body = "## Business Goal\n\nthe DM pays\n\n## Acceptance Criteria\n\n- given, when, then\n\n## Scope\n\nbranch: feat/pay\n".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE]), task.clone()],
            subs: vec![(4, vec![task])],
            prs,
            ..FakeGitHub::default()
        });
        let mut ports = ports_fake::with(Rc::clone(&gh));
        ports.sessions = Rc::new(Hangs);
        ports.spending = Rc::new(Quiet);
        (gh, round_on(&ports))
    }

    fn open_pr(num: &str, head: &str, labels: &[&str]) -> harness_core::domain::Pr {
        harness_core::domain::Pr {
            num: num.to_string(),
            base: "milestone/4-pay".to_string(),
            head: head.to_string(),
            url: format!("https://github.com/o/r/pull/{num}"),
            state: "OPEN".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..harness_core::domain::Pr::default()
        }
    }

    #[tokio::test]
    async fn a_session_killed_at_its_open_pr_delivered_the_pr_to_the_harness_s_cycle() {
        // ForgeDesMondes' #248, #249, #250: pushed their PR, waited on its CI
        // through a background watcher, killed as hung, the round FAILED,
        // the task sent to a second session while the PR was under review.
        let (gh, round) = hanging(vec![(
            "feat/pay".to_string(),
            open_pr("77", "feat/pay", &[]),
        )]);
        let mut context = ctx("", false);
        let verdict = round
            .execute(&mut context)
            .await
            .expect("a delivered round, not a failed one");
        assert_eq!(verdict, Verdict::Continue);
        assert!(context.state.pr_stands);
        let writes = gh.writes();
        assert_eq!(
            writes[0],
            Wrote::Label(77, labels::TO_REVIEW.to_string()),
            "a read-side PR enters the cycle through its tap: {writes:?}"
        );
        assert!(matches!(&writes[1], Wrote::Comment(11, text) if text.contains("pull/77")));
        assert_eq!(
            writes[2],
            Wrote::Label(11, labels::REVIEW_PENDING.to_string())
        );
        assert_eq!(writes.len(), 3, "{writes:?}");
    }

    #[tokio::test]
    async fn a_write_side_pr_already_in_the_cycle_is_not_labelled_twice() {
        let (gh, round) = hanging(vec![(
            "feat/pay".to_string(),
            open_pr("77", "feat/pay", &[labels::TO_REVIEW]),
        )]);
        let mut context = ctx("", false);
        round
            .execute(&mut context)
            .await
            .expect("a delivered round");
        let writes = gh.writes();
        assert!(
            !writes.contains(&Wrote::Label(77, labels::TO_REVIEW.to_string())),
            "{writes:?}"
        );
        assert!(writes.contains(&Wrote::Label(11, labels::REVIEW_PENDING.to_string())));
    }

    #[tokio::test]
    async fn a_session_killed_before_its_pr_is_the_failure_it_was() {
        // No PR on the branch: nothing stands, and the round fails as today.
        let (gh, round) = hanging(Vec::new());
        let mut context = ctx("", false);
        let err = round
            .execute(&mut context)
            .await
            .expect_err("a failed round");
        assert!(err.killed_as_hung(), "{}", err.reason());
        assert!(!context.state.pr_stands);
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
        // A PR of the branch closed since is no delivery either.
        let closed = harness_core::domain::Pr {
            state: "CLOSED".to_string(),
            ..open_pr("77", "feat/pay", &[])
        };
        let (gh, round) = hanging(vec![("feat/pay".to_string(), closed)]);
        let err = round
            .execute(&mut ctx("", false))
            .await
            .expect_err("a failed round");
        assert!(err.killed_as_hung());
        assert!(gh.writes().is_empty(), "gh.writes() should be empty");
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
