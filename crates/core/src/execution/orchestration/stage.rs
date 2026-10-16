//! The stage: the point of variation in the hierarchy.
//!
//! A Stage pays a session, or costs nothing — that's the only thing that
//! varies. A `Round` only ever holds a list of `Stage`: the diagram hierarchy
//! is held by types, not by convention.

use std::rc::Rc;

use async_trait::async_trait;

use crate::domain::{Outcome, Verdict};
use crate::execution::action::kinds::{Action, Open, SessionAction};
use crate::execution::checks::gate::Gate;
use crate::execution::data::context::Context;
use crate::execution::traits::Executable;
use crate::ports::agent::{SessionFactory, SessionSpec};

/// The two forms a Stage can take.
///
/// Closed by design: a workflow cannot invent a third way to run a stage,
/// and a `match` on this enumeration is always exhaustive.
pub enum StageBody<S> {
    /// An open session, multiple actions that communicate with it.
    Session {
        /// The model and effort requested at opening.
        spec: SessionSpec,
        /// What opens the session — the only test seam for this stage. An `Rc`,
        /// not a `Box`: stages of the same workflow typically share the same
        /// bearer.
        sessions: Rc<dyn SessionFactory>,
        /// In order. Multiple actions in the same session, where Python burned
        /// `/tech-analyst` and `/code` in a single prompt via `StageSpec.lead`.
        actions: Vec<Box<dyn SessionAction<S>>>,
        /// How many model turns the session may take, at most. `None`: the
        /// carrier's own limit. The safety net against a stage that loops —
        /// generous, because a cap hit mid-task wastes every turn before it.
        turn_cap: Option<u32>,
    },
    /// One or more local calls — nothing is paid.
    Local {
        /// In order.
        actions: Vec<Box<dyn Action<S>>>,
    },
}

/// A step of a round: its gates, and what it executes.
pub struct Stage<S> {
    /// How the stage is named in the logs.
    pub name: String,
    /// What must hold before paying.
    pub pre: Option<Gate<S>>,
    /// What must have been obtained after.
    pub post: Option<Gate<S>>,
    /// Open session, or local calls.
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
                turn_cap,
            } => {
                ctx.traces.say(&crate::traces::session_opens(&self.name));
                let spec = ctx.session_spec.as_ref().unwrap_or(spec);
                let mut backend = if let Some(base) = &ctx.base {
                    sessions.fork(spec, base).await?
                } else {
                    let opening = crate::ports::agent::Opening {
                        resume: ctx.resume.get(&self.name).cloned(),
                        max_turns: *turn_cap,
                    };
                    if let Some(id) = &opening.resume {
                        ctx.traces.say(&format!(
                            "{}: resuming the session {id} a previous run left interrupted",
                            self.name
                        ));
                    }
                    sessions.open_with(spec, &opening).await?
                };
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
    use crate::domain::Halt;
    use crate::execution::checks::gate::Gate;
    use crate::execution::data::context::Settings;
    use crate::execution::traits::{Guarded, Verification};
    use crate::ports::agent::{Base, Reply};
    use crate::traces::Logbook;

    /// A session that says which base it branched from, if any.
    struct FakeSession {
        from: Option<String>,
    }

    #[async_trait(?Send)]
    impl crate::ports::agent::Session for FakeSession {
        async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
            let text = self.from.as_ref().map_or_else(
                || format!("answered {prompt}"),
                |base| format!("{base} answered {prompt}"),
            );
            Ok(Reply {
                text,
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
        ) -> Outcome<Box<dyn crate::ports::agent::Session>> {
            Ok(Box::new(FakeSession { from: None }))
        }

        async fn fork(
            &self,
            _spec: &SessionSpec,
            base: &Base,
        ) -> Outcome<Box<dyn crate::ports::agent::Session>> {
            Ok(Box::new(FakeSession {
                from: Some(base.session.clone()),
            }))
        }

        /// Says what it was opened with, when it was told anything.
        async fn open_with(
            &self,
            _spec: &SessionSpec,
            opening: &crate::ports::agent::Opening,
        ) -> Outcome<Box<dyn crate::ports::agent::Session>> {
            let from = (opening.resume.is_some() || opening.max_turns.is_some()).then(|| {
                format!(
                    "{} capped {}",
                    opening.resume.as_deref().unwrap_or("fresh"),
                    opening
                        .max_turns
                        .map_or("never".to_string(), |c| c.to_string())
                )
            });
            Ok(Box::new(FakeSession { from }))
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
                turn_cap: None,
                spec: SessionSpec {
                    model: "opus".into(),
                    effort: "high".into(),
                },
                sessions: Rc::new(FakeFactory),
                // What `lead` mimicked on the Python side: one session, two
                // commands.
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

    fn capped(name: &str, cap: Option<u32>) -> Stage<Vec<String>> {
        Stage {
            name: name.into(),
            pre: None,
            post: None,
            body: StageBody::Session {
                turn_cap: cap,
                spec: SessionSpec {
                    model: "opus".into(),
                    effort: "high".into(),
                },
                sessions: Rc::new(FakeFactory),
                actions: vec![Box::new(Ask { prompt: "/code" })],
            },
        }
    }

    #[tokio::test]
    async fn a_session_stage_resumes_its_interrupted_session_under_its_cap() {
        let mut context = ctx();
        context.resume.insert("code".into(), "s-7".into());
        context.resume.insert("create-test".into(), "s-8".into());
        capped("code", Some(9)).execute(&mut context).await.unwrap();
        capped("review", Some(3))
            .execute(&mut context)
            .await
            .unwrap();
        capped("plain", None).execute(&mut context).await.unwrap();
        assert_eq!(
            context.state,
            vec![
                "s-7 capped 9 answered /code".to_string(),
                "fresh capped 3 answered /code".to_string(),
                "answered /code".to_string(),
            ],
            "its own stage's session only, and its own cap"
        );
    }

    #[tokio::test]
    async fn a_session_stage_branches_from_the_base_the_round_holds() {
        let stage = Stage {
            name: "coherence".into(),
            pre: None,
            post: None,
            body: StageBody::Session {
                turn_cap: None,
                spec: SessionSpec {
                    model: "sonnet".into(),
                    effort: "high".into(),
                },
                sessions: Rc::new(FakeFactory),
                actions: vec![Box::new(Ask { prompt: "/check" })],
            },
        };
        let mut context = ctx();
        context.base = Some(Base {
            session: "base-1".into(),
            cost_usd: Some(0.1),
        });
        stage.execute(&mut context).await.unwrap();
        assert_eq!(context.state, vec!["base-1 answered /check".to_string()]);
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
                turn_cap: None,
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
        assert_eq!(context.state, [] as [std::string::String; 0]);
    }
}
