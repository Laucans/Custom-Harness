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

use crate::common::review;
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
        let comments = self.gh.pr_comments(&num).await?;
        let blocking = review::status(&comments).wants_a_fix();
        let conflicting = self.gh.pr_mergeable(&num).await? == Some(false)
            && review::repairs(&comments) < review::MAX_FIXES;
        if failing.is_empty() && !blocking && !conflicting {
            ctx.traces.say(&format!(
                "nothing has failed on #{num}, its review asks for nothing and it merges — \
                 nothing to repair"
            ));
            return Ok(Verdict::Continue);
        }
        if conflicting {
            ctx.traces
                .say(&format!("#{num}: its branch conflicts with its base"));
        }
        if !failing.is_empty() {
            ctx.traces.say(&format!(
                "#{num}: {} failing check(s) — {}",
                failing.len(),
                failing.join("; ")
            ));
        }
        if blocking {
            ctx.traces
                .say(&format!("#{num}: its agent review asks for changes"));
        }
        ctx.state.comments = comments;
        ctx.state.failing = failing;
        ctx.state.review_blocking = blocking;
        ctx.state.conflicting = conflicting;
        Ok(Verdict::Continue)
    }
}

/// What broke, rendered for the prompt.
fn failing_block(failing: &[String]) -> String {
    if failing.is_empty() {
        return "  (none — every check is green; the review below is why you are here)".to_string();
    }
    failing
        .iter()
        .map(|line| format!("  - {line}"))
        .collect::<Vec<String>>()
        .join("\n")
}

/// The conflict with the base, rendered for the prompt — or nothing.
fn conflict_block(conflicting: bool, base: &str) -> String {
    if !conflicting {
        return String::new();
    }
    format!(
        "This PR no longer merges: its branch conflicts with `{base}` — another PR \
         landed there since. Bring the base in and resolve it:\n  git fetch origin && \
         git merge origin/{base}\nResolve every conflict keeping both sides' intent \
         (the other PR's change is not yours to undo), run the gates, commit the \
         merge and push. A merge, never a rebase, never a force-push.\n"
    )
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
        let conflict = conflict_block(open.state.conflicting, &pr.base);
        let prompt = splice(
            self.template,
            &[
                ("num", &pr.num),
                ("title", &pr.title),
                ("head", &pr.head),
                ("base", &pr.base),
                ("failing", &failing),
                ("comments", &comments),
                ("conflict", &conflict),
            ],
        );
        let task = format!("#{}", pr.num);
        ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}

/// Leaves the attempt's marker on the PR, before the paid session.
///
/// It is what owes the PR a new review once the repair is done, and what
/// counts the repairs a blocking review gets — a session that fails still
/// used one.
pub struct RecordAttempt {
    /// What posts the marker.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<FixState> for RecordAttempt {
    async fn run(&self, ctx: &mut Context<FixState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || !ctx.state.asks_for_a_repair() {
            return Ok(Verdict::Continue);
        }
        let pr = ctx.state.pr().clone();
        let Ok(number) = pr.num.parse::<u64>() else {
            return Ok(Verdict::Continue);
        };
        let why = if ctx.state.review_blocking {
            "the agent review asked for changes"
        } else if !ctx.state.failing.is_empty() {
            "a check went red"
        } else {
            "the branch conflicted with its base"
        };
        self.gh
            .post_issue_comment(
                number,
                &format!(
                    "{}\nRepair attempt by `pr_fix` — {why}. The PR is owed a new review.",
                    review::FIX_MARKER
                ),
            )
            .await?;
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

    #[test]
    fn a_conflict_asks_for_a_merge_of_the_base_never_a_rebase() {
        let said = conflict_block(true, "milestone/53-data");
        assert!(said.contains("git merge origin/milestone/53-data"));
        assert!(said.contains("never a rebase"));
        assert_eq!(conflict_block(false, "x"), "");
    }

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
        assert_eq!(context.state.failing, [] as [std::string::String; 0]);
        assert_eq!(context.state.comments, "");
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
