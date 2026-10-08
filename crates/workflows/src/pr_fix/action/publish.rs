//! The one thing a repair writes on GitHub: the label it was asked by.
//!
//! **Removed before the session is paid for, not after it succeeds.** The
//! alternative was tried on paper and rejected: a PR whose CI stays red —
//! a broken dependency, an environment the agent cannot reach, a test that
//! needs a human decision — would then buy one opus session every poll,
//! forever, with nobody watching. One label is one attempt, and the human
//! re-poses it if the attempt was worth repeating.
//!
//! The cost of that choice, stated plainly: a session that dies on a
//! transient error has spent the request. That is recoverable by re-posing
//! a label; an unbounded spend is not.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;

use crate::common::labels;
use crate::pr_fix::data::state::FixState;

/// Removes `harness:pr-fix` from the PR: the request has been taken.
pub struct ConsumeRequest {
    /// What removes the label.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<FixState> for ConsumeRequest {
    async fn run(&self, ctx: &mut Context<FixState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        // Nothing broke and the review asks for nothing, so nothing was asked
        // of anyone: leaving the label in place lets the poll that finds a
        // real failure use it.
        if ctx.state.failing.is_empty() && !ctx.state.review_blocking {
            return Ok(Verdict::Continue);
        }
        let pr = ctx.state.pr();
        let number = pr.num.parse::<u64>().map_err(|_| {
            Halt::Failed(format!(
                "{:?} is not a PR number — a label is posed by number, and \
                 this came from a PR listing that should only ever yield one",
                pr.num
            ))
        })?;
        self.gh.remove_label(number, labels::PR_FIX).await?;
        ctx.traces.say(&format!(
            "#{number}: {} consumed — one attempt follows",
            labels::PR_FIX
        ));
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Pr;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool, num: &str, failing: &[&str]) -> Context<FixState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            FixState {
                pr: Some(Pr {
                    num: num.to_string(),
                    ..Pr::default()
                }),
                failing: failing.iter().map(|f| (*f).to_string()).collect(),
                comments: String::new(),
                review_blocking: false,
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn the_label_is_removed_once_something_is_actually_broken() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, "32", &["ci — FAILURE"]);
        ConsumeRequest { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("consumed");
        assert_eq!(
            gh.writes(),
            vec![Wrote::Unlabelled(32, labels::PR_FIX.to_string())]
        );
    }

    #[tokio::test]
    async fn a_pr_with_nothing_broken_keeps_its_label_for_a_later_poll() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, "32", &[]);
        ConsumeRequest { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("nothing to consume");
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(true, "32", &["ci — FAILURE"]);
        ConsumeRequest { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("nothing to do");
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn a_pr_reference_that_is_not_a_number_fails_rather_than_labelling_something_else() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, "https://github.com/o/r/pull/32", &["ci — FAILURE"]);
        let err = ConsumeRequest { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }
}
