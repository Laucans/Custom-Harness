//! The arbitration's gates: whether the session is needed, and whether
//! its reply is a verdict.

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::arbiter::data::state::ArbiterState;
use crate::arbiter::data::verdict;

/// The session runs only on what the harness could not settle alone, and
/// never on what is a human's: a second round, a plan to change.
pub struct NeedsASession;

#[async_trait(?Send)]
impl Verification<ArbiterState> for NeedsASession {
    fn purpose(&self) -> String {
        "an id nobody on the board produces is left, and the question is one an agent may settle"
            .to_string()
    }

    async fn verify(&self, ctx: &Context<ArbiterState>) -> Outcome<Verdict> {
        let question = ctx.state.question();
        if question.raised_again() {
            return Ok(Verdict::Skip(
                "raised a second time after an arbitration — a human's, no session".to_string(),
            ));
        }
        if question.changes_the_plan() {
            return Ok(Verdict::Skip(
                "a produces to change or a task to cut is a human's — no session".to_string(),
            ));
        }
        if ctx.state.resolution.unresolved.is_empty() {
            return Ok(Verdict::Skip(
                "every id is produced on the board or already there — settled without a session"
                    .to_string(),
            ));
        }
        Ok(Verdict::Continue)
    }
}

/// The session's reply parses as a verdict.
pub struct VerdictParses {
    /// The stage whose reply this judges.
    pub stage: String,
}

#[async_trait(?Send)]
impl Verification<ArbiterState> for VerdictParses {
    fn purpose(&self) -> String {
        "the arbitrate stage's reply parses as a JSON verdict".to_string()
    }

    async fn verify(&self, ctx: &Context<ArbiterState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some(reply) = ctx.results.get(&self.stage) else {
            return Err(Halt::Failed(format!(
                "no reply recorded under {} — the arbitrate stage did not run",
                self.stage
            )));
        };
        match verdict::parse(&reply.text) {
            Ok(_) => Ok(Verdict::Continue),
            Err(e) => Err(Halt::Failed(format!(
                "the verdict did not parse as JSON: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arbiter::data::board::Resolution;
    use crate::common::decision::Question;
    use harness_core::domain::Spend;
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    fn ctx(question: Question, unresolved: &[&str], dry_run: bool) -> Context<ArbiterState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            ArbiterState {
                question: Some(question),
                resolution: Resolution {
                    unresolved: unresolved.iter().map(|s| (*s).to_string()).collect(),
                    ..Resolution::default()
                },
                ..ArbiterState::default()
            },
            Logbook::null(),
        )
    }

    fn question() -> Question {
        Question {
            blocks: "x".to_string(),
            round: 1,
            ..Question::default()
        }
    }

    #[tokio::test]
    async fn the_session_runs_only_on_an_open_id_an_agent_may_settle() {
        let gate = NeedsASession;
        assert!(gate.purpose().contains("nobody on the board produces"));
        assert_eq!(
            gate.verify(&ctx(question(), &["Dice"], false))
                .await
                .expect("ok"),
            Verdict::Continue
        );
        assert!(matches!(
            gate.verify(&ctx(question(), &[], false)).await.expect("ok"),
            Verdict::Skip(why) if why.contains("without a session")
        ));
        let again = Question {
            round: 2,
            ..question()
        };
        assert!(matches!(
            gate.verify(&ctx(again, &["Dice"], false)).await.expect("ok"),
            Verdict::Skip(why) if why.contains("second")
        ));
        let cut = Question {
            cut: Some("halves".to_string()),
            ..question()
        };
        assert!(matches!(
            gate.verify(&ctx(cut, &["Dice"], false)).await.expect("ok"),
            Verdict::Skip(why) if why.contains("human")
        ));
    }

    #[tokio::test]
    async fn the_verdict_gate_demands_a_parsable_reply_unless_dry() {
        let gate = VerdictParses {
            stage: "arbitrate".to_string(),
        };
        assert!(gate.purpose().contains("JSON verdict"));
        let mut context = ctx(question(), &["Dice"], false);
        assert!(gate.verify(&context).await.is_err(), "no reply");
        context.results.insert(
            "arbitrate".to_string(),
            Reply {
                text: "no json".to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        assert!(gate.verify(&context).await.is_err(), "no object");
        context.results.insert(
            "arbitrate".to_string(),
            Reply {
                text: r#"{"decision":"human","confidence":5}"#.to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        assert_eq!(gate.verify(&context).await.expect("ok"), Verdict::Continue);
        let dry = ctx(question(), &["Dice"], true);
        assert_eq!(gate.verify(&dry).await.expect("ok"), Verdict::Continue);
    }
}
