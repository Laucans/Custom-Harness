//! What a review step requires, and what makes it skip.
//!
//! Don't confuse with the **workflow** gates (tooling, verified once before
//! all, mounted by the launcher). These only make sense in the review's
//! sequence.
//!
//! What the "inline" pass produced is **not** here: it's a read, not a
//! judgment, and it lives in [`data::findings`](crate::pr_review::data::findings).

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::pr_review::data::state::ReviewState;

/// `--no-inline`: the summary comment, and nothing else.
pub struct InlinePassIsOff {
    /// `--no-inline`.
    pub no_inline: bool,
}

#[async_trait(?Send)]
impl Verification<ReviewState> for InlinePassIsOff {
    async fn verify(&self, _ctx: &Context<ReviewState>) -> Outcome<Verdict> {
        if !self.no_inline {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip("pass 1/2 skipped — --no-inline".to_string()))
    }
}

/// A dry-run posts nothing: it wrote the prompts and stops there.
pub struct NothingIsPosted;

#[async_trait(?Send)]
impl Verification<ReviewState> for NothingIsPosted {
    async fn verify(&self, ctx: &Context<ReviewState>) -> Outcome<Verdict> {
        if !ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip("dry run — nothing posted".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool) -> Context<ReviewState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            ReviewState::default(),
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn no_inline_skips_the_inline_pass() {
        let gate = InlinePassIsOff { no_inline: true };
        let Verdict::Skip(why) = gate.verify(&ctx(false)).await.expect("verdict") else {
            panic!("a skip");
        };
        assert!(why.contains("--no-inline"));
    }

    #[tokio::test]
    async fn without_no_inline_the_inline_pass_runs() {
        let gate = InlinePassIsOff { no_inline: false };
        assert_eq!(
            gate.verify(&ctx(false)).await.expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_dry_run_skips_publishing() {
        assert!(matches!(
            NothingIsPosted.verify(&ctx(true)).await.expect("verdict"),
            Verdict::Skip(_)
        ));
    }
}
