//! What split reads for free, and what it asks of its paid session.
//!
//! [`AskForSlice`] receives its template rather than fetching it: texts live
//! with the table that sends them (`orchestration::stages`), and an action
//! that went to read them would reverse the composition's direction.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Issue, Outcome, Verdict, prompts};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::split::data::plan;
use crate::split::data::state::SplitState;

/// Reads, for free, what the paid step must not repeat: the task slices
/// already open under this milestone.
pub struct ReadExistingTasks {
    /// What lists the milestone's existing sub-issues.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<SplitState> for ReadExistingTasks {
    async fn run(&self, ctx: &mut Context<SplitState>) -> Outcome<Verdict> {
        let number = ctx.state.milestone().number;
        let existing = still_open(self.gh.sub_issues(number).await?);
        if !existing.is_empty() {
            ctx.traces.say(&format!(
                "{} task(s) already open under #{number} — the slice must \
                 not reopen them",
                existing.len()
            ));
        }
        ctx.state.existing = existing;
        Ok(Verdict::Continue)
    }
}

/// The open ones, by number: a task closed as superseded or done is not a
/// slice the plan must avoid — GitHub's sub-issue list carries both.
fn still_open(mut issues: Vec<Issue>) -> Vec<Issue> {
    issues.retain(Issue::is_open);
    issues.sort_by_key(|issue| issue.number);
    issues
}

/// What's already open under the milestone, rendered for the prompt.
fn existing_block(existing: &[Issue]) -> String {
    if existing.is_empty() {
        return "No task exists yet under this milestone.".to_string();
    }
    let lines: Vec<String> = existing
        .iter()
        .map(|issue| format!("  - #{} {}", issue.number, issue.title))
        .collect();
    format!(
        "Tasks already open under this milestone — do not reopen them, only \
         plan what is still missing:\n{}",
        lines.join("\n")
    )
}

/// Sends the prompt that asks for the slice plan, and tolerates one
/// malformed reply by asking again with the parse error attached.
///
/// Not [`harness_core::execution::Tolerance`] — that forgives a whole stage
/// and moves to the next one. This is a second attempt at the *same* stage,
/// because a model asked to emit nothing but JSON sometimes doesn't on the
/// first try. If the second attempt still doesn't parse, this action does
/// not fail: the stage's post-gate (`checks::gates::SliceParses`) does, so
/// the judgment stays out of the action that sends the prompt.
pub struct AskForSlice {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: &'static str,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<SplitState> for AskForSlice {
    async fn run(&self, open: &mut Open<'_, SplitState>) -> Outcome<Verdict> {
        let milestone = open.state.milestone().clone();
        let existing = existing_block(&open.state.existing);
        let body = {
            let trimmed = milestone.body.trim();
            if trimmed.is_empty() {
                prompts::EMPTY_BODY.to_string()
            } else {
                trimmed.to_string()
            }
        };
        let prompt = splice(
            self.template,
            &[
                ("num", &milestone.number.to_string()),
                ("title", &milestone.title),
                ("body", &body),
                ("existing", &existing),
            ],
        );
        let task = format!("#{}", milestone.number);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64, state: &str) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            ..Issue::default()
        }
    }

    #[test]
    fn closed_sub_issues_are_not_tasks_already_open() {
        let kept = still_open(vec![issue(9, "open"), issue(8, "closed"), issue(7, "open")]);
        let numbers: Vec<u64> = kept.iter().map(|issue| issue.number).collect();
        assert_eq!(numbers, [7, 9]);
    }

    #[test]
    fn a_milestone_whose_tasks_were_all_closed_is_sliced_from_scratch() {
        assert_eq!(
            existing_block(&still_open(vec![issue(1, "closed")])),
            "No task exists yet under this milestone."
        );
    }
}
