//! Sends the diagnosis prompt, reads the answer back into the state.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Open, SessionAction, ask_and_record};
use harness_core::ports::store::spending::Spending;

use crate::data_diagnosis::data::diagnosis::Diagnosis;
use crate::data_diagnosis::data::state::DiagnosisState;

/// Asks the session why the two sides disagree.
pub struct AskDiagnosis {
    /// The stage name, for the journal and the ledger.
    pub stage: String,
    /// The prompt, `{branch}`, `{sha}`, `{dir}`, `{divergences}` still to fill.
    pub template: String,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<DiagnosisState> for AskDiagnosis {
    async fn run(&self, open: &mut Open<'_, DiagnosisState>) -> Outcome<Verdict> {
        let (branch, sha) = open.state.head.clone();
        let dir = open.state.dir.clone();
        let divergences = open
            .state
            .divergences
            .iter()
            .map(|d| format!("- {d}"))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = splice(
            &self.template,
            &[
                ("branch", &branch),
                ("sha", &sha),
                ("dir", &dir),
                ("divergences", &divergences),
            ],
        );
        // The breaker's key: the same version diagnosed again is the same task.
        let task = format!("{branch}@{}", sha.chars().take(7).collect::<String>());
        let reply =
            ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        if open.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let diagnosis = Diagnosis::parse(&reply.text)
            .map_err(|why| Halt::Failed(format!("the diagnosis could not be read: {why}")))?;
        open.state.diagnosis = Some(diagnosis);
        Ok(Verdict::Continue)
    }
}
