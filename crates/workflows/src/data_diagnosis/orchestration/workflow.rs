//! The whole diagnosis: one round, no lock — two diagnoses of the same
//! version would only cost twice.

use std::cell::Cell;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Executable, Round, Workflow};

use crate::data_diagnosis::data::state::DiagnosisState;

/// A whole diagnosis.
pub struct DiagnosisRun {
    /// One round.
    pub remaining: Cell<u32>,
    /// The round, already mounted.
    pub round: Round<DiagnosisState>,
}

#[async_trait(?Send)]
impl Workflow<DiagnosisState> for DiagnosisRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<DiagnosisState> + '_> {
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<DiagnosisState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing diagnosed".to_string());
        }
        let (branch, sha) = &ctx.state.head;
        ctx.state.diagnosis.as_ref().map(|d| {
            format!(
                "diagnosed {branch} @ {}: {} finding(s)",
                sha.chars().take(7).collect::<String>(),
                d.findings.len()
            )
        })
    }
}

#[async_trait(?Send)]
impl Executable<DiagnosisState> for DiagnosisRun {
    async fn perform(&self, ctx: &mut Context<DiagnosisState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}
