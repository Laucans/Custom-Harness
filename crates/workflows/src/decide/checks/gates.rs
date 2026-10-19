//! What the decision's paid stage requires before it costs anything, and
//! what it must obtain.

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::decide::data::decision;
use crate::decide::data::state::DecideState;

/// Someone must decide: an owner's answer under `## Decision` leaves
/// nothing to pay for.
pub struct NeedsASession;

#[async_trait(?Send)]
impl Verification<DecideState> for NeedsASession {
    fn purpose(&self) -> String {
        "no owner answered under ## Decision already".to_string()
    }

    async fn verify(&self, ctx: &Context<DecideState>) -> Outcome<Verdict> {
        Ok(if ctx.state.owner_decided {
            Verdict::Skip("the owner already decided".to_string())
        } else {
            Verdict::Continue
        })
    }
}

/// The session's reply reads as a decision this issue can carry out.
pub struct DecisionParses {
    /// The stage whose reply this judges.
    pub stage: String,
}

#[async_trait(?Send)]
impl Verification<DecideState> for DecisionParses {
    fn purpose(&self) -> String {
        "the reply reads as a decision: an action this issue allows, a summary".to_string()
    }

    async fn verify(&self, ctx: &Context<DecideState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || ctx.state.owner_decided {
            return Ok(Verdict::Continue);
        }
        let Some(reply) = ctx.results.get(&self.stage) else {
            return Err(Halt::Failed(format!(
                "no reply recorded under {} — the decide stage did not run",
                self.stage
            )));
        };
        decision::parse(
            &reply.text,
            ctx.state.pr.is_some(),
            ctx.state.issue().number,
        )
        .map(|_| Verdict::Continue)
        .map_err(|why| {
            Halt::Failed(format!(
                "the decision did not read, even after one retry: {why}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    fn ctx(answer: Option<&str>, owner: bool) -> Context<DecideState> {
        let mut ctx = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            DecideState {
                issue: Some(Issue::default()),
                owner_decided: owner,
                ..DecideState::default()
            },
            Logbook::null(),
        );
        if let Some(text) = answer {
            ctx.results.insert(
                "decide".to_string(),
                Reply {
                    text: text.to_string(),
                    stop_line: None,
                    spend: Spend::default(),
                },
            );
        }
        ctx
    }

    #[tokio::test]
    async fn an_owner_s_answer_spares_the_session() {
        assert!(NeedsASession.purpose().contains("owner"));
        assert_eq!(
            NeedsASession
                .verify(&ctx(None, true))
                .await
                .expect("verdict"),
            Verdict::Skip("the owner already decided".to_string())
        );
        assert_eq!(
            NeedsASession
                .verify(&ctx(None, false))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_reply_that_is_no_decision_stops_the_round() {
        let gate = DecisionParses {
            stage: "decide".to_string(),
        };
        assert!(gate.purpose().contains("decision"));
        let good = ctx(Some(r#"{"action":"answer","summary":"go"}"#), false);
        assert_eq!(gate.verify(&good).await.expect("reads"), Verdict::Continue);
        let merge = ctx(Some(r#"{"action":"merge","summary":"go"}"#), false);
        assert!(gate.verify(&merge).await.is_err(), "no PR to merge");
        assert!(gate.verify(&ctx(None, false)).await.is_err(), "no reply");
        assert_eq!(
            gate.verify(&ctx(None, true)).await.expect("owner"),
            Verdict::Continue
        );
    }
}
