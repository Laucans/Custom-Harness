//! What the repair reads for free, and what it asks of its paid session.
//!
//! [`AskForFix`] receives its template rather than fetching it: texts live
//! with the table that sends them (`orchestration::stages`), and an action
//! that went to read them would reverse the composition's direction.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::pr_fix::data::state::FixState;

/// Reads, for free, what the paid step needs to know: which checks broke,
/// and what has already been said on the PR.
pub struct ReadBreakage {
    /// What reads the checks and the comments.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<FixState> for ReadBreakage {
    async fn run(&self, ctx: &mut Context<FixState>) -> Outcome<Verdict> {
        let num = ctx.state.pr().num.clone();
        // Read before the request is consumed, and read first: a failure
        // here must leave the label in place, so the next poll can retry
        // rather than silently swallow an attempt nobody got.
        let failing = self.gh.pr_failing_checks(&num).await?;
        if failing.is_empty() {
            ctx.traces
                .say(&format!("nothing has failed on #{num} — nothing to repair"));
            return Ok(Verdict::Continue);
        }
        ctx.traces.say(&format!(
            "#{num}: {} failing check(s) — {}",
            failing.len(),
            failing.join("; ")
        ));
        ctx.state.comments = self.gh.pr_comments(&num).await?;
        ctx.state.failing = failing;
        Ok(Verdict::Continue)
    }
}

/// What broke, rendered for the prompt.
fn failing_block(failing: &[String]) -> String {
    failing
        .iter()
        .map(|line| format!("  - {line}"))
        .collect::<Vec<String>>()
        .join("\n")
}

/// What was already said on the PR, rendered for the prompt.
fn comments_block(comments: &str) -> String {
    let trimmed = comments.trim();
    if trimmed.is_empty() {
        "Nothing has been commented on this PR.".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Sends the prompt that asks for the repair.
///
/// One attempt, no retry: unlike a reply that must parse
/// (`split`/`planner`), there is nothing here a second ask would fix that
/// the first could not. Whether the repair worked is read from CI on a
/// later poll, not from what the session claims.
pub struct AskForFix {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: &'static str,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<FixState> for AskForFix {
    async fn run(&self, open: &mut Open<'_, FixState>) -> Outcome<Verdict> {
        let pr = open.state.pr().clone();
        let failing = failing_block(&open.state.failing);
        let comments = comments_block(&open.state.comments);
        let prompt = splice(
            self.template,
            &[
                ("num", &pr.num),
                ("title", &pr.title),
                ("head", &pr.head),
                ("base", &pr.base),
                ("failing", &failing),
                ("comments", &comments),
            ],
        );
        let task = format!("#{}", pr.num);
        ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use harness_core::domain::{Halt, Pr};
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;
    use std::collections::HashMap;

    fn ctx() -> Context<FixState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            FixState {
                pr: Some(Pr {
                    num: "32".to_string(),
                    ..Pr::default()
                }),
                ..FixState::default()
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn what_broke_and_what_was_said_both_land_in_the_state() {
        let gh = Rc::new(FakeGitHub {
            failing_checks: HashMap::from([(
                "32".to_string(),
                vec!["ci — FAILURE — https://x/1".to_string()],
            )]),
            pr_comment_bodies: vec![("32".to_string(), "the test is flaky".to_string())],
            ..FakeGitHub::default()
        });
        let mut context = ctx();
        ReadBreakage { gh }.run(&mut context).await.expect("read");
        assert_eq!(context.state.failing.len(), 1);
        assert!(context.state.comments.contains("flaky"));
    }

    #[tokio::test]
    async fn a_green_pr_reads_nothing_further_and_leaves_the_state_empty() {
        // The comments read is skipped too: there is no prompt to compose.
        let gh = Rc::new(FakeGitHub {
            failing_checks: HashMap::from([("32".to_string(), Vec::new())]),
            ..FakeGitHub::default()
        });
        let mut context = ctx();
        ReadBreakage { gh }.run(&mut context).await.expect("read");
        assert!(context.state.failing.is_empty());
        assert!(context.state.comments.is_empty());
    }

    #[tokio::test]
    async fn an_unreadable_check_list_fails_rather_than_reading_as_nothing_broken() {
        // Otherwise an expired token would look exactly like a green PR,
        // and the attempt would be consumed for nothing.
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Failed("no credentials".to_string())),
            ..FakeGitHub::default()
        });
        let mut context = ctx();
        let err = ReadBreakage { gh }
            .run(&mut context)
            .await
            .expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }

    #[test]
    fn an_empty_comment_blob_is_said_in_words_rather_than_left_blank() {
        assert!(comments_block("  \n ").contains("Nothing has been commented"));
    }

    #[test]
    fn every_failing_check_is_listed_as_its_own_bullet() {
        let block = failing_block(&["ci — FAILURE".to_string(), "fmt — ERROR".to_string()]);
        assert_eq!(block.lines().count(), 2);
        assert!(block.starts_with("  - ci"));
    }
}
