//! The whole planner run: what it establishes before paying, and the lock
//! it holds.
//!
//! Two refusals in `precheck`, before any session: the roadmap issue is
//! closed, or it doesn't carry `harness:roadmap`. A third case — the
//! roadmap item exists but isn't `harness:ready` yet — is **not** a
//! refusal: most roadmap items, most of the time, are still being reviewed,
//! and that is the normal state of the world, not a configuration error. It
//! is the clean "nothing to do" stop (`Ok(Some(message))`): traced, then
//! `Verdict::Continue`, the lock never touched. Unlike refinement, there is
//! no label this workflow removes on success — the router's own re-trigger
//! guard is the *absence* of a milestone under the roadmap item, not a label
//! flip here; `harness:ready` stays on the issue once planning is done.
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
use crate::planner::data::state::PlannerState;

/// A whole planner run: precheck, lock, the sequence, a summary.
pub struct PlannerRun {
    /// The tooling this workflow demands.
    pub pre: Gate<PlannerState>,
    /// How many rounds are left. One run plans one roadmap item.
    pub remaining: Cell<u32>,
    /// What reads the roadmap issue and writes the milestones it opens.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// This roadmap item's lock directory.
    pub planner_dir: PathBuf,
    /// The roadmap issue to plan.
    pub roadmap: u64,
    /// `roadmap`, as text — it is the name the lock carries.
    pub roadmap_key: String,
    /// The round, already mounted: the map, then the table's steps.
    pub round: Round<PlannerState>,
}

#[async_trait(?Send)]
impl Workflow<PlannerState> for PlannerRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<PlannerState>) -> Outcome<Option<String>> {
        let issue = self.gh.issue(self.roadmap).await?;

        if issue.is_closed() {
            return Err(Halt::Halted(format!(
                "#{} is closed — reopen it, or plan another roadmap item",
                self.roadmap
            )));
        }
        if !issue.has(labels::ROADMAP) {
            return Err(Halt::Halted(format!(
                "#{} does not carry {} — the planner works a roadmap item",
                self.roadmap,
                labels::ROADMAP
            )));
        }
        if !issue.has(labels::READY) {
            return Ok(Some(format!(
                "#{} is not {} yet — nothing to plan",
                self.roadmap,
                labels::READY
            )));
        }

        ctx.traces
            .say(&format!("planning #{} — {}", issue.number, issue.title));
        ctx.state.roadmap = Some(issue);
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.planner_dir,
            name: &self.roadmap_key,
        })
    }

    fn held(&self, _ctx: &Context<PlannerState>) -> String {
        format!(
            "skip — a planning run of #{} is already running",
            self.roadmap
        )
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<PlannerState> + '_> {
        // Borrowed, not rebuilt: there is only ever one round per run.
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<PlannerState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing written".to_string());
        }
        Some(format!("planned #{}", self.roadmap))
    }
}

#[async_trait(?Send)]
impl Executable<PlannerState> for PlannerRun {
    fn pre(&self) -> Option<&Gate<PlannerState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<PlannerState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    //! The three precheck outcomes, against a fake GitHub. The round
    //! is mounted by `run::build` — a test may go back to assembly,
    //! production code may not.

    use super::*;
    use crate::common::explore;
    use crate::common::fake_github::FakeGitHub;
    use crate::planner::config::fake as config_fake;
    use crate::planner::ports::fake as ports_fake;
    use crate::planner::run;
    use harness_core::domain::Issue;
    use harness_core::execution::{Guarded, Settings};
    use harness_core::traces::Logbook;

    fn dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("harness-planner-run-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn built(gh: &Rc<FakeGitHub>, planner_dir: PathBuf) -> PlannerRun {
        let config = config_fake::in_dir(planner_dir);
        let explore_config = explore::fake::config(config.artifacts_dir.clone());
        run::build(
            &ports_fake::with(Rc::clone(gh)),
            &config,
            &explore::fake::ports(),
            &explore_config,
            &run::Request { roadmap: 4 },
            Gate::empty("outillage"),
        )
    }

    fn ctx(dry_run: bool) -> Context<PlannerState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            PlannerState::default(),
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
    async fn a_closed_roadmap_issue_halts_before_any_session() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "closed", &[labels::ROADMAP])],
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
    async fn an_issue_without_the_roadmap_label_is_refused() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "open", &[labels::MILESTONE])],
            ..FakeGitHub::default()
        });
        let round = built(&gh, dir("not-roadmap"));
        let mut context = ctx(true);
        let err = Workflow::execute(&round, &mut context)
            .await
            .expect_err("must stop");
        assert!(err.reason().contains(labels::ROADMAP));
    }

    #[tokio::test]
    async fn a_roadmap_item_not_ready_yet_is_a_clean_skip_not_a_failure() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "open", &[labels::ROADMAP])],
            ..FakeGitHub::default()
        });
        let round = built(&gh, dir("not-ready"));
        let mut context = ctx(true);
        let verdict = Guarded::execute(&round, &mut context)
            .await
            .expect("a clean skip, not an error");
        assert_eq!(verdict, Verdict::Continue);
        assert!(context.state.roadmap.is_none(), "nothing picked");
    }
}
