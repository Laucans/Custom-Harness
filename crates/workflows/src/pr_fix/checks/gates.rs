//! What the repair's paid stage requires before it costs anything.
//!
//! One gate, and it guards the only expensive thing this workflow does: a
//! session is worth opening once a check has **concluded** in failure. A PR
//! still building is not a PR to repair, and the difference is a `Skip`, not
//! an error — "CI has not finished yet" is the normal state of the world a
//! few minutes after a push.

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::pr_fix::data::state::FixState;

/// Something has actually broken on this PR.
pub struct SomethingIsRed;

#[async_trait(?Send)]
impl Verification<FixState> for SomethingIsRed {
    async fn verify(&self, ctx: &Context<FixState>) -> Outcome<Verdict> {
        if ctx.state.failing.is_empty() {
            return Ok(Verdict::Skip(
                "no check has failed — nothing to repair".to_string(),
            ));
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::Pr;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(failing: &[&str]) -> Context<FixState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            FixState {
                pr: Some(Pr::default()),
                failing: failing.iter().map(|f| (*f).to_string()).collect(),
                comments: String::new(),
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn a_failing_check_opens_the_paid_stage() {
        let gate = SomethingIsRed;
        assert_eq!(
            gate.verify(&ctx(&["ci — FAILURE"])).await.expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn nothing_broken_skips_the_stage_instead_of_failing() {
        let gate = SomethingIsRed;
        let verdict = gate.verify(&ctx(&[])).await.expect("a skip, not an error");
        assert!(matches!(verdict, Verdict::Skip(_)));
    }
}
