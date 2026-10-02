//! What an action does: either locally, or against an open session.

use std::ops::{Deref, DerefMut};

use async_trait::async_trait;

use crate::adapters::agent::Session;
use crate::domain::{Outcome, Verdict};
use crate::execution::data::context::Context;

/// An action that costs nothing: a local call (choose a task, observe state,
/// apply a label).
#[async_trait(?Send)]
pub trait Action<S> {
    /// The work of this action.
    async fn run(&self, ctx: &mut Context<S>) -> Outcome<Verdict>;
}

/// What a session action receives: the run context, and the session
/// opened by its Stage.
///
/// `Deref`/`DerefMut` to `Context<S>`: a session action reads and writes
/// state like a local action, without extra ceremony — only
/// `open.session` is added.
pub struct Open<'a, S> {
    /// The run context.
    pub ctx: &'a mut Context<S>,
    /// The session opened by the Stage that holds this action.
    pub session: &'a mut dyn Session,
}

impl<S> Deref for Open<'_, S> {
    type Target = Context<S>;

    fn deref(&self) -> &Self::Target {
        self.ctx
    }
}

impl<S> DerefMut for Open<'_, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx
    }
}

/// An action that communicates with the open session of its Stage.
///
/// Cannot exist outside a Stage: `Open` is only constructed there, and a
/// `Session` is never a field of a shared `Context` — guaranteed by types,
/// not by convention.
#[async_trait(?Send)]
pub trait SessionAction<S> {
    /// The work of this action, against the open session.
    async fn run(&self, open: &mut Open<'_, S>) -> Outcome<Verdict>;
}

/// A local action, inserted into a stage's session list.
///
/// A paid stage has local work to do **after** its session: re-read
/// what the session wrote, apply a label, declare itself done. This
/// work doesn't touch the session, and wrapping it says exactly that — the type
/// inside receives only `&mut Context`, so it *cannot* spend.
///
/// Named for what it guarantees: what's inside doesn't cost anything.
///
/// A generic `impl` of [`SessionAction`] for every [`Action`] would say the
/// same thing without wrapping, but coherence forbids it — it would conflict with
/// any hand-written impl, as the compiler cannot prove that a type does *not*
/// implement `Action`.
pub struct Unpaid<A>(pub A);

#[async_trait(?Send)]
impl<S, A> SessionAction<S> for Unpaid<A>
where
    A: Action<S>,
{
    async fn run(&self, open: &mut Open<'_, S>) -> Outcome<Verdict> {
        self.0.run(open.ctx).await
    }
}
