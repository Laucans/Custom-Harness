//! Une porte : plusieurs vérifications, la première qui échoue gagne.

use async_trait::async_trait;

use crate::domain::{Outcome, Verdict};
use crate::execution::context::Context;
use crate::execution::traits::Verification;

/// Plusieurs vérifications entourant un exécutable, à n'importe quel
/// niveau — autour d'une action pour valider unitairement, autour d'une
/// stage ou d'un round pour valider l'intégré.
pub struct Gate<S> {
    /// Nommée pour qu'un lecteur liste ce qui est vérifié sans lire les
    /// corps, et pour qu'une passe d'observabilité ait quelque chose à
    /// journaliser quand chacune passe.
    pub name: &'static str,
    /// Dans l'ordre où elles se vérifient. Elles se supposent les unes les
    /// autres : demander à `gh` quelles étiquettes existent n'a pas de sens
    /// tant qu'on ne sait pas s'il est authentifié.
    pub checks: Vec<Box<dyn Verification<S>>>,
}

impl<S> Gate<S> {
    /// Une porte nommée, sans rien à vérifier.
    #[must_use]
    pub fn empty(name: &'static str) -> Self {
        Self {
            name,
            checks: Vec::new(),
        }
    }
}

#[async_trait(?Send)]
impl<S> Verification<S> for Gate<S> {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        for check in &self.checks {
            let verdict = check.verify(ctx).await?;
            if !matches!(verdict, Verdict::Continue) {
                return Ok(verdict);
            }
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Halt;
    use crate::execution::context::Settings;
    use crate::traces::Logbook;

    struct AlwaysSkip;

    #[async_trait(?Send)]
    impl Verification<()> for AlwaysSkip {
        async fn verify(&self, _ctx: &Context<()>) -> Outcome<Verdict> {
            Ok(Verdict::Skip("already done".into()))
        }
    }

    struct AlwaysHalts;

    #[async_trait(?Send)]
    impl Verification<()> for AlwaysHalts {
        async fn verify(&self, _ctx: &Context<()>) -> Outcome<Verdict> {
            Err(Halt::Halted("cannot verify".into()))
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
    async fn the_first_check_that_does_not_continue_wins() {
        let gate = Gate {
            name: "test",
            checks: vec![Box::new(AlwaysSkip)],
        };
        let verdict = gate.verify(&ctx()).await.unwrap();
        assert_eq!(verdict, Verdict::Skip("already done".into()));
    }

    #[tokio::test]
    async fn an_empty_gate_continues() {
        let gate: Gate<()> = Gate::empty("test");
        assert_eq!(gate.verify(&ctx()).await.unwrap(), Verdict::Continue);
    }

    #[tokio::test]
    async fn a_failing_check_propagates_as_a_halt() {
        let gate = Gate {
            name: "test",
            checks: vec![Box::new(AlwaysHalts)],
        };
        assert!(gate.verify(&ctx()).await.is_err());
    }
}
