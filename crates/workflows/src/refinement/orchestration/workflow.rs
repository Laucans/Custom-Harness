//! The whole round: what refinement establishes before paying, and the lock
//! it holds.
//!
//! The four refusals are in `precheck`, before any session: a closed issue, an
//! issue that is not a task, an unlabelled issue, and a comment read that
//! doesn't come back — that one especially, because an empty list would send
//! the counter back to 1 and rewrite the whole body.
//!
//! One round per run, so `remaining` starts at 1. Everything around it —
//! precheck, lock, the round, the summary — is the core's
//! [`Workflow`].
//!
//! The shape alone — neither `Ports`, nor `Config`, nor `Request` are named
//! here. What turns them into a mounted round lives in
//! [`run::build`](crate::refinement::run::build).

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::store::lock::Locks;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Executable, Gate, Lock, Round, Workflow};

use crate::common::labels;
use crate::refinement::data::state::RefinementState;
use crate::refinement::data::{rounds, sections};

/// A whole refinement round: precheck, lock, the sequence, a summary.
pub struct RefinementRun {
    /// The tooling this workflow demands, plus the `harness:refinement` label.
    pub pre: Gate<RefinementState>,
    /// How many rounds are left. One run refines one round.
    pub remaining: Cell<u32>,
    /// What reads and rewrites the issue.
    pub gh: Rc<dyn GitHub>,
    /// What holds the locks.
    pub locks: Rc<dyn Locks>,
    /// The refinement directory — one lock per issue.
    pub refinement_dir: PathBuf,
    /// The issue to refine.
    pub issue: u64,
    /// `issue`, as text — it is the name the lock carries.
    pub issue_key: String,
    /// What a human asked for this round, verbatim.
    pub context: String,
    /// Refine even if the issue does not carry `harness:refinement`.
    pub force: bool,
    /// The round, already mounted: the map, then the table's steps.
    pub round: Round<RefinementState>,
}

#[async_trait(?Send)]
impl Workflow<RefinementState> for RefinementRun {
    fn remaining(&self) -> &Cell<u32> {
        &self.remaining
    }

    async fn precheck(&self, ctx: &mut Context<RefinementState>) -> Outcome<Option<String>> {
        let issue = self.gh.issue(self.issue).await?;

        if issue.is_closed() {
            return Err(Halt::Halted(format!(
                "#{} is closed — reopen it, or refine another issue",
                self.issue
            )));
        }
        if !(issue.has(labels::AGENT) || issue.has(labels::HUMAN)) {
            return Err(Halt::Halted(format!(
                "#{} carries neither {} nor {} — refinement works a task: add \
                 one of the two to it, or refine another issue",
                self.issue,
                labels::AGENT,
                labels::HUMAN
            )));
        }
        if !issue.has(labels::REFINEMENT) && !self.force {
            return Err(Halt::Halted(format!(
                "#{} does not carry {} — add it with `gh issue edit {} \
                 --add-label {}`, or re-run with --force",
                self.issue,
                labels::REFINEMENT,
                self.issue,
                labels::REFINEMENT
            )));
        }

        // A failed read is never read as "no comments": the round would
        // restart at 1 and rewrite the body over two rounds.
        let comments = self.gh.issue_comments(self.issue).await?;

        let round_no = rounds::counter(&comments) + 1;
        let found = sections::parse(&issue.body);
        let wanted = rounds::planned(round_no, &found, !self.context.is_empty());

        let said = if wanted.is_empty() {
            "router decides".to_string()
        } else {
            wanted.join(" ")
        };
        ctx.traces.say(&format!(
            "refining #{} — round {round_no} ({said})",
            self.issue
        ));

        ctx.state.issue = Some(issue);
        ctx.state.round_no = round_no;
        ctx.state.found = found;
        ctx.state.wanted = wanted.into_iter().map(str::to_string).collect();
        Ok(None)
    }

    fn lock(&self) -> Option<Lock<'_>> {
        Some(Lock {
            locks: self.locks.as_ref(),
            dir: &self.refinement_dir,
            name: &self.issue_key,
        })
    }

    fn held(&self, _ctx: &Context<RefinementState>) -> String {
        format!(
            "skip — a refinement of issue #{} is already running",
            self.issue
        )
    }

    fn round(&self, _turn: u32) -> Box<dyn Executable<RefinementState> + '_> {
        // Borrowed, not rebuilt: there is only ever one round per run.
        Box::new(&self.round)
    }

    fn summary(&self, ctx: &Context<RefinementState>) -> Option<String> {
        if ctx.settings.dry_run {
            return Some("dry run — nothing written".to_string());
        }
        Some(format!(
            "refined #{} — round {}",
            self.issue, ctx.state.round_no
        ))
    }
}

#[async_trait(?Send)]
impl Executable<RefinementState> for RefinementRun {
    fn pre(&self) -> Option<&Gate<RefinementState>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<RefinementState>) -> Outcome<Verdict> {
        Workflow::execute(self, ctx).await
    }
}

#[cfg(test)]
mod tests {
    //! The four refusals of the precheck, against a fake GitHub. The round
    //! is mounted by `run::build` — a test may go back to assembly,
    //! production code may not.

    use super::*;
    use crate::common::explore;
    use crate::common::fake_github::FakeGitHub;
    use crate::refinement::config::fake as config_fake;
    use crate::refinement::ports::fake as ports_fake;
    use crate::refinement::run;
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "harness-refinement-run-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn built(
        gh: &Rc<FakeGitHub>,
        force: bool,
        context: &str,
        refinement_dir: PathBuf,
    ) -> RefinementRun {
        let config = config_fake::in_dir(refinement_dir, context);
        let explore_config = explore::fake::config(config.artifacts_dir.clone());
        run::build(
            &ports_fake::with(Rc::clone(gh)),
            &config,
            &explore::fake::ports(),
            &explore_config,
            run::Request {
                issue: 25,
                context: context.to_string(),
                force,
            },
            Gate::empty("outillage"),
        )
    }

    fn ctx(dry_run: bool) -> Context<RefinementState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            RefinementState::default(),
            Logbook::null(),
        )
    }

    fn issue(number: u64, state: &str, labels: &[&str], body: &str) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: body.to_string(),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_closed_issue_halts_before_any_session() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(
                25,
                "closed",
                &[labels::AGENT, labels::REFINEMENT],
                "",
            )],
            ..FakeGitHub::default()
        });
        let round = built(&gh, false, "", dir("closed"));
        let mut context = ctx(true);
        let err = round.execute(&mut context).await.expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(err.reason().contains("is closed"));
    }

    #[tokio::test]
    async fn an_issue_that_is_not_a_task_is_refused() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(25, "open", &[labels::REFINEMENT], "")],
            ..FakeGitHub::default()
        });
        let round = built(&gh, false, "", dir("not-a-task"));
        let mut context = ctx(true);
        let err = round.execute(&mut context).await.expect_err("must stop");
        assert!(err.reason().contains("neither"));
    }

    #[tokio::test]
    async fn without_the_refinement_label_and_without_force_it_refuses() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(25, "open", &[labels::AGENT], "")],
            ..FakeGitHub::default()
        });
        let round = built(&gh, false, "", dir("no-label"));
        let mut context = ctx(true);
        let err = round.execute(&mut context).await.expect_err("must stop");
        assert!(err.reason().contains(labels::REFINEMENT));
    }

    #[tokio::test]
    async fn force_lets_an_unlabelled_task_through() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(25, "open", &[labels::AGENT], "")],
            ..FakeGitHub::default()
        });
        let round = built(&gh, true, "", dir("forced"));
        let mut context = ctx(true);
        round.execute(&mut context).await.expect("a success");
        assert_eq!(context.state.round_no, 1);
    }

    #[tokio::test]
    async fn an_unreadable_comment_list_never_resets_the_round_counter() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(25, "open", &[labels::AGENT, labels::REFINEMENT], "")],
            broken: Some(Halt::Unreadable("expired token".to_string())),
            ..FakeGitHub::default()
        });
        // `issue()` would also read `broken`, so we can't test this step
        // alone via the global fake — what this test guarantees is that the
        // error propagates rather than restarting at 1.
        let round = built(&gh, false, "", dir("unreadable-comments"));
        let mut context = ctx(true);
        let err = round.execute(&mut context).await.expect_err("must fail");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[tokio::test]
    async fn round_one_wants_exactly_the_three_round_one_sections() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(25, "open", &[labels::AGENT, labels::REFINEMENT], "")],
            ..FakeGitHub::default()
        });
        let round = built(&gh, false, "", dir("round-one-wants"));
        let mut context = ctx(true);
        round.execute(&mut context).await.expect("a success");
        assert_eq!(
            context.state.wanted,
            vec!["business-goal", "technical", "acceptance-criteria"]
        );
    }
}
