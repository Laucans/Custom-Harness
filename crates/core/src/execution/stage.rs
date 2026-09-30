//! La stage : le point de variation de la hiérarchie.
//!
//! Une Stage paye une session, ou ne paie rien — c'est la seule chose qui
//! varie. Un `Round` ne tient jamais qu'une liste de `Stage` : la hiérarchie
//! du diagramme est tenue par les types, pas par une convention.

use std::rc::Rc;

use async_trait::async_trait;

use crate::adapters::agent::{SessionFactory, SessionSpec};
use crate::domain::{Outcome, Verdict};
use crate::execution::action::{Action, Open, SessionAction};
use crate::execution::context::Context;
use crate::execution::gate::Gate;
use crate::execution::traits::Executable;

/// Les deux formes qu'une Stage peut prendre.
///
/// Fermé à dessein : un workflow ne peut pas inventer une troisième façon de
/// faire tourner une stage, et un `match` sur cette énumération est toujours
/// exhaustif.
pub enum StageBody<S> {
    /// Une session ouverte, plusieurs actions qui dialoguent avec elle.
    Session {
        /// Le modèle et l'effort demandés à l'ouverture.
        spec: SessionSpec,
        /// Ce qui ouvre la session — la seule couture de test pour cette
        /// stage. Un `Rc`, pas un `Box` : les stages d'un même workflow
        /// partagent typiquement le même porteur.
        sessions: Rc<dyn SessionFactory>,
        /// Dans l'ordre. Plusieurs actions dans la même session, là où le
        /// Python cramait `/tech-analyst` et `/code` dans un seul prompt via
        /// `StageSpec.lead`.
        actions: Vec<Box<dyn SessionAction<S>>>,
    },
    /// Un ou plusieurs appels locaux — rien n'est payé.
    Local {
        /// Dans l'ordre.
        actions: Vec<Box<dyn Action<S>>>,
    },
}

/// Une étape d'un round : ses gates, et ce qu'elle exécute.
pub struct Stage<S> {
    /// Comment la stage est nommée dans les journaux.
    pub name: String,
    /// Ce qui doit tenir avant de payer.
    pub pre: Option<Gate<S>>,
    /// Ce qui doit avoir été obtenu après.
    pub post: Option<Gate<S>>,
    /// Session ouverte, ou appels locaux.
    pub body: StageBody<S>,
}

#[async_trait(?Send)]
impl<S> Executable<S> for Stage<S> {
    fn pre(&self) -> Option<&Gate<S>> {
        self.pre.as_ref()
    }

    fn post(&self) -> Option<&Gate<S>> {
        self.post.as_ref()
    }

    async fn perform(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        match &self.body {
            StageBody::Local { actions } => {
                for action in actions {
                    action.run(ctx).await?;
                }
                Ok(Verdict::Continue)
            }
            StageBody::Session {
                spec,
                sessions,
                actions,
            } => {
                let mut backend = sessions.open(spec).await?;
                let mut open = Open {
                    ctx,
                    session: &mut *backend,
                };
                for action in actions {
                    action.run(&mut open).await?;
                }
                Ok(Verdict::Continue)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::agent::Reply;
    use crate::domain::Halt;
    use crate::execution::context::Settings;
    use crate::execution::gate::Gate;
    use crate::execution::traits::{Guarded, Verification};
    use crate::traces::Logbook;

    struct FakeSession;

    #[async_trait(?Send)]
    impl crate::adapters::agent::Session for FakeSession {
        async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
            Ok(Reply {
                text: format!("answered {prompt}"),
                stop_line: None,
                spend: crate::domain::Spend {
                    cost_usd: Some(0.01),
                    ..crate::domain::Spend::default()
                },
            })
        }
    }

    struct FakeFactory;

    #[async_trait(?Send)]
    impl SessionFactory for FakeFactory {
        async fn open(
            &self,
            _spec: &SessionSpec,
        ) -> Outcome<Box<dyn crate::adapters::agent::Session>> {
            Ok(Box::new(FakeSession))
        }
    }

    struct Ask {
        prompt: &'static str,
    }

    #[async_trait(?Send)]
    impl SessionAction<Vec<String>> for Ask {
        async fn run(&self, open: &mut Open<'_, Vec<String>>) -> Outcome<Verdict> {
            let reply = open.session.ask(self.prompt).await?;
            open.state.push(reply.text);
            Ok(Verdict::Continue)
        }
    }

    struct RecordLocal;

    #[async_trait(?Send)]
    impl Action<Vec<String>> for RecordLocal {
        async fn run(&self, ctx: &mut Context<Vec<String>>) -> Outcome<Verdict> {
            ctx.state.push("local".into());
            Ok(Verdict::Continue)
        }
    }

    struct AlwaysHalts;

    #[async_trait(?Send)]
    impl Verification<Vec<String>> for AlwaysHalts {
        async fn verify(&self, _ctx: &Context<Vec<String>>) -> Outcome<Verdict> {
            Err(Halt::Halted("blocked".into()))
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
    async fn a_session_stage_runs_its_actions_against_one_open_session() {
        let stage = Stage {
            name: "code".into(),
            pre: None,
            post: None,
            body: StageBody::Session {
                spec: SessionSpec {
                    model: "opus".into(),
                    effort: "high".into(),
                },
                sessions: Rc::new(FakeFactory),
                // Ce que `lead` imitait côté Python : une session, deux
                // commandes.
                actions: vec![
                    Box::new(Ask {
                        prompt: "/tech-analyst",
                    }),
                    Box::new(Ask { prompt: "/code" }),
                ],
            },
        };
        let mut context = ctx();
        stage.execute(&mut context).await.unwrap();
        assert_eq!(
            context.state,
            vec![
                "answered /tech-analyst".to_string(),
                "answered /code".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn a_local_stage_never_touches_a_session_factory() {
        let stage = Stage {
            name: "pick-task".into(),
            pre: None,
            post: None,
            body: StageBody::Local {
                actions: vec![Box::new(RecordLocal)],
            },
        };
        let mut context = ctx();
        stage.execute(&mut context).await.unwrap();
        assert_eq!(context.state, vec!["local".to_string()]);
    }

    #[tokio::test]
    async fn a_pre_gate_that_halts_stops_before_any_session_opens() {
        let stage = Stage {
            name: "code".into(),
            pre: Some(Gate {
                name: "blocked",
                checks: vec![Box::new(AlwaysHalts)],
            }),
            post: None,
            body: StageBody::Session {
                spec: SessionSpec {
                    model: "opus".into(),
                    effort: "high".into(),
                },
                sessions: Rc::new(FakeFactory),
                actions: vec![Box::new(Ask { prompt: "/code" })],
            },
        };
        let mut context = ctx();
        let err = stage.execute(&mut context).await.unwrap_err();
        assert!(matches!(err, Halt::Halted(_)));
        assert!(context.state.is_empty());
    }
}
