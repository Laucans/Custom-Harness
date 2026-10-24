//! The whole arbitration: what it establishes before paying, and the lock
//! it holds.
//!
//! The refusals are in `precheck`, before any session: a closed task, one
//! that does not carry the label (unless forced), one with no question
//! under `## Decision`, one with no parsable `## Architecture`, one under
//! no open milestone. One arbitration per run, so `remaining` starts at 1.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict, milestone_label};
use harness_core::execution::{Context, Executable, Gate, Lock, Round, Workflow};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;

use crate::arbiter::data::board;
use crate::arbiter::data::state::ArbiterState;
use crate::common::decision::Question;
use crate::common::{labels, sections};

/// A whole arbitration: precheck, lock, the sequence, a summary.
pub struct ArbiterRun {
    /// The tooling this workflow demands.
    pub pre: Gate<ArbiterState>,
    /// How many arbitrations are left. One run does one.
    pub remaining: Cell<u32>,
    /// What reads the task and its milestone.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// The locks' folder — one lock per task.
    pub arbiter_dir: PathBuf,
    /// The task to arbitrate.
    pub issue: u64,
    /// `issue`, as text — the name the lock carries.
    pub lock_name: String,
    /// Arbitrate even without the label.
    pub force: bool,
    /// The round, already mounted.
    pub round: Round<ArbiterState>,
}

#[async_trait(?Send)]
impl Workflow<ArbiterState> for ArbiterRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<ArbiterState>) -> Outcome<Option<String>> {
        let issue = self.gh.issue(self.issue).await?;
        if issue.is_closed() {
            return Err(Halt::Halted(format!(
                "{} is closed — nothing to arbitrate",
                issue.reference()
            )));
        }
        if !issue.has(labels::NEEDS_AI_DECISION) && !self.force {
            return Err(Halt::Halted(format!(
                "{} does not carry {} — add it with `gh issue edit {} --add-label {}`, or \
                 re-run with --force",
                issue.reference(),
                labels::NEEDS_AI_DECISION,
                self.issue,
                labels::NEEDS_AI_DECISION
            )));
        }
        let found = sections::parse(&issue.body);
        let Some(question) = found
            .get(sections::DECISION)
            .and_then(|text| Question::parse(text))
        else {
            return Err(Halt::Halted(format!(
                "{} has no question under ## Decision — nothing to arbitrate",
                issue.reference()
            )));
        };
        if board::declaration_of(&issue).is_none() {
            return Err(Halt::Halted(format!(
                "{} has no parsable ## Architecture section — nothing to chain on",
                issue.reference()
            )));
        }
        let mut placed = None;
        for milestone in self.gh.issues_labelled(labels::MILESTONE, "open").await? {
            let subs = self.gh.sub_issues(milestone.number).await?;
            if subs.iter().any(|sub| sub.number == issue.number) {
                placed = Some((milestone.number, milestone.title, subs));
                break;
            }
        }
        let Some((milestone, title, siblings)) = placed else {
            return Err(Halt::Halted(format!(
                "{} is under no open milestone — the board it would be chained on does not exist",
                issue.reference()
            )));
        };
        ctx.traces.say(&format!(
            "arbitrating {} ({}) — milestone #{milestone}, round {}: {}",
            issue.reference(),
            issue.title,
            question.round,
            question.blocks
        ));
        ctx.state.issue = Some(issue);
        ctx.state.question = Some(question);
        ctx.state.milestone = Some(milestone);
        ctx.tags.milestone = milestone_label(milestone, &title);
        ctx.state.siblings = siblings;
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.arbiter_dir,
            name: &self.lock_name,
        })
    }

    fn held(&self, _ctx: &Context<ArbiterState>) -> String {
        format!(
            "skip — an arbitration of issue #{} is already running",
            self.issue
        )
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<ArbiterState> + '_> {
        // Borrowed, not rebuilt: there is only ever one round per run.
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<ArbiterState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing written".to_string());
        }
        ctx.state.outcome.clone()
    }
}

#[async_trait(?Send)]
impl Executable<ArbiterState> for ArbiterRun {
    fn pre(&self) -> Option<&Gate<ArbiterState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<ArbiterState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    //! The refusals of the precheck, against a fake GitHub.

    use super::*;
    use crate::arbiter::config::fake as config_fake;
    use crate::arbiter::ports::fake as ports_fake;
    use crate::arbiter::run;
    use crate::common::fake_github::FakeGitHub;
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn issue(number: u64, labels: &[&str], body: &str) -> Issue {
        Issue {
            number,
            title: format!("task {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: body.to_string(),
            ..Issue::default()
        }
    }

    const TASK: &str = "## Architecture\n\nunit: micro-ui\nsystem: c\nproduces: Pjs\n\n\
                        ## Decision\n\nblocks: lists PJs\nneeds: PJ\nround: 1\n";

    fn built(gh: &Rc<FakeGitHub>, force: bool) -> ArbiterRun {
        run::build(
            &ports_fake::with(Rc::clone(gh)),
            &config_fake::config(),
            &run::Request { issue: 20, force },
            Gate::empty("tooling"),
        )
    }

    fn ctx() -> Context<ArbiterState> {
        Context::new(
            Settings {
                dry_run: true,
                stages: String::new(),
            },
            ArbiterState::default(),
            Logbook::null(),
        )
    }

    fn github(task: Issue) -> Rc<FakeGitHub> {
        Rc::new(FakeGitHub {
            issues: vec![
                task.clone(),
                issue(5, &[labels::MILESTONE], "the milestone"),
            ],
            subs: vec![(
                5,
                vec![
                    task,
                    issue(
                        12,
                        &[],
                        "## Architecture\n\nunit: aggregate\nsystem: c\nproduces: PJ",
                    ),
                ],
            )],
            ..FakeGitHub::default()
        })
    }

    #[tokio::test]
    async fn a_labelled_task_under_a_milestone_is_read_with_its_siblings() {
        let gh = github(issue(20, &[labels::AGENT, labels::NEEDS_AI_DECISION], TASK));
        let mut context = ctx();
        built(&gh, false).precheck(&mut context).await.expect("ok");
        assert_eq!(context.state.milestone, Some(5));
        assert_eq!(
            context.tags.milestone, "#5 task 5",
            "the ledger's milestone"
        );
        assert_eq!(context.state.siblings.len(), 2);
        assert_eq!(context.state.question().needs, ["PJ"]);
    }

    #[tokio::test]
    async fn a_closed_task_an_unlabelled_one_and_one_without_a_question_are_refused() {
        let mut closed = issue(20, &[labels::NEEDS_AI_DECISION], TASK);
        closed.state = "closed".to_string();
        assert!(
            built(&github(closed), false)
                .precheck(&mut ctx())
                .await
                .is_err()
        );

        let unlabelled = issue(20, &[labels::AGENT], TASK);
        assert!(
            built(&github(unlabelled.clone()), false)
                .precheck(&mut ctx())
                .await
                .is_err()
        );
        assert!(
            built(&github(unlabelled), true)
                .precheck(&mut ctx())
                .await
                .is_ok(),
            "forced"
        );

        let no_question = issue(
            20,
            &[labels::NEEDS_AI_DECISION],
            "## Architecture\n\nunit: micro-ui\nsystem: c",
        );
        assert!(
            built(&github(no_question), false)
                .precheck(&mut ctx())
                .await
                .is_err()
        );

        let no_architecture = issue(
            20,
            &[labels::NEEDS_AI_DECISION],
            "## Decision\n\nblocks: x\nround: 1",
        );
        assert!(
            built(&github(no_architecture), false)
                .precheck(&mut ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_task_under_no_open_milestone_is_refused() {
        let task = issue(20, &[labels::NEEDS_AI_DECISION], TASK);
        let gh = Rc::new(FakeGitHub {
            issues: vec![task.clone()],
            ..FakeGitHub::default()
        });
        assert!(built(&gh, false).precheck(&mut ctx()).await.is_err());
        // A milestone whose tasks do not include this one is not its board.
        let elsewhere = Rc::new(FakeGitHub {
            issues: vec![task, issue(5, &[labels::MILESTONE], "the milestone")],
            subs: vec![(5, vec![issue(12, &[], "")])],
            ..FakeGitHub::default()
        });
        assert!(built(&elsewhere, false).precheck(&mut ctx()).await.is_err());
    }

    #[test]
    fn the_run_locks_its_task_and_carries_its_tooling_gate() {
        let workflow = built(&Rc::new(FakeGitHub::default()), false);
        let lock = Workflow::lock(&workflow).expect("one lock per task");
        assert_eq!(lock.name, "20");
        assert_eq!(
            Executable::pre(&workflow).map(|gate| gate.name),
            Some("tooling")
        );
    }

    #[tokio::test]
    async fn a_dry_run_through_the_whole_sequence_writes_nothing() {
        let gh = github(issue(20, &[labels::AGENT, labels::NEEDS_AI_DECISION], TASK));
        let workflow = built(&gh, false);
        let mut context = ctx();
        let verdict = harness_core::execution::Guarded::execute(&workflow, &mut context).await;
        assert!(verdict.is_ok(), "{verdict:?}");
        assert!(gh.writes().is_empty(), "expected nothing");
        assert!(
            context.state.outcome.is_some(),
            "the round ran through apply, which says what it would have written"
        );
        assert_eq!(context.state.resolution.chained, [("PJ".to_string(), 12)]);
        assert_eq!(
            workflow.summary(&context).as_deref(),
            Some("dry run — nothing written")
        );
        assert!(workflow.held(&context).contains("#20"));
    }
}
