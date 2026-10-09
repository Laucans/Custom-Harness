//! What a split stage requires before paying, and what it must obtain.
//!
//! Only one gate: the slice step's reply must parse, even after its own one
//! retry (`action::actions::AskForSlice`). A `Verification` judges and does
//! not write (decision #1) — the publish action re-parses the same text to
//! actually act on it, the acknowledged price of that split.

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::split::data::plan;
use crate::split::data::state::SplitState;

/// The slice stage's reply parses as a JSON array of task slices.
pub struct SliceParses {
    /// The stage whose reply this judges.
    pub stage: String,
}

#[async_trait(?Send)]
impl Verification<SplitState> for SliceParses {
    fn purpose(&self) -> String {
        "the slice stage's reply parses as a JSON array of task slices".to_string()
    }

    async fn verify(&self, ctx: &Context<SplitState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some(reply) = ctx.results.get(&self.stage) else {
            return Err(Halt::Failed(format!(
                "no reply recorded under {} — the slice stage did not run",
                self.stage
            )));
        };
        match plan::parse(&reply.text) {
            Ok(_) => Ok(Verdict::Continue),
            Err(e) => Err(Halt::Failed(format!(
                "the slice plan did not parse as JSON, even after one retry: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool) -> Context<SplitState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            SplitState {
                milestone: Some(Issue::default()),
                ..SplitState::default()
            },
            Logbook::null(),
        )
    }

    fn reply(text: &str) -> Reply {
        Reply {
            text: text.to_string(),
            stop_line: None,
            spend: Spend::default(),
        }
    }

    #[tokio::test]
    async fn a_parseable_plan_passes() {
        let mut context = ctx(false);
        context.results.insert("slice".to_string(), reply("[]"));
        let gate = SliceParses {
            stage: "slice".to_string(),
        };
        assert_eq!(
            gate.verify(&context).await.expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn an_unparseable_plan_fails_the_gate() {
        let mut context = ctx(false);
        context
            .results
            .insert("slice".to_string(), reply("not json"));
        let gate = SliceParses {
            stage: "slice".to_string(),
        };
        let err = gate.verify(&context).await.expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }

    #[tokio::test]
    async fn no_reply_at_all_fails_rather_than_reading_as_empty() {
        let context = ctx(false);
        let gate = SliceParses {
            stage: "slice".to_string(),
        };
        let err = gate.verify(&context).await.expect_err("must fail");
        assert!(err.reason().contains("did not run"));
    }

    #[tokio::test]
    async fn a_dry_run_skips_the_check_entirely() {
        let context = ctx(true);
        let gate = SliceParses {
            stage: "slice".to_string(),
        };
        assert_eq!(
            gate.verify(&context).await.expect("verdict"),
            Verdict::Continue
        );
    }
}
