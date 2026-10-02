//! What the inline pass produced, as the summary receives it.
//!
//! A read of `ctx.results`, nothing else: neither judgment — it renders no
//! `Verdict` — nor writing. This is why it's here, not in `checks`, where
//! it used to live next to the gate that reads the same flag.

use harness_core::execution::Context;

use crate::pr_review::data::state::ReviewState;

/// What the brief pass receives when there is nothing from the inline pass.
///
/// The two cases differ, because they mean different things to the reviewer:
/// one says the pass was disabled, the other that it ran and left nothing.
/// The pass was disabled by `--no-inline`.
pub const NOT_ASKED: &str = "(inline pass skipped)";
/// The pass ran (or failed, tolerated) and left nothing.
pub const NOTHING_BACK: &str = "(the inline pass did not run; no findings were posted)";

/// What the inline pass produced, or why there is nothing.
///
/// `inline_stage` is the name under which its response is stored in
/// `ctx.results` — received, never hardcoded: stage names are written in
/// the table and cascade down (`ARCHITECTURE.md`).
#[must_use]
pub fn findings(ctx: &Context<ReviewState>, inline_stage: &str, no_inline: bool) -> String {
    match ctx.results.get(inline_stage) {
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

    /// The name the table gives to the line-by-line pass.
    const INLINE: &str = "inline";

    fn ctx() -> Context<ReviewState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            ReviewState::default(),
            Logbook::null(),
        )
    }

    #[test]
    fn findings_distinguishes_disabled_from_empty() {
        assert_eq!(findings(&ctx(), INLINE, true), NOT_ASKED);
        assert_eq!(findings(&ctx(), INLINE, false), NOTHING_BACK);
    }

    #[test]
    fn findings_reads_what_the_inline_pass_actually_said() {
        let mut context = ctx();
        context.results.insert(
            INLINE.to_string(),
            Reply {
                text: "src/x.ts:12 — risque".to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        assert_eq!(findings(&context, INLINE, false), "src/x.ts:12 — risque");
    }
}
