//! A generic round: a list of stages, in order, until the first one that
//! fails without being tolerated.
//!
//! Serves the workflows that don't branch. A round that has to choose between
//! several paths — like the dev loop's rollover, which veers to `/planner`
//! when there is no task — writes its own type and implements `Executable`
//! directly rather than using this one; see `docs/ROUND-DRAFT.md`, variant B,
//! and the reason for that choice.
//!
//! # Why tolerance lives here
//!
//! Tolerating a failed stage used to be a hook on the workflow
//! (`OneShot::tolerate`), which is why that trait wrote its own loop over the
//! stages and this type served nobody. Tolerance is a statement about *the
//! stages of a sequence*, so it belongs to whoever iterates them. One field,
//! not a router: the sequence stays the order of the `Vec`.

use async_trait::async_trait;

use crate::domain::{Halt, Outcome, Verdict};
use crate::execution::checks::gate::Gate;
use crate::execution::data::context::Context;
use crate::execution::orchestration::stage::Stage;
use crate::execution::traits::{Executable, Guarded};

/// Whether a failed stage can be lived with.
///
/// Not generic over the state: the decision reads the stage's name and what
/// broke, never what the workflow holds. A round that tolerates nothing
/// leaves this `None`.
pub trait Tolerance {
    /// `Some(message)` lets the sequence continue and logs that line; `None`
    /// propagates the failure.
    fn tolerate(&self, stage: &str, failed: &Halt) -> Option<String>;
}

/// A sequence of stages.
///
/// Holds strictly `Stage`s — it's the Stage that varies (session or local),
/// never the Round.
pub struct Round<S> {
    /// In execution order.
    pub stages: Vec<Stage<S>>,
    /// What the whole round must have obtained.
    pub post: Option<Gate<S>>,
    /// What a failed stage may be forgiven. `None`: nothing is.
    pub tolerance: Option<Box<dyn Tolerance>>,
}

impl<S> Round<S> {
    /// A round that tolerates nothing and verifies nothing afterwards.
    #[must_use]
    pub fn plain(stages: Vec<Stage<S>>) -> Self {
        Self {
            stages,
            post: None,
            tolerance: None,
        }
    }
}

#[async_trait(?Send)]
impl<S> Executable<S> for Round<S> {
    fn post(&self) -> Option<&Gate<S>> {
        self.post.as_ref()
    }

    async fn perform(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        for stage in &self.stages {
            if let Err(halt) = stage.execute(ctx).await {
                match self
                    .tolerance
                    .as_ref()
                    .and_then(|rule| rule.tolerate(&stage.name, &halt))
                {
                    Some(message) => ctx.traces.say(&message),
                    None => return Err(halt),
                }
            }
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::action::kinds::Action;
    use crate::execution::data::context::Settings;
    use crate::execution::orchestration::stage::StageBody;
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
        let round = Round::plain(vec![local_stage("a"), local_stage("b")]);
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

    fn halting_stage() -> Stage<Vec<String>> {
        let mut blocked = local_stage("b");
        blocked.pre = Some(Gate {
            name: "blocked",
            checks: vec![Box::new(AlwaysHalts)],
        });
        blocked
    }

    #[tokio::test]
    async fn a_stage_that_halts_stops_the_round_before_the_next_one() {
        let round = Round::plain(vec![local_stage("a"), halting_stage(), local_stage("c")]);
        let mut context = ctx();
        let err = round.execute(&mut context).await.unwrap_err();
        assert!(matches!(err, Halt::Halted(_)));
        // "b" never ran, and "c" was never reached.
        assert_eq!(context.state, vec!["a".to_string()]);
    }

    struct Forgives(&'static str);

    impl Tolerance for Forgives {
        fn tolerate(&self, stage: &str, failed: &Halt) -> Option<String> {
            (stage == self.0).then(|| format!("tolerated: {}", failed.reason()))
        }
    }

    #[tokio::test]
    async fn a_tolerated_stage_lets_the_sequence_continue() {
        let round = Round {
            stages: vec![local_stage("a"), halting_stage(), local_stage("c")],
            post: None,
            tolerance: Some(Box::new(Forgives("b"))),
        };
        let mut context = ctx();
        round.execute(&mut context).await.expect("tolerated");
        assert_eq!(context.state, vec!["a".to_string(), "c".to_string()]);
    }

    #[tokio::test]
    async fn tolerance_only_covers_the_stage_it_names() {
        let mut other = halting_stage();
        other.name = "elsewhere".to_string();
        let round = Round {
            stages: vec![other, local_stage("c")],
            post: None,
            tolerance: Some(Box::new(Forgives("b"))),
        };
        let mut context = ctx();
        let err = round.execute(&mut context).await.unwrap_err();
        assert!(matches!(err, Halt::Halted(_)));
        assert!(context.state.is_empty());
    }
}
