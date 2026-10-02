//! Ce qu'une étape de la revue exige, et ce qui la fait sauter.
//!
//! À ne pas confondre avec les portes du **workflow** (outillage, vérifiées
//! une fois avant tout, montées par le lanceur). Celles-ci n'ont de sens que
//! dans la séquence de la revue.

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::pr_review::state::ReviewState;

/// Ce que la passe « brief » reçoit quand il n'y a rien de la passe « inline ».
///
/// Les deux cas se distinguent, parce qu'ils ne veulent pas dire la même
/// chose au relecteur : l'un dit que la passe a été désactivée, l'autre
/// qu'elle a tourné et n'a rien laissé.
/// La passe a été désactivée par `--no-inline`.
pub const NOT_ASKED: &str = "(inline pass skipped)";
/// La passe a tourné (ou a échoué, toléré) et n'a rien laissé.
pub const NOTHING_BACK: &str = "(the inline pass did not run; no findings were posted)";

/// `--no-inline` : le commentaire de synthèse, et rien d'autre.
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

/// Un dry-run ne poste rien : il a écrit les prompts et s'arrête là.
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

/// Ce que la passe « inline » a rendu, ou pourquoi il n'y a rien.
#[must_use]
pub fn findings(ctx: &Context<ReviewState>, no_inline: bool) -> String {
    match ctx.results.get("inline") {
        Some(reply) => reply.text.clone(),
        None if no_inline => NOT_ASKED.to_string(),
        None => NOTHING_BACK.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::adapters::agent::Reply;
    use harness_core::domain::Spend;
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
            panic!("un saut");
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

    #[test]
    fn findings_distinguishes_disabled_from_empty() {
        assert_eq!(findings(&ctx(false), true), NOT_ASKED);
        assert_eq!(findings(&ctx(false), false), NOTHING_BACK);
    }

    #[test]
    fn findings_reads_what_the_inline_pass_actually_said() {
        let mut context = ctx(false);
        context.results.insert(
            "inline".to_string(),
            Reply {
                text: "src/x.ts:12 — risque".to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        assert_eq!(findings(&context, false), "src/x.ts:12 — risque");
    }
}
