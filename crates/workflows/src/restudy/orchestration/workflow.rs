//! The whole re-study: what it establishes before paying, and the lock it
//! holds.
//!
//! Precheck reads the task. One closed since the router saw it, no agent
//! task, or no longer due (another run re-studied it, an owner decided, its
//! streak dropped) is the clean "nothing to do" stop. Only a task that
//! cannot be read at all fails.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Executable, Gate, Lock, Round, Workflow};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;

use crate::common::{labels, restudy};
use crate::restudy::data::state::StudyState;

/// A whole re-study: precheck, lock, the sequence, a summary.
pub struct RestudyRun {
    /// The tooling this workflow demands.
    pub pre: Gate<StudyState>,
    /// How many rounds are left. One run is one re-study.
    pub remaining: Cell<u32>,
    /// What reads the task.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// The tasks' lock directory.
    pub restudy_dir: PathBuf,
    /// The task to re-study.
    pub task: u64,
    /// The task's number as the lock names it.
    pub lock_name: String,
    /// The round, already mounted.
    pub round: Round<StudyState>,
}

#[async_trait(?Send)]
impl Workflow<StudyState> for RestudyRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<StudyState>) -> Outcome<Option<String>> {
        let issue = self.gh.issue(self.task).await?;
        if !issue.is_open() {
            return Ok(Some(format!(
                "{} is closed — nothing to re-study",
                issue.reference()
            )));
        }
        if !issue.has(labels::AGENT) {
            return Ok(Some(format!(
                "{} is no {} task — nothing to re-study",
                issue.reference(),
                labels::AGENT
            )));
        }
        if !restudy::due_on(&issue) {
            return Ok(Some(format!(
                "{} is not due a re-study — re-studied already, or its streak is over",
                issue.reference()
            )));
        }
        ctx.traces
            .say(&format!("restudy {}  {}", issue.reference(), issue.title));
        ctx.state.issue = Some(issue);
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.restudy_dir,
            name: &self.lock_name,
        })
    }

    fn held(&self, _ctx: &Context<StudyState>) -> String {
        format!(
            "skip — a re-study of task #{} is already running",
            self.task
        )
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<StudyState> + '_> {
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<StudyState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing written".to_string());
        }
        Some(format!(
            "task #{}: re-studied — {}",
            self.task,
            ctx.state
                .published
                .as_deref()
                .unwrap_or("nothing published")
        ))
    }
}

#[async_trait(?Send)]
impl Executable<StudyState> for RestudyRun {
    fn pre(&self) -> Option<&Gate<StudyState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<StudyState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_github::FakeGitHub;
    use crate::restudy::config::fake as config_fake;
    use crate::restudy::ports::fake as ports_fake;
    use crate::restudy::run;
    use harness_core::domain::{Halt, Issue};
    use harness_core::execution::{Guarded, Settings};
    use harness_core::traces::Logbook;

    fn built(gh: &Rc<FakeGitHub>) -> RestudyRun {
        run::build(
            &ports_fake::with(Rc::clone(gh), &FakeRepos::default()),
            &config_fake::in_dir(PathBuf::from("/tmp/restudy-test")),
            &run::Request { task: 219 },
            Gate::empty("tooling"),
        )
    }

    fn ctx(dry_run: bool) -> Context<StudyState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            StudyState::default(),
            Logbook::null(),
        )
    }

    fn task(state: &str, labels_on: &[&str], decision: &str) -> Issue {
        Issue {
            number: 219,
            title: "Freeze".to_string(),
            state: state.to_string(),
            labels: labels_on.iter().map(ToString::to_string).collect(),
            body: format!("## Scope\n\nx\n\n## Decision\n\n{decision}"),
            ..Issue::default()
        }
    }

    const DUE: &str = "kind: wait\nblocks: x\nround: 1\nmilestone: #28\ndiscovered: 2";

    #[tokio::test]
    async fn a_closed_a_stranger_s_or_a_task_no_longer_due_is_a_clean_skip() {
        for issue in [
            task("closed", &[labels::AGENT], DUE),
            task("open", &[labels::HUMAN], DUE),
            task(
                "open",
                &[labels::AGENT],
                &format!("{DUE}\nrestudied: 2\nrestudies: 1"),
            ),
            task(
                "open",
                &[labels::AGENT],
                "blocks: x\ndiscovered: 1\nmilestone: #28",
            ),
        ] {
            let gh = Rc::new(FakeGitHub {
                issues: vec![issue.clone()],
                ..FakeGitHub::default()
            });
            let run = built(&gh);
            let mut context = ctx(false);
            let skipped = Workflow::precheck(&run, &mut context).await.expect("clean");
            assert!(skipped.is_some(), "{}", issue.body);
            assert!(context.state.issue.is_none());
            let verdict = Guarded::execute(&run, &mut ctx(false))
                .await
                .expect("clean");
            assert_eq!(verdict, Verdict::Continue);
            assert!(gh.writes().is_empty(), "{:?}", gh.writes());
        }
        let due = Rc::new(FakeGitHub {
            issues: vec![task("open", &[labels::AGENT], DUE)],
            ..FakeGitHub::default()
        });
        let mut context = ctx(false);
        assert_eq!(
            Workflow::precheck(&built(&due), &mut context)
                .await
                .expect("due"),
            None
        );
        assert_eq!(context.state.issue.map(|i| i.number), Some(219));
    }

    #[tokio::test]
    async fn an_unreadable_task_fails() {
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Failed("no credentials".to_string())),
            ..FakeGitHub::default()
        });
        let err = Workflow::execute(&built(&gh), &mut ctx(false))
            .await
            .expect_err("fails");
        assert!(err.reason().contains("no credentials"));
    }

    #[test]
    fn one_re_study_per_task_holds_its_lock_and_says_what_it_published() {
        let gh = Rc::new(FakeGitHub::default());
        let run = built(&gh);
        let lock = Workflow::lock(&run).expect("locked");
        assert_eq!(
            (lock.dir, lock.name),
            (std::path::Path::new("/tmp/restudy-test"), "219")
        );
        assert!(Workflow::held(&run, &ctx(false)).contains("#219"));
        assert_eq!(Executable::pre(&run).expect("mounted").name, "tooling");
        assert!(std::ptr::eq(
            Workflow::remaining(&run),
            std::ptr::from_ref(&run.remaining)
        ));
        assert_eq!(
            Workflow::summary(&run, &ctx(true)).as_deref(),
            Some("dry run — nothing written")
        );
        assert_eq!(
            Workflow::summary(&run, &ctx(false)).as_deref(),
            Some("task #219: re-studied — nothing published")
        );
        let mut done = ctx(false);
        done.state.published = Some("waits on #567".to_string());
        assert_eq!(
            Workflow::summary(&run, &done).as_deref(),
            Some("task #219: re-studied — waits on #567")
        );
    }
}
