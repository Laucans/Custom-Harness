//! The whole split run: what it establishes before paying, and the lock it
//! holds.
//!
//! Two refusals in `precheck`, before any session: the milestone issue is
//! closed, or it doesn't carry `harness:milestone`. A third case — the
//! milestone exists but isn't `harness:ready` yet — is **not** a refusal:
//! most milestones, most of the time, simply aren't ready, and that is the
//! normal state of the world, not a configuration error. It is the clean
//! "nothing to do" stop (`Ok(Some(message))`): traced, then
//! `Verdict::Continue`, the lock never touched.
//!
//! One round per run, so `remaining` starts at 1. Everything around it —
//! precheck, lock, the round, the summary — is the core's [`Workflow`].

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Executable, Gate, Lock, Round, Workflow};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;

use crate::common::labels;
use crate::split::data::state::SplitState;

/// A whole split run: precheck, lock, the sequence, a summary.
pub struct SplitRun {
    /// The tooling this workflow demands.
    pub pre: Gate<SplitState>,
    /// How many rounds are left. One run splits one milestone.
    pub remaining: Cell<u32>,
    /// What reads the milestone issue and writes the tasks it opens.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// This milestone's lock directory.
    pub split_dir: PathBuf,
    /// The milestone issue to split.
    pub milestone: u64,
    /// `milestone`, as text — it is the name the lock carries.
    pub milestone_key: String,
    /// The round, already mounted: the table's steps.
    pub round: Round<SplitState>,
}

#[async_trait(?Send)]
impl Workflow<SplitState> for SplitRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<SplitState>) -> Outcome<Option<String>> {
        let issue = self.gh.issue(self.milestone).await?;

        if issue.is_closed() {
            return Err(Halt::Halted(format!(
                "#{} is closed — reopen it, or split another milestone",
                self.milestone
            )));
        }
        if !issue.has(labels::MILESTONE) {
            return Err(Halt::Halted(format!(
                "#{} does not carry {} — split works a milestone",
                self.milestone,
                labels::MILESTONE
            )));
        }
        if !issue.has(labels::READY) {
            return Ok(Some(format!(
                "#{} is not {} yet — nothing to split",
                self.milestone,
                labels::READY
            )));
        }

        ctx.traces
            .say(&format!("splitting #{} — {}", issue.number, issue.title));
        ctx.state.milestone = Some(issue);
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.split_dir,
            name: &self.milestone_key,
        })
    }

    fn held(&self, _ctx: &Context<SplitState>) -> String {
        format!(
            "skip — a split run of #{} is already running",
            self.milestone
        )
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<SplitState> + '_> {
        // Borrowed, not rebuilt: there is only ever one round per run.
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<SplitState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing written".to_string());
        }
        Some(format!("split #{}", self.milestone))
    }
}

#[async_trait(?Send)]
impl Executable<SplitState> for SplitRun {
    fn pre(&self) -> Option<&Gate<SplitState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<SplitState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    //! The three precheck outcomes, against a fake GitHub. The round is
    //! mounted by `run::build` — a test may go back to assembly, production
    //! code may not.

    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use crate::split::config::fake as config_fake;
    use crate::split::ports::fake as ports_fake;
    use crate::split::run;
    use harness_core::domain::{Issue, Verdict};
    use harness_core::execution::{Guarded, Settings};
    use harness_core::traces::Logbook;

    fn dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("harness-split-run-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn built(gh: &Rc<FakeGitHub>, split_dir: PathBuf) -> SplitRun {
        run::build(
            &ports_fake::with(Rc::clone(gh)),
            &config_fake::in_dir(split_dir),
            &run::Request { milestone: 4 },
            Gate::empty("tooling"),
        )
    }

    fn ctx(dry_run: bool) -> Context<SplitState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            SplitState::default(),
            Logbook::null(),
        )
    }

    fn issue(number: u64, state: &str, labels: &[&str]) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_closed_milestone_halts_before_any_session() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "closed", &[labels::MILESTONE, labels::READY])],
            ..FakeGitHub::default()
        });
        let round = built(&gh, dir("closed"));
        let mut context = ctx(true);
        let err = Workflow::execute(&round, &mut context)
            .await
            .expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(err.reason().contains("is closed"));
    }

    #[tokio::test]
    async fn an_issue_without_the_milestone_label_is_refused() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "open", &[labels::ROADMAP, labels::READY])],
            ..FakeGitHub::default()
        });
        let round = built(&gh, dir("not-milestone"));
        let mut context = ctx(true);
        let err = Workflow::execute(&round, &mut context)
            .await
            .expect_err("must stop");
        assert!(err.reason().contains(labels::MILESTONE));
    }

    #[tokio::test]
    async fn a_milestone_not_ready_yet_is_a_clean_skip_not_a_failure() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "open", &[labels::MILESTONE])],
            ..FakeGitHub::default()
        });
        let round = built(&gh, dir("not-ready"));
        let mut context = ctx(true);
        let verdict = Guarded::execute(&round, &mut context)
            .await
            .expect("a clean skip, not an error");
        assert_eq!(verdict, Verdict::Continue);
        assert!(context.state.milestone.is_none(), "nothing picked");
    }
}
