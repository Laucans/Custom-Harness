//! The whole loop: its gates, then N turns of the same round.
//!
//! **The core counts the turns, the round receives its number** (decision
//! #13). The budget is a setting of the run; a round has no business knowing
//! it is the 3rd of 5, and cannot know it — it receives a `u32` and nothing
//! else.
//!
//! # What is left here, and what moved
//!
//! The `for` loop over the turns, the resume point written before propagating,
//! the early stop on `NothingLeft` — all of it is now
//! [`Workflow`] in the core. This file
//! keeps only what is proper to *this* loop: a fresh round per turn, the
//! `stages_done` dropped between two tasks, and where the resume point is
//! written.
//!
//! What justified writing the loop by hand was the rule that a shape gets
//! factored out once there are two examples, not one. There are three now, and
//! the second and third (`pr_review`, `refinement`) differ from this one by a
//! single number: how many rounds they run.
//!
//! # What the workflow does not verify afterwards
//!
//! Nothing. What the loop guarantees is guaranteed **per round** — a merged PR
//! carrying `Closes #N` — so at the moment the round ends, so before the next
//! one is paid. Re-verifying it here would say nothing more and would say it
//! too late.

use std::cell::Cell;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Resumable, Verdict};
use harness_core::execution::{Context, Executable, Gate, Workflow};
use harness_core::ports::store::checkpoint::Checkpoints;

use crate::dev_loop::data::state::Loop;
use crate::dev_loop::orchestration::round::TaskRound;

/// The development loop: a preflight, then turns.
pub struct DevLoop {
    /// What must hold before the first stage is paid.
    pub pre: Gate<Loop>,
    /// How many turns are left — the budget, at assembly time.
    pub remaining: Cell<u32>,
    /// What builds a turn's round. A factory and not a reused round: each turn
    /// has its number, which goes into the ledger's `round` column and into
    /// the journal's tag.
    pub rounds: Box<dyn Fn(u32) -> TaskRound>,
    /// Where the resume point is written, when there is one.
    pub store: Option<Rc<dyn Checkpoints>>,
    /// The flow's identifier, which names the state file.
    pub flow_id: String,
}

#[async_trait(?Send)]
impl Workflow<Loop> for DevLoop {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    fn round(&self, turn: u32) -> Box<dyn Executable<Loop> + '_> {
        Box::new((self.rounds)(turn))
    }

    fn between(&self, ctx: &mut Context<Loop>) {
        // The previous task is finished: its `stages_done` goes with it, or
        // the next one's three stages would be skipped.
        ctx.state = ctx.state.turned();
    }

    /// Writes the pointer and the state, if there is a store.
    fn remember(&self, ctx: &Context<Loop>) -> Outcome<()> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        if ctx.settings.dry_run || !ctx.state.has_task() {
            return Ok(());
        }
        store.set_pointer(&ctx.state.task_key, &self.flow_id)?;
        let state = serde_json::to_value(&ctx.state)
            .map_err(|e| Halt::Failed(format!("the round state does not serialize: {e}")))?;
        let step = ctx.state.done().last().cloned().unwrap_or_default();
        store.save(&self.flow_id, &step, &state)
    }

    /// Clears the resume point.
    fn forget(&self) -> Outcome<()> {
        self.store.as_ref().map_or(Ok(()), |store| store.clear())
    }
}

#[async_trait(?Send)]
impl Executable<Loop> for DevLoop {
    fn pre(&self) -> Option<&Gate<Loop>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use crate::common::labels;
    use crate::dev_loop::action::actions::{MarkWaitingMerge, PickTask};
    use crate::dev_loop::config::fake as config_fake;
    use crate::dev_loop::orchestration::stages;
    use crate::dev_loop::ports::fake as ports_fake;
    use harness_core::domain::Issue;
    use harness_core::execution::{Guarded, Settings};
    use harness_core::traces::{Logbook, Sink, Verbosity};
    use std::cell::RefCell;

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("issue {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    /// A board whose every agent task is closed: every round therefore finds
    /// nothing to pick, and none of them pays.
    fn finished_milestone() -> Rc<FakeGitHub> {
        let mut done = issue(11, &[labels::AGENT]);
        done.state = "closed".to_string();
        Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![done])],
            ..FakeGitHub::default()
        })
    }

    fn loop_over(gh: &Rc<FakeGitHub>, budget: u32) -> DevLoop {
        let port = Rc::clone(gh);
        DevLoop {
            pre: Gate::empty("preflight"),
            remaining: Cell::new(budget),
            rounds: Box::new(move |turn| {
                let ports = ports_fake::with(Rc::clone(&port));
                let config = config_fake::config();
                let gh = Rc::clone(&ports.gh);
                TaskRound {
                    turn,
                    pick: PickTask {
                        gh: Rc::clone(&gh),
                        resuming: None,
                        wanted: None,
                    },
                    stages: stages::table(&ports, &config, turn),
                    delivered: MarkWaitingMerge {
                        gh,
                        integration_branch: config.integration_branch.clone(),
                    },
                    post: Gate::empty("delivered"),
                }
            }),
            store: None,
            flow_id: "test".to_string(),
        }
    }

    fn ctx(log: Logbook) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            Loop::default(),
            log,
        )
    }

    #[tokio::test]
    async fn nothing_left_stops_the_loop_instead_of_replaying_the_same_round() {
        // Without this, a finished milestone would run all three rounds of
        // the budget to learn three times over that there is nothing there.
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        let mut context = ctx(log);
        let built = loop_over(&finished_milestone(), 3);
        let verdict = Guarded::execute(&built, &mut context)
            .await
            .expect("a clean stop");
        assert!(matches!(verdict, Verdict::NothingLeft(_)));
        assert_eq!(built.remaining.get(), 2, "two turns went unpaid");
        let said = capture.0.borrow().join("\n");
        assert!(said.contains("round 1/3"));
        assert!(
            !said.contains("round 2/3"),
            "the second turn never happened"
        );
    }

    #[tokio::test]
    async fn a_round_that_halts_stops_the_loop_and_keeps_its_reason() {
        // An open but unplayable task: the round stops, and the remaining
        // budget is not spent asking for it again.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![issue(11, &[labels::AGENT])])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Logbook::null());
        let built = loop_over(&gh, 3);
        let err = Guarded::execute(&built, &mut context)
            .await
            .expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(err.reason().contains(labels::READY));
    }

    #[test]
    fn a_new_turn_forgets_the_stages_of_the_task_that_just_finished() {
        // The invariant: `stages_done` is "what ran for this task". Keeping it
        // from one turn to the next would skip the next three stages.
        let mut first = Loop::default();
        first.milestone.number = "4".to_string();
        first.task_key = "11".to_string();
        first.mark("code");
        let next = first.turned();
        assert_eq!(next.milestone.number, "4", "the milestone stays");
        assert!(next.done().is_empty());
        assert!(!next.has_task());
    }
}
