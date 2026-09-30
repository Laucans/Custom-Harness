//! Les deux traits que tout niveau d'exécution partage.

use async_trait::async_trait;

use crate::domain::{Outcome, Verdict};
use crate::execution::context::Context;
use crate::execution::gate::Gate;

/// Une vérification : elle **juge**, elle n'écrit jamais dans le `Context`.
///
/// Le `&Context` (immuable), plutôt qu'un unique trait pour tout, est ce qui
/// rend la règle « Verification juge, Action fait » vraie à la compilation
/// au lieu d'être une convention — le borrow checker refuse qu'une
/// `Verification` mute l'état.
#[async_trait(?Send)]
pub trait Verification<S> {
    /// Rend `Continue` si tout tient, `Skip` pour sauter sans payer, ou un
    /// `Halt` pour arrêter la séquence.
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict>;
}

/// Le travail propre à un niveau — Workflow, Round, Stage ou Action.
///
/// Distinct de [`Guarded::execute`] : `perform` est ce qu'un niveau écrit,
/// `execute` est la séquence pré-gate → perform → post-gate que personne ne
/// réécrit.
#[async_trait(?Send)]
pub trait Executable<S> {
    /// Ce qui doit tenir avant de payer. `None` : rien à vérifier.
    fn pre(&self) -> Option<&Gate<S>> {
        None
    }

    /// Ce qui doit avoir été obtenu après. `None` : rien à vérifier.
    fn post(&self) -> Option<&Gate<S>> {
        None
    }

    /// Le travail propre à ce niveau, entre les deux gardes.
    async fn perform(&self, ctx: &mut Context<S>) -> Outcome<Verdict>;
}

/// La séquence commune à tout exécutable : pré-gate, `perform`, post-gate.
///
/// Un *blanket impl* plutôt qu'une méthode par défaut sur `Executable` : ça
/// rend la séquence non contournable. Une impl de `Guarded` écrite à la main
/// pour un type qui implémente déjà `Executable` entrerait en conflit avec
/// celle-ci — il n'y a qu'un seul chemin pour exécuter quoi que ce soit, et
/// c'est celui-là. C'est ce que `contract/workflow.py` décrivait comme
/// « quinze lignes, jamais réécrites », tenu ici par le compilateur.
#[async_trait(?Send)]
pub trait Guarded<S> {
    /// Pré-gate → `perform` → post-gate. Ce que le séquenceur appelle, et la
    /// seule chose qu'il appelle.
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
    use crate::execution::context::Settings;
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
        // Continue, pas Skip : le parent enchaîne, seule la ligne journalisée
        // dit qu'on a sauté perform.
        assert_eq!(verdict, Verdict::Continue);
    }

    #[tokio::test]
    async fn without_a_pre_gate_perform_runs() {
        let stage = Counts { pre: None };
        let mut context = ctx();
        stage.execute(&mut context).await.unwrap();
    }
}
