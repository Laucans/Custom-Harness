//! The session's question: write `data/model.json` from the dump and the code.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Open, SessionAction, ask_and_record};
use harness_core::ports::store::spending::Spending;

use crate::data_model_draft::data::state::DraftState;

/// Asks the session to write the model.
pub struct AskDraft {
    /// The stage name, for the journal and the ledger.
    pub stage: String,
    /// The prompt, `{base}`, `{sha}`, `{schema}` still to fill.
    pub template: String,
    /// The base branch.
    pub base: String,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<DraftState> for AskDraft {
    async fn run(&self, open: &mut Open<'_, DraftState>) -> Outcome<Verdict> {
        let sha = open.state.sha.clone();
        let schema = open.state.schema_path.display().to_string();
        let prompt = splice(
            &self.template,
            &[("base", &self.base), ("sha", &sha), ("schema", &schema)],
        );
        let task = format!("model-draft {}", self.base);
        ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}
