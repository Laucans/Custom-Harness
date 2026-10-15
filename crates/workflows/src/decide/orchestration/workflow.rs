//! The whole decision: what it establishes before paying, and the lock it
//! holds.
//!
//! Precheck reads the issue. One closed since the router saw it, or no
//! longer asking (`harness:needs-decision` off: a human decided, or another
//! run did), is the clean "nothing to do" stop. Only an issue that cannot
//! be read at all fails.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Executable, Gate, Lock, Round, Workflow};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;

use crate::common::labels;
use crate::decide::data::state::DecideState;

/// A whole decision: precheck, lock, the sequence, a summary.
pub struct DecideRun {
    /// The tooling this workflow demands.
    pub pre: Gate<DecideState>,
    /// How many rounds are left. One run is one decision.
    pub remaining: Cell<u32>,
    /// What reads the issue.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// The issues' lock directory.
    pub decide_dir: PathBuf,
    /// The issue to decide on.
    pub issue: u64,
    /// The issue's number as the lock names it.
    pub lock_name: String,
    /// The round, already mounted.
    pub round: Round<DecideState>,
}

#[async_trait(?Send)]
impl Workflow<DecideState> for DecideRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<DecideState>) -> Outcome<Option<String>> {
        let issue = self.gh.issue(self.issue).await?;
        if !issue.is_open() {
            return Ok(Some(format!(
                "{} is closed — nothing to decide",
                issue.reference()
            )));
        }
        if !issue.has(labels::NEEDS_DECISION) {
            return Ok(Some(format!(
                "{} does not carry {} — nobody asks for a decision now",
                issue.reference(),
                labels::NEEDS_DECISION
            )));
        }
        ctx.traces
            .say(&format!("decide {}  {}", issue.reference(), issue.title));
        ctx.state.issue = Some(issue);
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.decide_dir,
            name: &self.lock_name,
        })
    }

    fn held(&self, _ctx: &Context<DecideState>) -> String {
        format!(
            "skip — a decision on issue #{} is already being taken",
            self.issue
        )
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<DecideState> + '_> {
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<DecideState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing written".to_string());
        }
        Some(if ctx.state.owner_decided {
            format!(
                "issue #{}: the owner had decided — nothing paid",
                self.issue
            )
        } else {
            format!("issue #{}: decided ({})", self.issue, ctx.state.kind)
        })
    }
}

#[async_trait(?Send)]
impl Executable<DecideState> for DecideRun {
    fn pre(&self) -> Option<&Gate<DecideState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<DecideState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use crate::decide::config::fake as config_fake;
    use crate::decide::ports::fake as ports_fake;
    use crate::decide::run;
    use harness_core::domain::{Halt, Issue};
    use harness_core::execution::{Guarded, Settings};
    use harness_core::traces::Logbook;

    fn dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("harness-decide-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn built(gh: &Rc<FakeGitHub>, decide_dir: PathBuf) -> DecideRun {
        run::build(
            &ports_fake::with(Rc::clone(gh)),
            &config_fake::in_dir(decide_dir),
            &run::Request { issue: 7 },
            Gate::empty("tooling"),
        )
    }

    fn ctx(dry_run: bool) -> Context<DecideState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            DecideState::default(),
            Logbook::null(),
        )
    }

    fn issue(state: &str, labels: &[&str], body: &str) -> Issue {
        Issue {
            number: 7,
            title: "Pay".to_string(),
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: body.to_string(),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_closed_or_unasked_issue_is_a_clean_skip() {
        for (state, labelled) in [("closed", &[labels::NEEDS_DECISION][..]), ("open", &[][..])] {
            let gh = Rc::new(FakeGitHub {
                issues: vec![issue(state, labelled, "")],
                ..FakeGitHub::default()
            });
            let run = built(&gh, dir(&format!("skip-{state}")));
            let mut context = ctx(false);
            let verdict = Guarded::execute(&run, &mut context).await.expect("clean");
            assert_eq!(verdict, Verdict::Continue);
            assert!(context.state.issue.is_none() && gh.writes().is_empty());
        }
    }

    #[tokio::test]
    async fn an_unreadable_issue_fails() {
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Failed("no credentials".to_string())),
            ..FakeGitHub::default()
        });
        let err = Workflow::execute(&built(&gh, dir("broken")), &mut ctx(false))
            .await
            .expect_err("fails");
        assert!(err.reason().contains("no credentials"));
    }

    #[tokio::test]
    async fn an_owner_s_answer_takes_the_request_and_pays_nothing() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(
                "open",
                &[labels::NEEDS_DECISION],
                "## Decision\n\nkind: stop\nblocks: which\nround: 1\ndecided-by: owner\nverdict: Stripe",
            )],
            ..FakeGitHub::default()
        });
        let run = built(&gh, dir("owner"));
        let mut context = ctx(false);
        Workflow::execute(&run, &mut context).await.expect("taken");
        assert_eq!(
            gh.writes(),
            [crate::common::fake_github::Wrote::Unlabelled(
                7,
                labels::NEEDS_DECISION.to_string()
            )]
        );
        assert!(
            Workflow::summary(&run, &context)
                .unwrap_or_default()
                .contains("nothing paid")
        );
    }

    #[test]
    fn one_decision_per_issue_holds_its_lock() {
        let gh = Rc::new(FakeGitHub::default());
        let decide_dir = dir("lock");
        let run = built(&gh, decide_dir.clone());
        let lock = Workflow::lock(&run).expect("locked");
        assert_eq!((lock.dir, lock.name), (decide_dir.as_path(), "7"));
        assert!(Workflow::held(&run, &ctx(false)).contains("#7"));
        assert_eq!(Executable::pre(&run).expect("mounted").name, "tooling");
        assert!(std::ptr::eq(
            Workflow::remaining(&run),
            std::ptr::from_ref(&run.remaining)
        ));
        assert_eq!(
            Workflow::summary(&run, &ctx(true)).as_deref(),
            Some("dry run — nothing written")
        );
    }
}
