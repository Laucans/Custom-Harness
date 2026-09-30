//! Un round générique : une liste de stages, dans l'ordre, jusqu'à la
//! première qui échoue.
//!
//! Sert les workflows sans branchement. Un round qui doit choisir entre
//! plusieurs chemins — comme le rollover de la boucle de dev, qui bifurque
//! vers `/planner` quand il n'y a aucune task — écrit son propre type et
//! implémente `Executable` directement plutôt que d'utiliser celui-ci ; voir
//! `docs/ROUND-DRAFT.md`, variante B, et la raison de ce choix.

use async_trait::async_trait;

use crate::domain::{Outcome, Verdict};
use crate::execution::context::Context;
use crate::execution::gate::Gate;
use crate::execution::stage::Stage;
use crate::execution::traits::{Executable, Guarded};

/// Une séquence de stages.
///
/// Tient strictement des `Stage` — c'est la Stage qui varie (session ou
/// locale), jamais le Round.
pub struct Round<S> {
    /// Dans l'ordre d'exécution.
    pub stages: Vec<Stage<S>>,
    /// Ce que le round entier doit avoir obtenu.
    pub post: Option<Gate<S>>,
}

#[async_trait(?Send)]
impl<S> Executable<S> for Round<S> {
    fn post(&self) -> Option<&Gate<S>> {
        self.post.as_ref()
    }

    async fn perform(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        for stage in &self.stages {
            stage.execute(ctx).await?;
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Halt;
    use crate::execution::action::Action;
    use crate::execution::context::Settings;
    use crate::execution::stage::StageBody;
    use crate::execution::traits::Verification;
    use crate::traces::Logbook;

    struct Push(&'static str);

    #[async_trait(?Send)]
    impl Action<Vec<String>> for Push {
        async fn run(&self, ctx: &mut Context<Vec<String>>) -> Outcome<Verdict> {
            ctx.state.push(self.0.to_string());
            Ok(Verdict::Continue)
        }
    }

    fn local_stage(tag: &'static str) -> Stage<Vec<String>> {
        Stage {
            name: tag.to_string(),
            pre: None,
            post: None,
            body: StageBody::Local {
                actions: vec![Box::new(Push(tag))],
            },
        }
    }

    fn ctx() -> Context<Vec<String>> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            Vec::new(),
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn stages_run_in_the_order_of_the_vec() {
        let round = Round {
            stages: vec![local_stage("a"), local_stage("b")],
            post: None,
        };
        let mut context = ctx();
        round.execute(&mut context).await.unwrap();
        assert_eq!(context.state, vec!["a".to_string(), "b".to_string()]);
    }

    struct AlwaysHalts;

    #[async_trait(?Send)]
    impl Verification<Vec<String>> for AlwaysHalts {
        async fn verify(&self, _ctx: &Context<Vec<String>>) -> Outcome<Verdict> {
            Err(Halt::Halted("blocked".into()))
        }
    }

    #[tokio::test]
    async fn a_stage_that_halts_stops_the_round_before_the_next_one() {
        let mut blocked = local_stage("b");
        blocked.pre = Some(Gate {
            name: "blocked",
            checks: vec![Box::new(AlwaysHalts)],
        });
        let round = Round {
            stages: vec![local_stage("a"), blocked, local_stage("c")],
            post: None,
        };
        let mut context = ctx();
        let err = round.execute(&mut context).await.unwrap_err();
        assert!(matches!(err, Halt::Halted(_)));
        // "b" n'a jamais tourné, et "c" n'a jamais été atteint.
        assert_eq!(context.state, vec!["a".to_string()]);
    }
}
