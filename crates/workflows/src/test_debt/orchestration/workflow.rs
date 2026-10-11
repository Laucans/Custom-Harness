//! The whole test-debt run: what it establishes before paying, and the lock
//! it holds.
//!
//! Precheck reads the issue. An issue closed since the router saw it, or one
//! no longer carrying `harness:test-debt`, is **not** a refusal: another
//! attempt took it, or a human settled it — the clean "nothing to do" stop
//! (`Ok(Some(message))`), the lock never touched. Only an issue that cannot
//! be read at all fails.
//!
//! Whether the title names a file is **not** decided here: that read
//! belongs to the free stage, where its result also decides whether the
//! request is consumed.
//!
//! One round per run, so `remaining` starts at 1. Everything around it —
//! precheck, lock, the round, the summary — is the core's [`Workflow`].

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Executable, Gate, Lock, Round, Workflow};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;

use crate::common::labels;
use crate::test_debt::data::state::DebtState;

/// A whole test-debt attempt: precheck, lock, the sequence, a summary.
pub struct DebtRun {
    /// The tooling this workflow demands.
    pub pre: Gate<DebtState>,
    /// How many rounds are left. One run is one attempt.
    pub remaining: Cell<u32>,
    /// What reads the issue.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// The issues' lock directory.
    pub debt_dir: PathBuf,
    /// The test-debt issue's number.
    pub issue: u64,
    /// The issue's number as the lock names it.
    pub lock_name: String,
    /// The round, already mounted: the table's steps.
    pub round: Round<DebtState>,
}

#[async_trait(?Send)]
impl Workflow<DebtState> for DebtRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<DebtState>) -> Outcome<Option<String>> {
        let issue = self.gh.issue(self.issue).await?;
        if !issue.is_open() {
            return Ok(Some(format!(
                "{} is closed — the debt is settled",
                issue.reference()
            )));
        }
        if !issue.has(labels::TEST_DEBT) {
            return Ok(Some(format!(
                "{} does not carry {} — nobody asks for its tests now",
                issue.reference(),
                labels::TEST_DEBT
            )));
        }
        ctx.traces
            .say(&format!("test debt {}  {}", issue.reference(), issue.title));
        ctx.state.issue = Some(issue);
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.debt_dir,
            name: &self.lock_name,
        })
    }

    fn held(&self, _ctx: &Context<DebtState>) -> String {
        format!(
            "skip — an attempt at issue #{}'s test debt is already running",
            self.issue
        )
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<DebtState> + '_> {
        // Borrowed, not rebuilt: there is only ever one round per run.
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<DebtState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing pushed".to_string());
        }
        let Some(file) = &ctx.state.file else {
            return Some(format!(
                "issue #{} names no file — nothing asked",
                self.issue
            ));
        };
        // Deliberately not "covered": whether the mutants are caught is read
        // by the next rotation that judges the file, never claimed here.
        Some(format!(
            "one attempt at the tests of {file} (issue #{})",
            self.issue
        ))
    }
}

#[async_trait(?Send)]
impl Executable<DebtState> for DebtRun {
    fn pre(&self) -> Option<&Gate<DebtState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<DebtState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    //! The precheck outcomes, against a fake GitHub. The round is mounted by
    //! `run::build` — a test may go back to assembly, production code may not.

    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use crate::test_debt::config::fake as config_fake;
    use crate::test_debt::ports::fake as ports_fake;
    use crate::test_debt::run;
    use harness_core::domain::{Halt, Issue};
    use harness_core::execution::{Guarded, Settings};
    use harness_core::traces::Logbook;

    fn dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("harness-test-debt-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn built(gh: &Rc<FakeGitHub>, debt_dir: PathBuf) -> DebtRun {
        run::build(
            &ports_fake::with(Rc::clone(gh)),
            &config_fake::in_dir(debt_dir),
            &run::Request { issue: 7 },
            Gate::empty("tooling"),
        )
    }

    fn ctx(dry_run: bool) -> Context<DebtState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            DebtState::default(),
            Logbook::null(),
        )
    }

    fn issue(state: &str, labels: &[&str]) -> Issue {
        Issue {
            number: 7,
            title: "Test debt: crates/a/src/lib.rs".to_string(),
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: "- line 3: replace f with true".to_string(),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_closed_or_unlabelled_issue_is_a_clean_skip_not_a_failure() {
        for (state, labelled) in [("closed", &[labels::TEST_DEBT][..]), ("open", &[])] {
            let gh = Rc::new(FakeGitHub {
                issues: vec![issue(state, labelled)],
                ..FakeGitHub::default()
            });
            let run = built(&gh, dir(&format!("skip-{state}")));
            let mut context = ctx(false);
            let verdict = Guarded::execute(&run, &mut context)
                .await
                .expect("a clean skip, not an error");
            assert_eq!(verdict, Verdict::Continue, "{state}");
            assert!(context.state.issue.is_none(), "nothing picked");
            assert!(gh.writes().is_empty(), "no label touched");
        }
    }

    #[tokio::test]
    async fn an_unreadable_issue_fails() {
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Failed("no credentials".to_string())),
            ..FakeGitHub::default()
        });
        let run = built(&gh, dir("unreadable"));
        let mut context = ctx(false);
        let err = Workflow::execute(&run, &mut context)
            .await
            .expect_err("must fail");
        assert!(err.reason().contains("no credentials"));
    }

    #[tokio::test]
    async fn a_dry_run_reads_the_debt_and_writes_nothing() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue("open", &[labels::TEST_DEBT])],
            ..FakeGitHub::default()
        });
        let run = built(&gh, dir("dry"));
        let mut context = ctx(true);
        Workflow::execute(&run, &mut context)
            .await
            .expect("a rehearsal");
        assert_eq!(context.state.file.as_deref(), Some("crates/a/src/lib.rs"));
        assert!(gh.writes().is_empty(), "a dry run keeps the label");
        assert_eq!(
            Workflow::summary(&run, &context).as_deref(),
            Some("dry run — nothing pushed")
        );
    }

    #[test]
    fn one_attempt_per_issue_holds_its_lock_and_its_tooling_gate() {
        let gh = Rc::new(FakeGitHub::default());
        let debt_dir = dir("lock");
        let run = built(&gh, debt_dir.clone());
        let lock = Workflow::lock(&run).expect("an attempt is locked per issue");
        assert_eq!(lock.dir, debt_dir.as_path());
        assert_eq!(lock.name, "7");
        let held = Workflow::held(&run, &ctx(false));
        assert!(held.starts_with("skip") && held.contains("#7"), "{held}");
        let pre = Executable::pre(&run).expect("the tooling gate is mounted");
        assert_eq!(pre.name, "tooling");
        assert!(std::ptr::eq(
            Workflow::remaining(&run),
            std::ptr::from_ref(&run.remaining)
        ));
    }

    #[tokio::test]
    async fn the_summary_never_claims_the_mutants_are_caught() {
        let gh = Rc::new(FakeGitHub::default());
        let run = built(&gh, dir("summary"));
        let mut context = ctx(false);
        context.state.file = Some("crates/a/src/lib.rs".to_string());
        let summary = Workflow::summary(&run, &context).expect("a summary");
        assert!(summary.contains("one attempt"), "{summary}");
        assert!(!summary.contains("caught"), "{summary}");
        context.state.file = None;
        let summary = Workflow::summary(&run, &context).expect("a summary");
        assert!(summary.contains("names no file"), "{summary}");
    }
}
