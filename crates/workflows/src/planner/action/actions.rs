//! What the planner reads for free, and what it asks of its paid session.
//!
//! [`AskForPlan`] receives its template rather than fetching it: texts live
//! with the table that sends them (`orchestration::stages`), and an action
//! that went to read them would reverse the composition's direction.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Issue, Outcome, Verdict, prompts};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::common::explore;
use crate::planner::data::state::PlannerState;
use crate::planner::data::{grounding, plan};

/// Reads, for free, what the paid step must not repeat: the milestones
/// already open under this roadmap item, and the grilling digests.
pub struct ReadContext {
    /// What lists the roadmap item's existing sub-issues.
    pub gh: Rc<dyn GitHub>,
    /// What reads the grilling digests, if either exists.
    pub disk: Rc<dyn Disk>,
    /// Where the two digests live.
    pub grill_dir: PathBuf,
}

#[async_trait(?Send)]
impl Action<PlannerState> for ReadContext {
    async fn run(&self, ctx: &mut Context<PlannerState>) -> Outcome<Verdict> {
        let number = ctx.state.roadmap().number;
        let mut existing = self.gh.sub_issues(number).await?;
        existing.sort_by_key(|issue| issue.number);
        if !existing.is_empty() {
            ctx.traces.say(&format!(
                "{} milestone(s) already open under #{number} — the plan must \
                 not reopen them",
                existing.len()
            ));
        }
        ctx.state.existing = existing;

        let business = self
            .disk
            .read_to_string(&self.grill_dir.join("business-digest.md"));
        let technical = self
            .disk
            .read_to_string(&self.grill_dir.join("technical-digest.md"));
        ctx.state.grounding = grounding::combine(business.as_deref(), technical.as_deref());
        Ok(Verdict::Continue)
    }
}

/// What's already open under the roadmap item, rendered for the prompt.
fn existing_block(existing: &[Issue]) -> String {
    if existing.is_empty() {
        return "No milestone exists yet under this roadmap item.".to_string();
    }
    let lines: Vec<String> = existing
        .iter()
        .map(|issue| format!("  - #{} {}", issue.number, issue.title))
        .collect();
    format!(
        "Milestones already open under this roadmap item — do not reopen \
         them, only plan what is still missing:\n{}",
        lines.join("\n")
    )
}

/// The grounding digests, rendered for the prompt — empty when neither
/// exists, so the template carries no empty heading.
fn grounding_block(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!(
        "Gathered ahead of time, before any human was reachable — verify \
         ground truth in the repo where it disagrees:\n\n{text}"
    )
}

/// Sends the prompt that asks for the plan, and tolerates one malformed
/// reply by asking again with the parse error attached.
///
/// Not [`harness_core::execution::Tolerance`] — that forgives a whole stage
/// and moves to the next one. This is a second attempt at the *same* stage,
/// because a model asked to emit nothing but JSON sometimes doesn't on the
/// first try. If the second attempt still doesn't parse, this action does
/// not fail: the stage's post-gate (`checks::gates::PlanParses`) does, so
/// the judgment stays out of the action that sends the prompt.
pub struct AskForPlan {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: &'static str,
    /// This roadmap item's artifacts folder — where the repo map is kept.
    pub artifacts_dir: PathBuf,
    /// `--explore`.
    pub explore: bool,
    /// What reads a kept map back.
    pub disk: Rc<dyn Disk>,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<PlannerState> for AskForPlan {
    async fn run(&self, open: &mut Open<'_, PlannerState>) -> Outcome<Verdict> {
        let roadmap = open.state.roadmap().clone();
        let existing = existing_block(&open.state.existing);
        let grounding = grounding_block(&open.state.grounding);
        let body = {
            let trimmed = roadmap.body.trim();
            if trimmed.is_empty() {
                prompts::EMPTY_BODY.to_string()
            } else {
                trimmed.to_string()
            }
        };
        let filled = splice(
            self.template,
            &[
                ("num", &roadmap.number.to_string()),
                ("title", &roadmap.title),
                ("body", &body),
                ("existing", &existing),
                ("grounding", &grounding),
            ],
        );
        let map_file = explore::map_file_path(&self.artifacts_dir);
        let prefix = explore::repo_context(open.ctx, self.explore, &map_file, self.disk.as_ref());
        let prompt = format!("{prefix}\n\n{filled}");
        let task = format!("#{}", roadmap.number);

        let reply =
            ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        if plan::parse(&reply.text).is_ok() {
            return Ok(Verdict::Continue);
        }
        let error = plan::parse(&reply.text).expect_err("just failed above");
        let retry_prompt = format!(
            "{prompt}\n\nYour previous answer's JSON did not parse: {error}. Reply \
             again, with ONLY the JSON array this time — no prose before or after it."
        );
        ask_and_record(
            open,
            &retry_prompt,
            &self.stage,
            1,
            &task,
            self.spending.as_ref(),
        )
        .await?;
        Ok(Verdict::Continue)
    }
}
