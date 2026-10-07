//! What refinement asks of its sessions, and what it records afterward.
//!
//! [`AskRefine`] receives its template rather than fetching it: texts live
//! with the table that sends them
//! ([`orchestration::prompts`](crate::refinement::orchestration::prompts)), and
//! an action that went to read them would reverse the composition's direction.
//! Same for stage names: they descend as a field from the table, the only place
//! where they are written.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::store::spending::Spending;

use crate::common::explore;
use crate::refinement::data::phase::Phase;
use crate::refinement::data::state::RefinementState;
use crate::refinement::data::{rounds, sections};

/// What a human asked for this round, verbatim. Empty when they said nothing.
fn additional_context_block(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!(
        "--- additional_context ---\nWhat the human asked for in this round, \
         verbatim:\n{text}\n--- end additional_context ---"
    )
}

/// Sends the prompt for a paid refinement step.
///
/// One type for every paid step — router, sections, coherence, advice — because they
/// compose their prompt the same way: the same template, the same scope,
/// prefixed by the repository map.
pub struct AskRefine {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: &'static str,
    /// Does this step read the body this round is about to publish, rather
    /// than the one before this round?
    ///
    /// True for coherence and the advice: they read together sections that
    /// no one else, in this round, has read together.
    pub merged_body: bool,
    /// Which half of the refinement this step belongs to — the keys it
    /// names to the model are this phase's.
    pub phase: Phase,
    /// What a human asked for this round, verbatim.
    pub context: String,
    /// This issue's artifacts folder.
    pub artifacts_dir: PathBuf,
    /// `--explore`.
    pub explore: bool,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<RefinementState> for AskRefine {
    async fn run(&self, open: &mut Open<'_, RefinementState>) -> Outcome<Verdict> {
        let issue = open.state.issue().clone();
        let body = if self.merged_body {
            sections::render(&sections::merge(
                &open.state.found,
                &open.state.wanted,
                &open.ctx.results,
            ))
        } else {
            let trimmed = issue.body.trim();
            if trimmed.is_empty() {
                harness_core::domain::prompts::EMPTY_BODY.to_string()
            } else {
                trimmed.to_string()
            }
        };
        let filled = splice(
            self.template,
            &[
                ("num", &issue.number.to_string()),
                ("title", &issue.title),
                ("round", &open.state.round_no.to_string()),
                ("keys", &self.phase.keys().join("\n")),
                ("drags", self.phase.drags()),
                (
                    "additional_context",
                    &additional_context_block(&self.context),
                ),
                ("body", &body),
            ],
        );
        let map_file = explore::map_file_path(&self.artifacts_dir);
        let prefix = explore::repo_context(open.ctx, self.explore, &map_file);
        let hierarchy = &open.state.hierarchy;
        let prompt = if hierarchy.is_empty() {
            format!("{prefix}\n\n{filled}")
        } else {
            format!("{prefix}\n\n{hierarchy}\n\n{filled}")
        };
        let round = open.state.round_no;
        let task = format!("#{}", issue.number);
        ask_and_record(
            open,
            &prompt,
            &self.stage,
            round,
            &task,
            self.spending.as_ref(),
        )
        .await?;
        Ok(Verdict::Continue)
    }
}

/// La moitié « écrit » de l'ex-`router_named_sections` : pose `state.wanted`
/// depuis la réponse du routeur.
///
/// Une action locale, glissée après la session — même scission que
/// `dev_loop::action::actions`.
pub struct RecordWantedSections {
    /// Le nom du stage du routeur, sous lequel sa réponse est rangée dans
    /// `ctx.results`. Reçu de la table, jamais écrit en dur ici.
    pub router: String,
}

#[async_trait(?Send)]
impl Action<RefinementState> for RecordWantedSections {
    async fn run(&self, ctx: &mut Context<RefinementState>) -> Outcome<Verdict> {
        if let Some(reply) = ctx.results.get(&self.router) {
            ctx.state.wanted = rounds::wanted_from(&reply.text, ctx.state.phase);
        }
        Ok(Verdict::Continue)
    }
}
