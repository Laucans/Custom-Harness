//! A workflow: N rounds of the same shape, and everything that surrounds
//! them — the precheck, the lock, the resume point, the closing line.
//!
//! **Every workflow has a round, and a single execution is a workflow of one
//! round.** That's the whole of it: [`Workflow::remaining`] starts at `1` for
//! a PR review or a refinement round, at the turn budget for the dev loop,
//! and nothing else about the shape changes. Before this trait there were
//! two — a hand-written loop for the dev loop, a `OneShot` for the other two
//! — which is to say the same five concerns written twice, with "how many
//! times" as the only real difference between them.
//!
//! # What the core decides, and what it doesn't
//!
//! It decides the order: precheck, lock, rounds, summary; and per turn:
//! separator, round, resume point, counter. It decides nothing about the round
//! — [`Workflow::round`] returns an [`Executable`], which is a
//! [`Round`](crate::execution::Round) when the sequence doesn't branch and a
//! hand-written type when it does.
//!
//! # Why the tooling gate is not a hook here
//!
//! It is [`Executable::pre`], where every other level already carries its
//! guards, and [`Guarded::execute`] runs it before `perform` — which is where
//! a workflow delegates to [`Workflow::execute`]. Declaring it twice would
//! mean two places to keep in sync for one gate.
//!
//! # Why `Executable` is implemented by hand
//!
//! A blanket `impl<S, W: Workflow<S>> Executable<S> for W` would conflict with
//! the impls that [`Round`](crate::execution::Round) and
//! [`Stage`](crate::execution::Stage) already carry, as far as the compiler is
//! concerned — it cannot prove a type will never carry both. So each workflow
//! writes two lines that delegate here.

use std::cell::Cell;
use std::path::Path;

use async_trait::async_trait;

use crate::domain::{Outcome, Verdict};
use crate::execution::data::context::Context;
use crate::execution::traits::{Executable, Guarded};
use crate::ports::store::lock::Locks;

/// Where a workflow's lock lives, and what holds it.
///
/// One descriptor rather than three accessors: a workflow either locks its
/// target or it doesn't, and "a directory without a name" is not a state the
/// caller should have to rule out.
pub struct Lock<'a> {
    /// What holds locks.
    pub locks: &'a dyn Locks,
    /// The directory the lock lives under.
    pub dir: &'a Path,
    /// The lock's name: at most one bearer of this target at a time.
    pub name: &'a str,
}

/// The policy of a workflow — what it establishes before paying, and how many
/// rounds it runs.
#[async_trait(?Send)]
pub trait Workflow<S> {
    /// How many rounds are still to run. `1` is a single execution.
    ///
    /// A `Cell` because the workflow genuinely carries the remaining count
    /// and the core decrements it: the number is readable afterwards — by a
    /// test, a trace, a future resume point — instead of living only inside a
    /// loop variable. The interior mutability is what lets it stay a `&self`
    /// API, like every other level.
    fn remaining(&self) -> &Cell<u32>;

    /// Is there anything to do? `Ok(Some(message))` stops the run cleanly,
    /// before even trying the lock — a draft PR, an already-reviewed one, a
    /// closed issue have nothing to obtain, and that is not a failure.
    ///
    /// # Errors
    /// A read that didn't come back.
    async fn precheck(&self, _ctx: &mut Context<S>) -> Outcome<Option<String>> {
        Ok(None)
    }

    /// The lock this workflow holds while it runs. `None`: nothing to hold.
    fn lock(&self) -> Option<Lock<'_>> {
        None
    }

    /// The message when the lock is already held by someone else.
    fn held(&self, _ctx: &Context<S>) -> String {
        "skip — another run already holds the lock".to_string()
    }

    /// The round of turn `turn`, counted from 1.
    ///
    /// A method and not a field: the dev loop builds a fresh round per turn
    /// because the turn number goes into the ledger and the journal, while a
    /// single-round workflow hands out the one it built at assembly time.
    fn round(&self, turn: u32) -> Box<dyn Executable<S> + '_>;

    /// Called between two rounds, never before the first.
    ///
    /// What the dev loop drops here: the finished task's `stages_done`, which
    /// would otherwise skip the next task's stages.
    fn between(&self, _ctx: &mut Context<S>) {}

    /// Writes the resume point, if this workflow has one.
    ///
    /// # Errors
    /// A store that didn't answer.
    fn remember(&self, _ctx: &Context<S>) -> Outcome<()> {
        Ok(())
    }

    /// Clears the resume point once a round delivered.
    ///
    /// # Errors
    /// A store that didn't answer.
    fn forget(&self) -> Outcome<()> {
        Ok(())
    }

    /// The line a successful run leaves behind. `None`: say nothing.
    fn summary(&self, _ctx: &Context<S>) -> Option<String> {
        None
    }

    /// Precheck, lock, the rounds, the summary — in that order, and that is
    /// all a workflow is.
    ///
    /// # Errors
    /// Whatever a round propagates, and a lock that couldn't be placed.
    async fn execute(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        if let Some(message) = self.precheck(ctx).await? {
            ctx.traces.say(&message);
            return Ok(Verdict::Continue);
        }

        if let Some(lock) = self.lock()
            && !lock.locks.acquire(lock.dir, lock.name)?
        {
            ctx.traces.say(&self.held(ctx));
            return Ok(Verdict::Continue);
        }

        let ran = self.rounds(ctx).await;
        if let Some(lock) = self.lock() {
            lock.locks.release(lock.dir, lock.name);
        }

        if let Verdict::NothingLeft(why) = ran? {
            ctx.traces.say(&why);
            return Ok(Verdict::NothingLeft(why));
        }
        if let Some(line) = self.summary(ctx) {
            ctx.traces.say(&line);
        }
        Ok(Verdict::Continue)
    }

    /// One round per remaining turn, until a round says there is nothing left.
    ///
    /// # Errors
    /// Whatever a round propagates.
    async fn rounds(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        let total = self.remaining().get();
        while self.remaining().get() > 0 {
            let left = self.remaining().get();
            let turn = total.saturating_sub(left).saturating_add(1);
            if turn > 1 {
                self.between(ctx);
            }
            // Said only when there are several: a single round numbering
            // itself 1/1 is noise.
            if total > 1 {
                ctx.traces.say(&format!("--- round {turn}/{total} ---"));
            }

            let verdict = self.round(turn).execute(ctx).await;
            // Written before propagating: a round that stopped halfway is
            // exactly the one a resume needs. Not doing so would re-pay the
            // stages that already passed.
            self.remember(ctx)?;
            self.remaining().set(left.saturating_sub(1));

            if let Verdict::NothingLeft(why) = verdict? {
                return Ok(Verdict::NothingLeft(why));
            }
            // The round delivered: the resume point has nothing left to
            // describe, and leaving it would re-pick a finished target.
            self.forget()?;
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::store::lock::DirLocks;
    use crate::domain::Halt;
    use crate::execution::action::kinds::Action;
    use crate::execution::checks::gate::Gate;
    use crate::execution::data::context::Settings;
    use crate::execution::orchestration::round::Round;
    use crate::execution::orchestration::stage::{Stage, StageBody};
    use crate::execution::traits::Verification;
    use crate::traces::Logbook;
    use std::cell::RefCell;
    use std::path::PathBuf;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("harness-workflow-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A `RefCell` state: the rounds are handed out behind `&self`, so a test
    /// that wants to watch what ran cannot take `&mut` on the state.
    type Trail = RefCell<Vec<String>>;

    struct Push(&'static str);

    #[async_trait(?Send)]
    impl Action<Trail> for Push {
        async fn run(&self, ctx: &mut Context<Trail>) -> Outcome<Verdict> {
            ctx.state.borrow_mut().push(self.0.to_string());
            Ok(Verdict::Continue)
        }
    }

    fn local_stage(tag: &'static str) -> Stage<Trail> {
        Stage {
            name: tag.to_string(),
            pre: None,
            post: None,
            body: StageBody::Local {
                actions: vec![Box::new(Push(tag))],
            },
        }
    }

    struct AlwaysFails;

    #[async_trait(?Send)]
    impl Verification<Trail> for AlwaysFails {
        async fn verify(&self, _ctx: &Context<Trail>) -> Outcome<Verdict> {
            Err(Halt::Failed("broken".to_string()))
        }
    }

    fn failing_stage() -> Stage<Trail> {
        Stage {
            name: "fails".to_string(),
            pre: Some(Gate {
                name: "always",
                checks: vec![Box::new(AlwaysFails)],
            }),
            post: None,
            body: StageBody::Local { actions: vec![] },
        }
    }

    /// A round that says there is nothing left, like the dev loop's when a
    /// milestone has no runnable task.
    struct Exhausted;

    #[async_trait(?Send)]
    impl Executable<Trail> for Exhausted {
        async fn perform(&self, _ctx: &mut Context<Trail>) -> Outcome<Verdict> {
            Ok(Verdict::NothingLeft("nothing left".to_string()))
        }
    }

    /// A test workflow: one round built once, an optional lock, and counters
    /// for the hooks the core is supposed to call.
    struct Fixture {
        remaining: Cell<u32>,
        round: Round<Trail>,
        lock: Option<(PathBuf, String)>,
        exhausted_at: Option<u32>,
        betweens: Cell<u32>,
        remembered: Cell<u32>,
        forgotten: Cell<u32>,
    }

    impl Fixture {
        fn new(remaining: u32, stages: Vec<Stage<Trail>>) -> Self {
            Self {
                remaining: Cell::new(remaining),
                round: Round::plain(stages),
                lock: None,
                exhausted_at: None,
                betweens: Cell::new(0),
                remembered: Cell::new(0),
                forgotten: Cell::new(0),
            }
        }

        fn locking(mut self, dir: PathBuf, name: &str) -> Self {
            self.lock = Some((dir, name.to_string()));
            self
        }
    }

    #[async_trait(?Send)]
    impl Workflow<Trail> for Fixture {
        fn remaining(&self) -> &Cell<u32> {
            &self.remaining
        }

        fn lock(&self) -> Option<Lock<'_>> {
            self.lock.as_ref().map(|(dir, name)| Lock {
                locks: &DirLocks,
                dir,
                name,
            })
        }

        fn round(&self, turn: u32) -> Box<dyn Executable<Trail> + '_> {
            if self.exhausted_at == Some(turn) {
                return Box::new(Exhausted);
            }
            Box::new(&self.round)
        }

        fn between(&self, _ctx: &mut Context<Trail>) {
            self.betweens.set(self.betweens.get() + 1);
        }

        fn remember(&self, _ctx: &Context<Trail>) -> Outcome<()> {
            self.remembered.set(self.remembered.get() + 1);
            Ok(())
        }

        fn forget(&self) -> Outcome<()> {
            self.forgotten.set(self.forgotten.get() + 1);
            Ok(())
        }

        fn summary(&self, _ctx: &Context<Trail>) -> Option<String> {
            Some("done".to_string())
        }
    }

    fn ctx() -> Context<Trail> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            RefCell::new(Vec::new()),
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn one_remaining_round_runs_the_sequence_once() {
        let flow = Fixture::new(1, vec![local_stage("a"), local_stage("b")]);
        let mut context = ctx();
        flow.execute(&mut context).await.expect("a success");
        assert_eq!(
            *context.state.borrow(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(flow.remaining.get(), 0, "the round was spent");
        assert_eq!(flow.betweens.get(), 0, "nothing comes between one round");
    }

    #[tokio::test]
    async fn three_remaining_rounds_replay_the_same_round_and_drain_the_counter() {
        let flow = Fixture::new(3, vec![local_stage("a")]);
        let mut context = ctx();
        flow.execute(&mut context).await.expect("a success");
        assert_eq!(context.state.borrow().len(), 3);
        assert_eq!(flow.remaining.get(), 0);
        // Between the three rounds, so twice — never before the first.
        assert_eq!(flow.betweens.get(), 2);
        assert_eq!(flow.forgotten.get(), 3);
    }

    #[tokio::test]
    async fn nothing_left_stops_the_remaining_rounds() {
        // Without this, a workflow with nothing to do would pay its whole
        // budget to learn the same thing three times.
        let mut flow = Fixture::new(3, vec![local_stage("a")]);
        flow.exhausted_at = Some(2);
        let mut context = ctx();
        let verdict = flow.execute(&mut context).await.expect("a clean stop");
        assert!(matches!(verdict, Verdict::NothingLeft(_)));
        assert_eq!(context.state.borrow().len(), 1, "only the first round ran");
        assert_eq!(flow.remaining.get(), 1, "the third was never started");
        assert_eq!(
            flow.forgotten.get(),
            1,
            "the exhausted round forgot nothing"
        );
    }

    #[tokio::test]
    async fn a_failed_round_is_remembered_before_it_propagates() {
        // The resume point exists for exactly this round: the one that stopped
        // halfway. Writing it after propagating would re-pay its stages.
        let flow = Fixture::new(3, vec![local_stage("a"), failing_stage()]);
        let mut context = ctx();
        let err = flow.execute(&mut context).await.expect_err("must stop");
        assert!(matches!(err, Halt::Failed(_)));
        assert_eq!(flow.remembered.get(), 1);
        assert_eq!(flow.forgotten.get(), 0, "nothing delivered");
        assert_eq!(flow.remaining.get(), 2, "the budget is not spent");
    }

    #[tokio::test]
    async fn a_precheck_that_has_nothing_to_do_never_touches_the_lock() {
        struct NothingToDo {
            remaining: Cell<u32>,
            dir: PathBuf,
        }

        #[async_trait(?Send)]
        impl Workflow<Trail> for NothingToDo {
            fn remaining(&self) -> &Cell<u32> {
                &self.remaining
            }

            async fn precheck(&self, _ctx: &mut Context<Trail>) -> Outcome<Option<String>> {
                Ok(Some("skip — draft PR".to_string()))
            }

            fn lock(&self) -> Option<Lock<'_>> {
                Some(Lock {
                    locks: &DirLocks,
                    dir: &self.dir,
                    name: "32",
                })
            }

            fn round(&self, _turn: u32) -> Box<dyn Executable<Trail> + '_> {
                Box::new(Exhausted)
            }
        }

        let dir = Dir::new("precheck");
        let flow = NothingToDo {
            remaining: Cell::new(1),
            dir: dir.0.clone(),
        };
        let mut context = ctx();
        flow.execute(&mut context).await.expect("a success");
        assert_eq!(flow.remaining.get(), 1, "no round was started");
        assert!(DirLocks.acquire(&dir.0, "32").expect("still free"));
    }

    #[tokio::test]
    async fn the_lock_is_released_after_the_rounds() {
        let dir = Dir::new("released");
        let flow = Fixture::new(1, vec![local_stage("a")]).locking(dir.0.clone(), "32");
        let mut context = ctx();
        flow.execute(&mut context).await.expect("a success");
        assert!(DirLocks.acquire(&dir.0, "32").expect("re-acquired"));
    }

    #[tokio::test]
    async fn a_failure_still_releases_the_lock() {
        let dir = Dir::new("failed");
        let flow = Fixture::new(1, vec![failing_stage()]).locking(dir.0.clone(), "32");
        let mut context = ctx();
        flow.execute(&mut context).await.expect_err("must stop");
        assert!(DirLocks.acquire(&dir.0, "32").expect("released anyway"));
    }

    #[tokio::test]
    async fn a_lock_already_held_skips_the_rounds_without_erroring() {
        let dir = Dir::new("held");
        assert!(
            DirLocks
                .acquire(&dir.0, "32")
                .expect("held by someone else")
        );
        let flow = Fixture::new(1, vec![local_stage("never")]).locking(dir.0.clone(), "32");
        let mut context = ctx();
        flow.execute(&mut context)
            .await
            .expect("a success, not an error");
        assert!(context.state.borrow().is_empty(), "nothing must have run");
    }
}
