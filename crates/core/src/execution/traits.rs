//! The two traits every execution level shares.

use async_trait::async_trait;

use crate::domain::{Outcome, Verdict};
use crate::execution::checks::gate::Gate;
use crate::execution::data::context::Context;

/// A check: it **judges**, it never writes into the `Context`.
///
/// The `&Context` (immutable), rather than a single trait for everything, is
/// what makes the rule "a Verification judges, an Action does" true at compile
/// time instead of being a convention — the borrow checker refuses to let a
/// `Verification` mutate the state.
#[async_trait(?Send)]
pub trait Verification<S> {
    /// Returns `Continue` if everything holds, `Skip` to skip without paying,
    /// or a `Halt` to stop the sequence.
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict>;
}

/// The work proper to one level — Workflow, Round, Stage or Action.
///
/// Distinct from [`Guarded::execute`]: `perform` is what a level writes,
/// `execute` is the pre-gate → perform → post-gate sequence that nobody
/// rewrites.
#[async_trait(?Send)]
pub trait Executable<S> {
    /// What must hold before paying. `None`: nothing to check.
    fn pre(&self) -> Option<&Gate<S>> {
        None
    }

    /// What must have been obtained afterwards. `None`: nothing to check.
    fn post(&self) -> Option<&Gate<S>> {
        None
    }

    /// The work proper to this level, between the two guards.
    async fn perform(&self, ctx: &mut Context<S>) -> Outcome<Verdict>;
}

/// A borrowed executable is one too.
///
/// What this exists for: a workflow that builds its round once and hands it
/// out every turn returns `Box::new(&self.round)`, while one that builds a
/// fresh round per turn boxes the owned value. Both satisfy
/// [`Workflow::round`](crate::execution::Workflow::round) without the core
/// having to choose between owning and borrowing for them.
#[async_trait(?Send)]
impl<S, T> Executable<S> for &T
where
    T: Executable<S> + ?Sized,
{
    fn pre(&self) -> Option<&Gate<S>> {
        (**self).pre()
    }

    fn post(&self) -> Option<&Gate<S>> {
        (**self).post()
    }

    async fn perform(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        (**self).perform(ctx).await
    }
}

/// The sequence common to every executable: pre-gate, `perform`, post-gate.
///
/// A *blanket impl* rather than a default method on `Executable`: that makes
/// the sequence impossible to bypass. A hand-written `Guarded` impl for a type
/// that already implements `Executable` would conflict with this one — there
/// is exactly one path to execute anything, and this is it. It's what
/// `contract/workflow.py` described as "fifteen lines, never rewritten", held
/// here by the compiler.
#[async_trait(?Send)]
pub trait Guarded<S> {
    /// Pre-gate → `perform` → post-gate. What the sequencer calls, and the
    /// only thing it calls.
    async fn execute(&self, ctx: &mut Context<S>) -> Outcome<Verdict>;
}

#[async_trait(?Send)]
impl<S, E> Guarded<S> for E
where
    E: Executable<S> + ?Sized,
{
    async fn execute(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        if let Some(gate) = self.pre()
            && let Verdict::Skip(why) = gate.verify(ctx).await?
        {
            ctx.traces.say(&why);
            return Ok(Verdict::Continue);
        }
        let verdict = self.perform(ctx).await?;
        if let Some(gate) = self.post() {
            gate.verify(ctx).await?;
        }
        Ok(verdict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::data::context::Settings;
    use crate::traces::Logbook;

    struct AlwaysSkips;

    #[async_trait(?Send)]
    impl Verification<()> for AlwaysSkips {
        async fn verify(&self, _ctx: &Context<()>) -> Outcome<Verdict> {
            Ok(Verdict::Skip("already done".into()))
        }
    }

    struct Counts {
        pre: Option<Gate<()>>,
    }

    #[async_trait(?Send)]
    impl Executable<()> for Counts {
        fn pre(&self) -> Option<&Gate<()>> {
            self.pre.as_ref()
        }

        async fn perform(&self, ctx: &mut Context<()>) -> Outcome<Verdict> {
            ctx.traces.say("performed");
            Ok(Verdict::Continue)
        }
    }

    fn ctx() -> Context<()> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            (),
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn a_pre_gate_that_skips_never_runs_perform() {
        let stage = Counts {
            pre: Some(Gate {
                name: "test",
                checks: vec![Box::new(AlwaysSkips)],
            }),
        };
        let mut context = ctx();
        let verdict = stage.execute(&mut context).await.unwrap();
        // Continue, not Skip: the parent carries on, only the logged line says
        // that perform was skipped.
        assert_eq!(verdict, Verdict::Continue);
    }

    #[tokio::test]
    async fn without_a_pre_gate_perform_runs() {
        let stage = Counts { pre: None };
        let mut context = ctx();
        stage.execute(&mut context).await.unwrap();
    }

    #[tokio::test]
    async fn a_borrowed_executable_keeps_its_gates() {
        // What this guards: a workflow handing out `&self.round` every turn
        // must not lose that round's post-gate on the way.
        let stage = Counts {
            pre: Some(Gate {
                name: "test",
                checks: vec![Box::new(AlwaysSkips)],
            }),
        };
        let borrowed = &stage;
        let mut context = ctx();
        assert!(borrowed.pre().is_some());
        assert_eq!(
            borrowed.execute(&mut context).await.unwrap(),
            Verdict::Continue
        );
    }
}
