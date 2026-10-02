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
use harness_core::adapters::shell::github::GitHub;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::common::labels;
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::data::{board, tasks};

/// The label says the SPEC is already written: we resume after, we don't
/// re-charge a second write on top of the first.
pub struct SpecAlreadyWritten;

#[async_trait(?Send)]
impl Verification<Loop> for SpecAlreadyWritten {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if !ctx.state.spec_written {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!(
            "issue #{} already carries {} — skipping /business-analyst \
             (resuming a previous run)",
            ctx.state.task.number,
            labels::SPEC_WRITTEN
        )))
    }
}

/// The scope of the `code` stage: without an issue body, there's no SPEC.
pub struct CodeHasASpec;

#[async_trait(?Send)]
impl Verification<Loop> for CodeHasASpec {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || !ctx.state.task.body.trim().is_empty() {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "issue #{} has an empty body — there is no SPEC to build from. Run \
             /business-analyst on it first (--stages business-analyst).",
            ctx.state.task.number
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
        let shipped = here.is_closed()
            || tasks::waiting_merge(&here)
            || tasks::first_closing(&self.gh.merged_prs(&self.integration_branch).await?, number)
                .is_some();
        if !shipped {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!(
            "#{number} est déjà livrée sur {} — /code saute plutôt que d'être \
             repayé (--restart pour le rejouer)",
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
                "/business-analyst left issue #{number} with an empty body — \
                 the SPEC goes there, and /code reads nothing else"
            )));
        }
        Ok(Verdict::Continue)
    }
}

/// Does a merged PR carry `Closes #N` — the "judge" half of the old `task_is_delivered`.
///
/// What we require is not a stage's assertion, but a **merged** PR declaring it.
/// Without this check, `--stages business-analyst` would infinitely re-charge the
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

/// Rollover opened something to work on, in the next milestone.
///
/// Re-read from scratch: the current milestone may not be the same anymore.
/// It will be only if `/planner` closed the one it just finished — the loop
/// works on the lowest-numbered open milestone, so an old one left open would
/// roll all following rounds empty.
pub struct PlannerOpenedATask {
    /// What re-reads the board.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Verification<Loop> for PlannerOpenedATask {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let after = board::read(self.gh.as_ref()).await?;
        if after.open_agents().is_empty() {
            return Err(Halt::Halted(format!(
                "/planner left nothing to pick up: milestone {} still has no \
                 open {} issue. Either it opened none, or it did not close \
                 milestone #{} — the harness reads the lowest-numbered open \
                 milestone, and that one is still it.",
                after.milestone.reference(),
                labels::AGENT,
                ctx.state.milestone.number
            )));
        }
        Ok(Verdict::Continue)
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
    async fn a_spec_already_written_skips_rather_than_paying_twice() {
        let mut state = with_task("34", "the SPEC");
        state.spec_written = true;
        let verdict = SpecAlreadyWritten
            .verify(&ctx(state))
            .await
            .expect("a verdict");
        assert!(matches!(verdict, Verdict::Skip(_)));
    }

    #[tokio::test]
    async fn without_the_label_business_analyst_runs() {
        let verdict = SpecAlreadyWritten
            .verify(&ctx(with_task("34", "")))
            .await
            .expect("a verdict");
        assert_eq!(verdict, Verdict::Continue);
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
        assert!(gh.writes().is_empty());
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
            stages: "business-analyst".to_string(),
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

    #[tokio::test]
    async fn a_planner_that_opened_nothing_stops_and_says_which_of_two_causes() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE], "")],
            subs: vec![(4, vec![])],
            ..FakeGitHub::default()
        });
        let gate = PlannerOpenedATask { gh };
        let mut state = Loop::default();
        state.milestone.number = "4".to_string();
        let err = gate.verify(&ctx(state)).await.expect_err("must stop");
        let said = format!("{err}");
        assert!(said.contains("opened none"));
        assert!(said.contains("did not close milestone"));
    }
}
