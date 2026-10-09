//! The whole repair run: what it establishes before paying, and the lock it
//! holds.
//!
//! Two refusals in `precheck`, before any session: the PR cannot be read, or
//! it is no longer open. A third case — the PR is open but carries no
//! `harness:pr-fix` — is **not** a refusal: most PRs, most of the time,
//! nobody has asked anything about, and that is the normal state of the
//! world rather than a misconfiguration. It is the clean "nothing to do"
//! stop (`Ok(Some(message))`): traced, then `Verdict::Continue`, the lock
//! never touched.
//!
//! Whether anything is actually red is **not** decided here: that read
//! belongs to the free stage, where its result also decides whether the
//! request gets consumed. A precheck that read it too would read it twice.
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
use crate::pr_fix::data::state::FixState;

/// A whole repair attempt: precheck, lock, the sequence, a summary.
pub struct FixRun {
    /// The tooling this workflow demands.
    pub pre: Gate<FixState>,
    /// How many rounds are left. One run is one attempt.
    pub remaining: Cell<u32>,
    /// What reads the PR and removes the label.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// This PR's lock directory.
    pub fix_dir: PathBuf,
    /// The PR to repair, number or URL.
    pub pr_ref: String,
    /// The round, already mounted: the table's steps.
    pub round: Round<FixState>,
}

#[async_trait(?Send)]
impl Workflow<FixState> for FixRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<FixState>) -> Outcome<Option<String>> {
        let pr = self.gh.pr(&self.pr_ref).await?;
        if !pr.is_open() {
            return Err(Halt::Halted(format!(
                "PR {} is {} — a repair works an open PR",
                pr.reference(),
                pr.state
            )));
        }
        if !pr.has(labels::PR_FIX) {
            return Ok(Some(format!(
                "{} does not carry {} — nobody asked for a repair",
                pr.reference(),
                labels::PR_FIX
            )));
        }
        ctx.traces.say(&format!(
            "repairing {}  {} -> {}  ({})",
            pr.reference(),
            pr.head,
            pr.base,
            pr.title
        ));
        ctx.state.pr = Some(pr);
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.fix_dir,
            name: &self.pr_ref,
        })
    }

    fn held(&self, _ctx: &Context<FixState>) -> String {
        format!("skip — a repair of PR {} is already running", self.pr_ref)
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<FixState> + '_> {
        // Borrowed, not rebuilt: there is only ever one round per run.
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<FixState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing pushed".to_string());
        }
        if ctx.state.failing.is_empty() {
            return Some(format!("nothing to repair on PR {}", self.pr_ref));
        }
        // Deliberately not "repaired": whether CI went green is read on a
        // later poll, never claimed here.
        Some(format!("one repair attempted on PR {}", self.pr_ref))
    }
}

#[async_trait(?Send)]
impl Executable<FixState> for FixRun {
    fn pre(&self) -> Option<&Gate<FixState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<FixState>) -> Outcome<Verdict> {
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
    use crate::pr_fix::config::fake as config_fake;
    use crate::pr_fix::ports::fake as ports_fake;
    use crate::pr_fix::run;
    use harness_core::domain::Pr;
    use harness_core::execution::{Guarded, Settings};
    use harness_core::traces::Logbook;

    fn dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("harness-pr-fix-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn built(gh: &Rc<FakeGitHub>, fix_dir: PathBuf) -> FixRun {
        run::build(
            &ports_fake::with(Rc::clone(gh)),
            &config_fake::in_dir(fix_dir),
            &run::Request {
                pr_ref: "32".to_string(),
            },
            Gate::empty("tooling"),
        )
    }

    fn ctx(dry_run: bool) -> Context<FixState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            FixState::default(),
            Logbook::null(),
        )
    }

    fn pr(state: &str, labels: &[&str]) -> Pr {
        Pr {
            num: "32".to_string(),
            base: "milestone/4-territory".to_string(),
            head: "feat/cities".to_string(),
            title: "feat(db): cities".to_string(),
            url: String::new(),
            state: state.to_string(),
            draft: false,
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
        }
    }

    #[tokio::test]
    async fn a_merged_pr_halts_before_any_session() {
        let gh = Rc::new(FakeGitHub {
            prs: vec![("32".to_string(), pr("MERGED", &[labels::PR_FIX]))],
            ..FakeGitHub::default()
        });
        let run = built(&gh, dir("merged"));
        let mut context = ctx(true);
        let err = Workflow::execute(&run, &mut context)
            .await
            .expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(err.reason().contains("MERGED"));
    }

    #[tokio::test]
    async fn a_pr_nobody_asked_about_is_a_clean_skip_not_a_failure() {
        let gh = Rc::new(FakeGitHub {
            prs: vec![("32".to_string(), pr("OPEN", &[]))],
            ..FakeGitHub::default()
        });
        let run = built(&gh, dir("unasked"));
        let mut context = ctx(true);
        let verdict = Guarded::execute(&run, &mut context)
            .await
            .expect("a clean skip, not an error");
        assert_eq!(verdict, Verdict::Continue);
        assert!(context.state.pr.is_none(), "nothing picked");
        assert!(gh.writes().is_empty(), "no label touched");
    }

    #[tokio::test]
    async fn the_summary_never_claims_the_repair_worked() {
        // Only CI can say that, on a later poll.
        let gh = Rc::new(FakeGitHub::default());
        let run = built(&gh, dir("summary"));
        let mut context = ctx(false);
        context.state.pr = Some(pr("OPEN", &[labels::PR_FIX]));
        context.state.failing = vec!["ci — FAILURE".to_string()];
        let summary = Workflow::summary(&run, &context).expect("a summary");
        assert!(summary.contains("attempted"), "{summary}");
        assert!(!summary.contains("repaired"), "{summary}");
    }
}
