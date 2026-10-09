//! The whole draft: one round, no lock — one human click is one draft.

use std::cell::Cell;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Executable, Round, Workflow};

use crate::data_model_draft::data::state::DraftState;

/// A whole draft.
pub struct DraftRun {
    /// One round.
    pub remaining: Cell<u32>,
    /// The round, already mounted.
    pub round: Round<DraftState>,
}

#[async_trait(?Send)]
impl Workflow<DraftState> for DraftRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<DraftState> + '_> {
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<DraftState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing drafted".to_string());
        }
        ctx.state.pr.as_ref().map(|url| {
            format!(
                "drafted data/model.json: {url} ({} divergence(s) left)",
                ctx.state.divergences.len()
            )
        })
    }
}

#[async_trait(?Send)]
impl Executable<DraftState> for DraftRun {
    async fn perform(&self, ctx: &mut Context<DraftState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}
