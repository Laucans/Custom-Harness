//! Ce qu'une action fait : soit localement, soit contre une session ouverte.

use std::ops::{Deref, DerefMut};

use async_trait::async_trait;

use crate::adapters::agent::Session;
use crate::domain::{Outcome, Verdict};
use crate::execution::context::Context;

/// Une action qui ne paie rien : un appel local (choisir une task, constater
/// un état, poser une étiquette).
#[async_trait(?Send)]
pub trait Action<S> {
    /// Le travail de cette action.
    async fn run(&self, ctx: &mut Context<S>) -> Outcome<Verdict>;
}

/// Ce qu'une action de session reçoit : le contexte du run, et la session
/// ouverte par sa Stage.
///
/// `Deref`/`DerefMut` vers `Context<S>` : une action de session lit et écrit
/// l'état comme une action locale, sans cérémonie supplémentaire — seul
/// `open.session` s'ajoute.
pub struct Open<'a, S> {
    /// Le contexte du run.
    pub ctx: &'a mut Context<S>,
    /// La session ouverte par la Stage qui tient cette action.
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

/// Une action qui dialogue avec la session ouverte de sa Stage.
///
/// Ne peut pas exister en dehors d'une Stage : `Open` n'est construit que
/// là, et une `Session` n'est jamais un champ d'un `Context` partagé —
/// garanti par les types, pas par une convention.
#[async_trait(?Send)]
pub trait SessionAction<S> {
    /// Le travail de cette action, contre la session ouverte.
    async fn run(&self, open: &mut Open<'_, S>) -> Outcome<Verdict>;
}
