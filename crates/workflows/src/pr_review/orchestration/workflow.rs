//! The whole review: what it establishes before paying, and the lock it
//! holds.
//!
//! **A skipped review is a success**, not a shortfall: a draft PR, or one
//! already reviewed, has nothing to obtain.
//!
//! One round, so `remaining` starts at 1. That is the only thing that
//! separates this workflow from the dev loop's shape — everything else
//! (precheck, lock, the round, the summary) is the core's
//! [`Workflow`].
//!
//! The shape alone — neither `Ports`, nor `Config`, nor `Request` are named
//! here. What turns them into a mounted review lives in
//! [`run::build`](crate::pr_review::run::build).

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Executable, Gate, Lock, Round, Workflow};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;

use crate::pr_review::data::skip_rules;
use crate::pr_review::data::state::ReviewState;

/// A whole review: precheck, lock, the two passes, the publication.
pub struct ReviewRun {
    /// The tooling this review demands — nothing more.
    pub pre: Gate<ReviewState>,
    /// How many rounds are left. One review is one round.
    pub remaining: Cell<u32>,
    /// The PR board.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// The directory the locks live under.
    pub review_dir: PathBuf,
    /// The PR to review, number or URL.
    pub pr_ref: String,
    /// The branch a PR must target to be reviewed.
    pub base: String,
    /// Review even if a rule would say to skip.
    pub force: bool,
    /// The round, already mounted.
    pub round: Round<ReviewState>,
}

#[async_trait(?Send)]
impl Workflow<ReviewState> for ReviewRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<ReviewState>) -> Outcome<Option<String>> {
        let pr = self.gh.pr(&self.pr_ref).await?;
        let comments = if self.force {
            String::new()
        } else {
            self.gh.pr_comments(&pr.num).await?
        };
        if let Some(skip) = skip_rules::skip_reason(self.force, &self.base, &pr, &comments) {
            return Ok(Some(format!("skip — {skip}")));
        }
        ctx.traces.say(&format!(
            "reviewing #{}  {} -> {}  ({})",
            pr.num, pr.head, pr.base, pr.title
        ));
        ctx.state.pr = Some(pr);
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.review_dir,
            name: &self.pr_ref,
        })
    }

    fn held(&self, ctx: &Context<ReviewState>) -> String {
        let num = ctx
            .state
            .pr
            .as_ref()
            .map_or(self.pr_ref.as_str(), |pr| &pr.num);
        format!("skip — a review of PR #{num} is already running")
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<ReviewState> + '_> {
        // Borrowed, not rebuilt: nothing in this round varies by turn, and
        // there is only ever one.
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<ReviewState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing posted".to_string());
        }
        Some(format!("reviewed #{}", ctx.state.pr().num))
    }
}

#[async_trait(?Send)]
impl Executable<ReviewState> for ReviewRun {
    fn pre(&self) -> Option<&Gate<ReviewState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<ReviewState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    //! The precheck, against a fake GitHub. The review is mounted by
    //! `run::build` — a test may climb back up to the assembly, production
    //! code may not.

    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use crate::pr_review::config::fake as config_fake;
    use crate::pr_review::ports::fake as ports_fake;
    use crate::pr_review::run;
    use harness_core::domain::{Halt, Pr};
    use harness_core::execution::{Guarded, Settings};
    use harness_core::traces::Logbook;

    fn ctx() -> Context<ReviewState> {
        Context::new(
            Settings {
                dry_run: true,
                stages: String::new(),
            },
            ReviewState::default(),
            Logbook::null(),
        )
    }

    fn pr(num: &str, base: &str, draft: bool) -> Pr {
        Pr {
            num: num.to_string(),
            base: base.to_string(),
            head: "feat/x".to_string(),
            title: "a batch".to_string(),
            url: format!("https://github.com/o/r/pull/{num}"),
            state: "OPEN".to_string(),
            draft,
            labels: Vec::new(),
        }
    }

    fn review(gh: &Rc<FakeGitHub>, pr_ref: &str) -> ReviewRun {
        run::build(
            &ports_fake::with(Rc::clone(gh)),
            &config_fake::config(),
            run::Request {
                pr_ref: pr_ref.to_string(),
                base: "main_agent".to_string(),
                force: false,
            },
            Gate::empty("tooling"),
        )
    }

    #[tokio::test]
    async fn a_draft_pr_is_skipped_as_a_success_not_an_error() {
        let gh = Rc::new(FakeGitHub {
            prs: vec![("32".to_string(), pr("32", "main_agent", true))],
            ..FakeGitHub::default()
        });
        let built = review(&gh, "32");
        let mut context = ctx();
        // `Guarded::execute` named: that is the path the launcher takes, and
        // `Workflow::execute` carries the same name without the tooling gate.
        Guarded::execute(&built, &mut context)
            .await
            .expect("a success, not an error");
        assert!(context.state.pr.is_none(), "the precheck set nothing");
        assert_eq!(built.remaining.get(), 1, "no round was started");
    }

    #[tokio::test]
    async fn an_unreadable_pr_is_a_real_failure() {
        let gh = Rc::new(FakeGitHub::default());
        let built = review(&gh, "999");
        let mut context = ctx();
        let err = Guarded::execute(&built, &mut context)
            .await
            .expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }
}
