//! What the rules' paid stages require before they cost anything.
//!
//! One gate: the issue's request was read. An issue whose body cannot be
//! read is a `Skip`, not an error — the free stage already told it why.

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::rules::data::state::RulesState;

/// The issue says what it asks.
pub struct AsksSomething;

#[async_trait(?Send)]
impl Verification<RulesState> for AsksSomething {
    fn purpose(&self) -> String {
        "the rules' issue says what it asks".to_string()
    }

    async fn verify(&self, ctx: &Context<RulesState>) -> Outcome<Verdict> {
        Ok(if ctx.state.ask.is_some() {
            Verdict::Continue
        } else {
            Verdict::Skip(
                ctx.state
                    .unreadable
                    .clone()
                    .unwrap_or_else(|| "nothing asked".to_string()),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::data::request::Ask;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(ask: Option<Ask>) -> Context<RulesState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            RulesState {
                ask,
                unreadable: Some("no json block".to_string()),
                ..RulesState::default()
            },
            Logbook::null(),
        )
    }

    #[test]
    fn its_purpose_says_what_it_checks() {
        assert!(AsksSomething.purpose().contains("asks"));
    }

    #[tokio::test]
    async fn a_request_read_opens_the_paid_stages_and_none_skips_them() {
        let ask = Ask::Contest {
            said: "R1".to_string(),
        };
        assert_eq!(
            AsksSomething
                .verify(&ctx(Some(ask)))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
        assert_eq!(
            AsksSomething.verify(&ctx(None)).await.expect("verdict"),
            Verdict::Skip("no json block".to_string())
        );
    }
}
