//! The whole evolution: what it establishes before paying, and the lock it
//! holds.
//!
//! Precheck reads the request issue. One closed since the router saw it, or
//! no longer labelled `harness:contract-change` (a person took it), is the
//! clean "nothing to do" stop. Only an issue that cannot be read at all
//! fails.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Executable, Gate, Lock, Round, Workflow};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;

use crate::common::labels;
use crate::contract_manager::data::state::ManagerState;

/// A whole evolution: precheck, lock, the sequence, a summary.
pub struct ManagerRun {
    /// The tooling this workflow demands.
    pub pre: Gate<ManagerState>,
    /// How many rounds are left. One run is one evolution.
    pub remaining: Cell<u32>,
    /// What reads the issue.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// The requests' lock directory.
    pub lock_dir: PathBuf,
    /// The request issue.
    pub issue: u64,
    /// The issue's number as the lock names it.
    pub lock_name: String,
    /// The round, already mounted.
    pub round: Round<ManagerState>,
}

#[async_trait(?Send)]
impl Workflow<ManagerState> for ManagerRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<ManagerState>) -> Outcome<Option<String>> {
        let issue = self.gh.issue(self.issue).await?;
        if !issue.is_open() {
            return Ok(Some(format!(
                "{} is closed — nothing to evolve",
                issue.reference()
            )));
        }
        if !issue.has(labels::CONTRACT_CHANGE) {
            return Ok(Some(format!(
                "{} does not carry {} — nobody asks for a contract change now",
                issue.reference(),
                labels::CONTRACT_CHANGE
            )));
        }
        ctx.traces.say(&format!(
            "contract change {}  {}",
            issue.reference(),
            issue.title
        ));
        ctx.state.issue = Some(issue);
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.lock_dir,
            name: &self.lock_name,
        })
    }

    fn held(&self, _ctx: &Context<ManagerState>) -> String {
        format!(
            "skip — the contract change #{} is already being evolved",
            self.issue
        )
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<ManagerState> + '_> {
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<ManagerState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing written".to_string());
        }
        let id = ctx
            .state
            .request
            .as_ref()
            .map_or_else(|| "?".to_string(), |r| r.id.clone());
        Some(if ctx.state.nobody_else_s() {
            format!(
                "issue #{}: `{id}` is the requester's own — nothing paid",
                self.issue
            )
        } else {
            format!("issue #{}: `{id}` evolved", self.issue)
        })
    }
}

#[async_trait(?Send)]
impl Executable<ManagerState> for ManagerRun {
    fn pre(&self) -> Option<&Gate<ManagerState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<ManagerState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::contract_manager::config::fake as config_fake;
    use crate::contract_manager::ports::fake as ports_fake;
    use crate::contract_manager::run;
    use harness_core::domain::{Halt, Issue};
    use harness_core::execution::{Guarded, Settings};
    use harness_core::traces::Logbook;

    fn dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("harness-contract-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn built(gh: &Rc<FakeGitHub>, lock_dir: PathBuf) -> ManagerRun {
        run::build(
            &ports_fake::with(Rc::clone(gh)),
            &config_fake::in_dir(lock_dir),
            &run::Request { issue: 70 },
            Gate::empty("tooling"),
        )
    }

    fn ctx(dry_run: bool) -> Context<ManagerState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            ManagerState::default(),
            Logbook::null(),
        )
    }

    fn issue(state: &str, labels: &[&str], body: &str) -> Issue {
        Issue {
            number: 70,
            title: "Contract change: a.b for #61".to_string(),
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: body.to_string(),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_closed_or_unasked_issue_is_a_clean_skip() {
        for (state, labelled) in [
            ("closed", &[labels::CONTRACT_CHANGE][..]),
            ("open", &[][..]),
        ] {
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
    async fn an_id_nobody_else_produces_is_handed_back_without_a_session() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(
                "open",
                &[labels::CONTRACT_CHANGE],
                "id: a.b\ntask: #61\nneed: more",
            )],
            ..FakeGitHub::default()
        });
        let run = built(&gh, dir("own"));
        let mut context = ctx(false);
        Workflow::execute(&run, &mut context)
            .await
            .expect("handed back");
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::Closed(70)), "{writes:?}");
        assert!(writes.iter().any(
            |w| matches!(w, Wrote::Comment(61, said) if said.contains("change it in this task"))
        ));
        assert!(
            Workflow::summary(&run, &context)
                .unwrap_or_default()
                .contains("nothing paid")
        );
    }

    #[test]
    fn one_evolution_per_request_holds_its_lock() {
        let gh = Rc::new(FakeGitHub::default());
        let lock_dir = dir("lock");
        let run = built(&gh, lock_dir.clone());
        let lock = Workflow::lock(&run).expect("locked");
        assert_eq!((lock.dir, lock.name), (lock_dir.as_path(), "70"));
        assert!(Workflow::held(&run, &ctx(false)).contains("#70"));
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
