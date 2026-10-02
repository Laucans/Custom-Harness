//! Les règles que **tout** stage subit, quel que soit le workflow.
//!
//! Elles vivent ici et pas dans un workflow parce qu'aucune ne parle de ce
//! qu'un stage fait : « est-il dans ce run ? », « l'a-t-on déjà fait pour
//! cette task ? », « déclarons-le fait ». Côté Python elles étaient trois
//! branches dans le corps de `StageRunner.run`, donc impossibles à retirer
//! d'un stage, à tester seules, ou à lire dans la table du round.
//!
//! Les deux premières **jugent** et la troisième **fait** — la décision n°1
//! appliquée à une mécanique qui, avant, mélangeait les trois dans une même
//! fonction.

use async_trait::async_trait;

use crate::domain::{Outcome, Resumable, Verdict};
use crate::execution::action::Action;
use crate::execution::context::Context;
use crate::execution::traits::Verification;

/// Ce stage fait-il partie de ce run ? — le filtre `--stages`.
///
/// Rend un `Skip` **qui porte sa raison**, jamais un saut muet : un run
/// `--stages code` ne disait rien des stages qu'il laissait tomber, et le
/// journal se lisait comme un pipeline plus court qu'il ne l'est.
pub struct InThisRun {
    /// Le nom du stage, tel que `--stages` le nomme.
    pub stage: String,
}

#[async_trait(?Send)]
impl<S> Verification<S> for InThisRun {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        if ctx.settings.runs(&self.stage) {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!(
            "/{} skipped — not in --stages ({})",
            self.stage, ctx.settings.stages
        )))
    }
}

/// Ce stage a-t-il déjà tourné pour cette task, tous runs confondus ?
///
/// La borne [`Resumable`] est ce qui rend la question posable : un workflow
/// qui ne déclare pas d'état reprenable ne peut pas porter cette garde, et le
/// compilateur le dit.
pub struct StageAlreadyDone {
    /// Le nom du stage dans la liste des faits.
    pub stage: String,
}

#[async_trait(?Send)]
impl<S: Resumable> Verification<S> for StageAlreadyDone {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        if !ctx.state.is_done(&self.stage) {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!(
            "/{} already completed for this task — skipping (--restart to \
             force)",
            self.stage
        )))
    }
}

/// Déclare ce stage fait pour cette task.
///
/// L'autre moitié de [`StageAlreadyDone`], et une action parce qu'elle
/// **écrit** : elle se place dans le corps du stage, après ce qui le fait
/// tourner, pas dans une garde.
///
/// **Un dry-run ne marque rien.** Il n'exécute rien, donc il ne peut pas
/// déclarer un stage fait — sinon une répétition à blanc ferait sauter tout
/// le round suivant.
pub struct MarkDone {
    /// Le nom du stage à inscrire.
    pub stage: String,
}

#[async_trait(?Send)]
impl<S: Resumable> Action<S> for MarkDone {
    async fn run(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        if !ctx.settings.dry_run {
            ctx.state.mark(&self.stage);
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::context::Settings;
    use crate::traces::Logbook;

    #[derive(Default)]
    struct Done(Vec<String>);

    impl Resumable for Done {
        fn done(&self) -> &[String] {
            &self.0
        }

        fn mark(&mut self, stage: &str) {
            self.0.push(stage.to_string());
        }
    }

    fn ctx(stages: &str) -> Context<Done> {
        Context::new(
            Settings {
                dry_run: false,
                stages: stages.to_string(),
            },
            Done::default(),
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn an_empty_stages_filter_lets_everything_run() {
        let gate = InThisRun {
            stage: "code".to_string(),
        };
        assert_eq!(
            gate.verify(&ctx("")).await.expect("un verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_filtered_stage_says_why_it_is_being_skipped() {
        // Le mode de panne que ça évite : « pourquoi rien ne s'est passé ».
        let gate = InThisRun {
            stage: "create-test".to_string(),
        };
        let Verdict::Skip(why) = gate.verify(&ctx("code")).await.expect("un verdict") else {
            panic!("un saut, pas autre chose");
        };
        assert!(why.contains("not in --stages (code)"));
    }

    #[tokio::test]
    async fn a_stage_already_done_is_skipped_and_names_the_escape() {
        let gate = StageAlreadyDone {
            stage: "code".to_string(),
        };
        let mut context = ctx("");
        context.state.mark("code");
        let Verdict::Skip(why) = gate.verify(&context).await.expect("un verdict") else {
            panic!("un saut");
        };
        assert!(why.contains("--restart"), "dire comment le rejouer");
    }

    #[tokio::test]
    async fn marking_then_asking_is_consistent() {
        let mut context = ctx("");
        MarkDone {
            stage: "code".to_string(),
        }
        .run(&mut context)
        .await
        .expect("marque");
        let gate = StageAlreadyDone {
            stage: "code".to_string(),
        };
        assert!(matches!(
            gate.verify(&context).await.expect("un verdict"),
            Verdict::Skip(_)
        ));
    }

    #[tokio::test]
    async fn a_dry_run_marks_nothing_because_it_ran_nothing() {
        let mut context = ctx("");
        context.settings.dry_run = true;
        MarkDone {
            stage: "code".to_string(),
        }
        .run(&mut context)
        .await
        .expect("rien à marquer");
        assert!(context.state.done().is_empty());
    }
}
