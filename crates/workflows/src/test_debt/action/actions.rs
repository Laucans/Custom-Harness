//! What a test-debt attempt reads for free, and what it asks of its paid
//! session.
//!
//! [`AskForTests`] receives its template rather than fetching it: texts live
//! with the table that sends them (`orchestration::stages`), and an action
//! that went to read them would reverse the composition's direction.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::test_debt::data::debt;
use crate::test_debt::data::state::DebtState;

/// Reads, for free, what the paid step needs: the file the issue is about,
/// and everything the mutation runs said about it.
pub struct ReadDebt {
    /// What reads the issue's comments.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<DebtState> for ReadDebt {
    async fn run(&self, ctx: &mut Context<DebtState>) -> Outcome<Verdict> {
        let issue = ctx.state.issue().clone();
        let Some(file) = debt::file_of(&issue.title) else {
            ctx.traces.say(&format!(
                "{}: {:?} names no Rust source file — nothing to ask",
                issue.reference(),
                issue.title
            ));
            return Ok(Verdict::Continue);
        };
        // Read before the request is consumed: a failure here leaves the
        // label in place, so the next poll retries instead of spending it.
        let comments = self.gh.issue_comments(issue.number).await?;
        ctx.traces.say(&format!(
            "{}: test debt of {file} — {} later finding(s)",
            issue.reference(),
            comments.len()
        ));
        ctx.state.findings = debt::findings(&issue.body, &comments);
        ctx.state.file = Some(file);
        Ok(Verdict::Continue)
    }
}

/// Sends the prompt that asks for the tests.
///
/// One attempt, no retry: whether the mutants are caught is read by the next
/// rotation that judges the file, not from what the session claims.
pub struct AskForTests {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: String,
    /// The integration branch the tests' pull request targets.
    pub branch: String,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<DebtState> for AskForTests {
    async fn run(&self, open: &mut Open<'_, DebtState>) -> Outcome<Verdict> {
        let issue = open.state.issue().clone();
        let file = open.state.file.clone().unwrap_or_default();
        let findings = open.state.findings.clone();
        let num = issue.number.to_string();
        let prompt = splice(
            &self.template,
            &[
                ("num", &num),
                ("file", &file),
                ("branch", &self.branch),
                ("findings", &findings),
            ],
        );
        let task = issue.reference();
        ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use harness_core::domain::{Halt, Issue};
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(title: &str) -> Context<DebtState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            DebtState {
                issue: Some(Issue {
                    number: 7,
                    title: title.to_string(),
                    state: "open".to_string(),
                    body: "- line 3: replace f with true".to_string(),
                    ..Issue::default()
                }),
                ..DebtState::default()
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn the_file_and_every_finding_land_in_the_state() {
        let gh = Rc::new(FakeGitHub {
            issue_comments: vec!["Still missed in run 9 — 1 mutant(s)".to_string()],
            ..FakeGitHub::default()
        });
        let mut context = ctx("Test debt: crates/a/src/lib.rs");
        ReadDebt { gh }.run(&mut context).await.expect("read");
        assert_eq!(context.state.file.as_deref(), Some("crates/a/src/lib.rs"));
        assert!(context.state.findings.starts_with("- line 3"));
        assert!(context.state.findings.ends_with("1 mutant(s)"));
    }

    #[tokio::test]
    async fn a_title_naming_no_file_reads_nothing_further() {
        // A broken GitHub proves no comment was read.
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Failed("unreachable".to_string())),
            ..FakeGitHub::default()
        });
        let mut context = ctx("Test debt: README.md");
        ReadDebt { gh }
            .run(&mut context)
            .await
            .expect("not an error");
        assert!(context.state.file.is_none());
        assert!(context.state.findings.is_empty(), "expected nothing");
    }

    #[tokio::test]
    async fn unreadable_comments_fail_rather_than_ask_on_half_the_findings() {
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Failed("no credentials".to_string())),
            ..FakeGitHub::default()
        });
        let mut context = ctx("Test debt: crates/a/src/lib.rs");
        let err = ReadDebt { gh }
            .run(&mut context)
            .await
            .expect_err("must fail");
        assert!(err.reason().contains("no credentials"));
        assert!(context.state.file.is_none(), "nothing half-read");
    }
}
