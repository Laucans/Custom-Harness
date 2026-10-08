//! What the review asks of its sessions: the line-by-line pass, then the
//! summary.
//!
//! Both receive their text rather than fetching it: templates live with the
//! table that sends them
//! ([`orchestration::stages`](crate::pr_review::orchestration::stages)), and
//! an action that went to read them would reverse the composition's direction.
//!
//! A complete prompt and nothing else: a review has no preamble or scope,
//! unlike the loop — each pass is an entire text.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Open, SessionAction, ask_and_record};
use harness_core::ports::store::spending::Spending;

use crate::pr_review::data::findings;
use crate::pr_review::data::state::ReviewState;

/// Sends the `/code-review` command in its own session.
pub struct AskInline {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The level passed to `/code-review`.
    pub level: String,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<ReviewState> for AskInline {
    async fn run(&self, open: &mut Open<'_, ReviewState>) -> Outcome<Verdict> {
        let num = open.state.pr().num.clone();
        let prompt = format!("/code-review {} {num} --comment", self.level);
        ask_and_record(open, &prompt, &self.stage, 1, &num, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}

/// What the data layer is held to, on top of the common bar — or nothing.
const fn data_layer_bar(data_layer: bool) -> &'static str {
    if data_layer {
        "This pull request delivers a milestone's **data layer** (`harness:data-layer`):
its aggregates, invariants, migrations and DataCapabilities, which every
reader of the milestone will build on. Hold it to the write side's bar —
these are blocking too:
  - an invariant with no test that fails when it is violated;
  - a migration that does not replay on a fresh schema, or that drops or
    rewrites existing data without an expand/contract step;
  - a DataCapability whose declared `effect` and `touches` do not match what
    its code writes;
  - a privilege that lets the read-only role write, or the application role
    bypass the DataGuard.
"
    } else {
        ""
    }
}

/// Composes and sends the "brief" pass prompt.
pub struct AskBrief {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The name of the line-by-line pass stage, under which its response is
    /// stored in `ctx.results`. Received from the table, like any stage name.
    pub inline_stage: String,
    /// The prompt template, received from the table.
    pub template: &'static str,
    /// `--no-inline`, to tell the reviewer why they have no findings.
    pub no_inline: bool,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<ReviewState> for AskBrief {
    async fn run(&self, open: &mut Open<'_, ReviewState>) -> Outcome<Verdict> {
        let pr = open.state.pr().clone();
        let found = findings::findings(open.ctx, &self.inline_stage, self.no_inline);
        let prompt = splice(
            self.template,
            &[
                ("num", &pr.num),
                ("title", &pr.title),
                ("head", &pr.head),
                ("base", &pr.base),
                ("findings", &found),
                ("strictness", data_layer_bar(open.state.data_layer)),
            ],
        );
        ask_and_record(
            open,
            &prompt,
            &self.stage,
            1,
            &pr.num,
            self.spending.as_ref(),
        )
        .await?;
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_layer_is_held_to_a_higher_bar_and_the_rest_to_the_common_one() {
        assert!(data_layer_bar(true).contains("invariant with no test"));
        assert!(data_layer_bar(false).is_empty());
    }
}
