//! What a stage of the round requires before paying, and what it must obtain.
//!
//! Don't confuse with the **workflow** gates, verified once before and after
//! the run. These only make sense in the round.
//!
//! # The two hybrids, split
//!
//! Decision #1 says a `Verification` judges and doesn't write — and the borrow
//! checker holds that. Yet two Python guards **did write**:
//!
//! | Python | what it did | becomes here |
//! | --- | --- | --- |
//! | `spec_is_in_the_issue` | re-read, place `spec-written`, mutate state | [`IssueBodyIsNotEmpty`] + `RecordSpecWritten` |
//! | `task_is_delivered` | re-read, place `waiting-merge` | [`AMergedPrClosesTheTask`] + `MarkWaitingMerge` |
//!
//! The actions live in `actions.rs`: that's the point of the split.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};
use harness_core::ports::shell::github::GitHub;

use crate::common::labels;
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::data::tasks;

/// The label says the technical sections are already written — by a
/// technical refinement or by a previous run: we resume after, we don't
/// re-charge a second write on top of the first.
pub struct TechAlreadyWritten;

#[async_trait(?Send)]
impl Verification<Loop> for TechAlreadyWritten {
    fn purpose(&self) -> String {
        "the issue does not already carry harness:tech-written — the technical sections are not paid for twice".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if !ctx.state.tech_written {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!(
            "issue #{} already carries {} — skipping the technical \
             refinement",
            ctx.state.task.number,
            labels::TECH_WRITTEN
        )))
    }
}

/// The technical stage builds on the business sections: without them it
/// would plan a guess.
///
/// Refuses rather than writing the business half itself: that half is the
/// human's to ask for (`harness:refinement`), and the loop never invents it.
pub struct TaskHasABusinessSpec;

#[async_trait(?Send)]
impl Verification<Loop> for TaskHasABusinessSpec {
    fn purpose(&self) -> String {
        "the issue body carries its business sections (Business Goal, Acceptance Criteria, Business Rules) before the technical ones are planned".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || ctx.state.spec_written {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "issue #{} does not carry {} — its business sections are not \
             written. Ask for them with `gh issue edit {} --add-label {}`",
            ctx.state.task.number,
            labels::SPEC_WRITTEN,
            ctx.state.task.number,
            labels::REFINEMENT
        )))
    }
}

/// The scope of the `code` stage: without an issue body, there's no SPEC.
pub struct CodeHasASpec;

#[async_trait(?Send)]
impl Verification<Loop> for CodeHasASpec {
    fn purpose(&self) -> String {
        "the issue body is not empty — there is a SPEC to build".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || !ctx.state.task.body.trim().is_empty() {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "issue #{} has an empty body — there is no SPEC to build from. Run \
             business refinement on it first (label {}).",
            ctx.state.task.number,
            labels::REFINEMENT
        )))
    }
}

/// Has this `/code`'s PR already merged? — we don't re-charge it.
///
/// **Only on a resumed round.** On a new round, `/code` never ran, and
/// searching for the PR would cost an API call per round for an answer
/// known in advance.
///
/// The same proof [`AMergedPrClosesTheTask`] requires, placed *before*
/// charging rather than after: it's the only guard that says "it's already
/// done" rather than "this won't work".
pub struct CodeAlreadyDelivered {
    /// What reads the issue and PRs.
    pub gh: Rc<dyn GitHub>,
    /// The branch on which proof is sought.
    pub integration_branch: String,
    /// `--restart` replays the stage even if proof is there.
    pub restart: bool,
}

#[async_trait(?Send)]
impl Verification<Loop> for CodeAlreadyDelivered {
    fn purpose(&self) -> String {
        "on a resumed round, no merged pull request already closes this task — /code is not paid for twice".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if !ctx.state.resumed || ctx.settings.dry_run || self.restart {
            return Ok(Verdict::Continue);
        }
        let number: u64 = ctx.state.task.number.parse().map_err(|_| {
            Halt::Failed(format!(
                "unreadable task number: {:?}",
                ctx.state.task.number
            ))
        })?;
        let here = self.gh.issue(number).await?;
        // `review-pending` counts: the write side's PR is open and a human
        // merges it; paying `/code` again would open a second one.
        let shipped = here.is_closed()
            || tasks::waiting_merge(&here)
            || tasks::review_pending(&here)
            || tasks::first_closing(&self.gh.merged_prs(&self.integration_branch).await?, number)
                .is_some();
        if !shipped {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!(
            "#{number} is already delivered on {} — /code skips rather than \
             being paid again (--restart to replay it)",
            self.integration_branch
        )))
    }
}

/// The issue body is not empty — the "judge" half of the old `spec_is_in_the_issue`.
///
/// Re-read from GitHub, not assumed: the issue body **is** the SPEC, and
/// it's what the next stage receives in scope.
pub struct IssueBodyIsNotEmpty {
    /// What re-reads the issue.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Verification<Loop> for IssueBodyIsNotEmpty {
    fn purpose(&self) -> String {
        "the issue body, re-read from GitHub after the stage, is not empty".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let number: u64 = ctx.state.task.number.parse().map_err(|_| {
            Halt::Failed(format!(
                "unreadable task number: {:?}",
                ctx.state.task.number
            ))
        })?;
        let issue = self.gh.issue(number).await?;
        if issue.body.trim().is_empty() {
            return Err(Halt::Halted(format!(
                "the technical refinement left issue #{number} with an empty \
                 body — the SPEC goes there, and /code reads nothing else"
            )));
        }
        Ok(Verdict::Continue)
    }
}

/// Does a merged PR carry `Closes #N` — the "judge" half of the old `task_is_delivered`.
///
/// What we require is not a stage's assertion, but a **merged** PR declaring it.
/// Without this check, `--stages technical-refinement` would infinitely re-charge the
/// same SPEC write.
pub struct AMergedPrClosesTheTask {
    /// What reads the issue and PRs.
    pub gh: Rc<dyn GitHub>,
    /// The branch on which proof is sought.
    pub integration_branch: String,
    /// True if `--stages` left `code` out — changes the message, not the rule.
    pub code_runs: bool,
    /// What `--stages` was, to say it in the message.
    pub stages: String,
}

#[async_trait(?Send)]
impl Verification<Loop> for AMergedPrClosesTheTask {
    fn purpose(&self) -> String {
        "a merged pull request on the integration branch carries `Closes #N` for this task"
            .to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || !ctx.state.has_task() {
            return Ok(Verdict::Continue);
        }
        let number: u64 = ctx.state.task.number.parse().map_err(|_| {
            Halt::Failed(format!(
                "unreadable task number: {:?}",
                ctx.state.task.number
            ))
        })?;
        // Already closed or marked: proof is there, nothing to reassert.
        let here = self.gh.issue(number).await?;
        if here.is_closed() || tasks::waiting_merge(&here) {
            return Ok(Verdict::Continue);
        }
        // The write side delivers an open PR, not a merged one: the round
        // marked it `review-pending`, and that is what it had to achieve.
        if ctx.state.write_side && tasks::review_pending(&here) {
            return Ok(Verdict::Continue);
        }
        let merged = self.gh.merged_prs(&self.integration_branch).await?;
        if tasks::first_closing(&merged, number).is_some() {
            return Ok(Verdict::Continue);
        }
        let left_out = if self.code_runs {
            String::new()
        } else {
            format!(
                " --stages ({}) left /code out, and nothing else delivers a \
                 task.",
                self.stages
            )
        };
        Err(Halt::Halted(format!(
            "issue #{number} ended the round and nothing marks it delivered — \
             no merged PR on {} carries `Closes #{number}` on its own \
             line.{left_out} Stopping rather than looping on the same task.",
            self.integration_branch
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use harness_core::domain::{Issue, Named};
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(state: Loop) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            state,
            Logbook::null(),
        )
    }

    fn with_task(number: &str, body: &str) -> Loop {
        Loop {
            task: Named {
                number: number.to_string(),
                title: "a task".to_string(),
                body: body.to_string(),
            },
            task_key: number.to_string(),
            ..Loop::default()
        }
    }

    fn issue(number: u64, labels: &[&str], body: &str) -> Issue {
        Issue {
            number,
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: body.to_string(),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn technical_sections_already_written_skip_rather_than_paying_twice() {
        let mut state = with_task("34", "the SPEC");
        state.tech_written = true;
        let verdict = TechAlreadyWritten
            .verify(&ctx(state))
            .await
            .expect("a verdict");
        assert!(matches!(verdict, Verdict::Skip(_)));
    }

    #[tokio::test]
    async fn without_the_label_the_technical_refinement_runs() {
        let verdict = TechAlreadyWritten
            .verify(&ctx(with_task("34", "")))
            .await
            .expect("a verdict");
        assert_eq!(verdict, Verdict::Continue);
    }

    #[tokio::test]
    async fn the_technical_stage_refuses_a_task_without_a_business_spec() {
        let err = TaskHasABusinessSpec
            .verify(&ctx(with_task("34", "raw request")))
            .await
            .expect_err("must stop");
        assert!(err.reason().contains(labels::REFINEMENT));
        let mut state = with_task("34", "raw request");
        state.spec_written = true;
        assert_eq!(
            TaskHasABusinessSpec
                .verify(&ctx(state))
                .await
                .expect("a verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn code_refuses_to_improvise_on_an_empty_spec() {
        let err = CodeHasASpec
            .verify(&ctx(with_task("34", "   ")))
            .await
            .expect_err("must stop");
        let Halt::Halted(said) = err else {
            panic!("a voluntary stop");
        };
        assert!(said.contains("no SPEC to build from"));
    }

    #[tokio::test]
    async fn a_dry_run_lets_code_through_without_a_spec() {
        let mut context = ctx(with_task("34", ""));
        context.settings.dry_run = true;
        assert_eq!(
            CodeHasASpec.verify(&context).await.expect("a verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_fresh_round_never_looks_for_a_merged_pr_of_its_own_code() {
        // One API call per round for an answer known in advance: /code never
        // ran on a fresh round.
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Unreadable("must not be called".to_string())),
            ..FakeGitHub::default()
        });
        let gate = CodeAlreadyDelivered {
            gh,
            integration_branch: "main_agent".to_string(),
            restart: false,
        };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "the SPEC")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_resumed_round_whose_code_already_merged_skips_it() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            merged: vec![Issue {
                number: 99,
                body: "Closes #34".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        let gate = CodeAlreadyDelivered {
            gh,
            integration_branch: "main_agent".to_string(),
            restart: false,
        };
        let mut state = with_task("34", "the SPEC");
        state.resumed = true;
        let Verdict::Skip(why) = gate.verify(&ctx(state)).await.expect("verdict") else {
            panic!("a skip");
        };
        assert!(why.contains("--restart"), "say how to replay");
    }

    #[tokio::test]
    async fn restart_replays_code_even_when_the_proof_is_there() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[labels::WAITING_MERGE], "")],
            ..FakeGitHub::default()
        });
        let gate = CodeAlreadyDelivered {
            gh,
            integration_branch: "main_agent".to_string(),
            restart: true,
        };
        let mut state = with_task("34", "the SPEC");
        state.resumed = true;
        assert_eq!(
            gate.verify(&ctx(state)).await.expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn the_spec_is_reread_from_github_not_assumed() {
        // The issue carries an empty body on GitHub even if local state says
        // otherwise: GitHub is right, it's what /code will read.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            ..FakeGitHub::default()
        });
        let gate = IssueBodyIsNotEmpty { gh };
        let err = gate
            .verify(&ctx(with_task("34", "misleading local body")))
            .await
            .expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
    }

    #[tokio::test]
    async fn a_non_empty_body_satisfies_the_verification_without_writing() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "the SPEC")],
            ..FakeGitHub::default()
        });
        let gate = IssueBodyIsNotEmpty { gh: gh.clone() };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
        // The "judge" half applies no label: the action will, and the borrow
        // checker enforces it.
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn a_merged_pr_that_closes_the_task_is_the_proof() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            merged: vec![Issue {
                number: 99,
                body: "Closes #34".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn nothing_marking_delivery_stops_rather_than_looping() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        let err = gate
            .verify(&ctx(with_task("34", "")))
            .await
            .expect_err("must stop");
        let Halt::Halted(said) = err else {
            panic!("a voluntary stop");
        };
        assert!(said.contains("Closes #34"));
        assert!(said.contains("Stopping rather than looping"));
    }

    #[tokio::test]
    async fn leaving_code_out_of_stages_is_named_in_the_message() {
        // Without this, "nothing marks delivery" reads as a bug when `--stages`
        // actually removed the only stage that delivers.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: false,
            stages: "technical-refinement".to_string(),
        };
        let err = gate
            .verify(&ctx(with_task("34", "")))
            .await
            .expect_err("must stop");
        assert!(format!("{err}").contains("left /code out"));
    }

    #[tokio::test]
    async fn an_already_marked_task_needs_no_further_proof() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[labels::WAITING_MERGE], "")],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
    }
}
