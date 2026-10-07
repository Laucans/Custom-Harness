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
