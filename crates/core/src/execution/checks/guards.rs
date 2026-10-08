//! The rules that **every** stage undergoes, regardless of workflow.
//!
//! They live here and not in a workflow because none of them talk about what
//! a stage does: « is it in this run? », « have we already done it for this
//! task? », « declare it done ». On the Python side they were three branches
//! in the body of `StageRunner.run`, so impossible to remove from a stage,
//! test alone, or read in the round table.
//!
//! The first two **judge** and the third **does** — decision #1 applied to a
//! mechanism that, before, mixed all three in the same function.

use async_trait::async_trait;

use crate::domain::{Outcome, Resumable, Verdict};
use crate::execution::action::kinds::Action;
use crate::execution::data::context::Context;
use crate::execution::traits::Verification;

/// Is this stage part of this run? — the `--stages` filter.
///
/// Returns a `Skip` **that carries its reason**, never a silent skip: a
/// `--stages code` run said nothing about the stages it dropped, and the log
/// read like a shorter pipeline than it is.
pub struct InThisRun {
    /// The name of the stage, as `--stages` names it.
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

/// Has this stage already run for this task, across all runs?
///
/// The [`Resumable`] bound is what makes the question possible: a workflow
/// that doesn't declare resumable state can't carry this guard, and the
/// compiler says so.
pub struct StageAlreadyDone {
    /// The name of the stage in the done list.
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

/// Declare this stage done for this task.
///
/// The other half of [`StageAlreadyDone`], and an action because it
/// **writes**: it sits in the stage body, after what makes it run, not in a
/// guard.
///
/// **A dry-run marks nothing.** It runs nothing, so it can't declare a stage
/// done — otherwise a dry run would skip the entire next round.
pub struct MarkDone {
    /// The name of the stage to record.
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
    use crate::execution::data::context::Settings;
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
            gate.verify(&ctx("")).await.expect("a verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_filtered_stage_says_why_it_is_being_skipped() {
        // The failure mode it avoids: « why nothing happened ».
        let gate = InThisRun {
            stage: "create-test".to_string(),
        };
        let Verdict::Skip(why) = gate.verify(&ctx("code")).await.expect("a verdict") else {
            panic!("a skip, nothing else");
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
        let Verdict::Skip(why) = gate.verify(&context).await.expect("a verdict") else {
            panic!("a skip");
        };
        assert!(why.contains("--restart"), "say how to replay it");
    }

    #[tokio::test]
    async fn marking_then_asking_is_consistent() {
        let mut context = ctx("");
        MarkDone {
            stage: "code".to_string(),
        }
        .run(&mut context)
        .await
        .expect("marked");
        let gate = StageAlreadyDone {
            stage: "code".to_string(),
        };
        assert!(matches!(
            gate.verify(&context).await.expect("a verdict"),
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
        .expect("nothing to mark");
        assert_eq!(context.state.done(), [] as [String; 0]);
    }
}
