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

/// Une action locale, glissée dans la liste d'une stage à session.
///
/// Une stage payante a du travail local à faire **après** sa session : relire
/// ce que la session a écrit, poser une étiquette, se déclarer faite. Ce
/// travail ne touche pas la session, et l'enrober dit exactement ça — le type
/// à l'intérieur ne reçoit qu'un `&mut Context`, donc il ne *peut* pas
/// dépenser.
///
/// Nommé pour ce qu'il garantit : ce qui est dedans ne paie rien.
///
/// Un `impl` générique de [`SessionAction`] pour tout [`Action`] dirait la
/// même chose sans l'enrobage, mais la cohérence le refuse — il entrerait en
/// conflit avec toute impl écrite à la main, le compilateur ne sachant pas
/// prouver qu'un type n'implémente *pas* `Action`.
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
