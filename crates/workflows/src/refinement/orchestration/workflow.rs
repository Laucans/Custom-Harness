//! The whole round: what refinement establishes before paying, and the lock
//! it holds.
//!
//! The refusals are in `precheck`, before any session: a closed issue, an
//! issue that is neither a task nor a milestone, a milestone asked for its
//! technical half, an unlabelled issue, and a comment read that
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
use harness_core::domain::{Halt, Outcome, Verdict, prompts};
use harness_core::execution::{Context, Executable, Gate, Lock, Round, Workflow};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;

use crate::common::{hierarchy, labels};
use crate::refinement::data::phase::Phase;
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
    /// Refine even if the issue does not carry the phase's label.
    pub force: bool,
    /// Which half of the refinement this run does.
    pub phase: Phase,
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
        let milestone = issue.has(labels::MILESTONE);
        if !(milestone || issue.has(labels::AGENT) || issue.has(labels::HUMAN)) {
            return Err(Halt::Halted(format!(
                "#{} carries none of {}, {} or {} — refinement works a task or \
                 a milestone: add one to it, or refine another issue",
                self.issue,
                labels::AGENT,
                labels::HUMAN,
                labels::MILESTONE
            )));
        }
        // A milestone has no code of its own to design: its technical half is
        // written task by task, once `split` has cut it.
        if milestone && self.phase == Phase::Technical {
            return Err(Halt::Halted(format!(
                "#{} is a milestone — the technical refinement works a task: \
                 refine one of its tasks instead",
                self.issue
            )));
        }
        let asked = self.phase.requested_by();
        if !issue.has(asked) && !self.force {
            return Err(Halt::Halted(format!(
                "#{} does not carry {asked} — add it with `gh issue edit {} \
                 --add-label {asked}`, or re-run with --force",
                self.issue, self.issue,
            )));
        }
        // The technical half builds on the business one: refining the code
        // side of a body nobody has specified writes a plan for a guess.
        if self.phase == Phase::Technical && !issue.has(labels::SPEC_WRITTEN) && !self.force {
            return Err(Halt::Halted(format!(
                "#{} does not carry {} — run the business refinement ({}) \
                 first, or re-run with --force",
                self.issue,
                labels::SPEC_WRITTEN,
                labels::REFINEMENT
            )));
        }

        // A failed read is never read as "no comments": the round would
        // restart at 1 and rewrite the body over two rounds.
        let comments = self.gh.issue_comments(self.issue).await?;

        let round_no = rounds::counter(&comments, self.phase) + 1;
        let found = sections::parse(&issue.body);
        let wanted = rounds::planned(self.phase, round_no, !self.context.is_empty());

        let said = if wanted.is_empty() {
            "router decides".to_string()
        } else {
            wanted.join(" ")
        };
        ctx.traces.say(&format!(
            "refining #{} — round {round_no} ({said})",
            self.issue
        ));

        ctx.state.hierarchy = if milestone {
            hierarchy::above(self.gh.as_ref(), issue.number)
                .await?
                .map(|found| {
                    prompts::milestone_hierarchy_block(
                        Some(&found.roadmap),
                        &found.milestones,
                        &issue.number.to_string(),
                        prompts::INJECTOR,
                    )
                })
                .unwrap_or_default()
        } else {
            hierarchy::around(self.gh.as_ref(), &issue)
                .await?
                .map(|scope| prompts::hierarchy_block(&scope, prompts::INJECTOR))
                .unwrap_or_default()
        };
        ctx.state.issue = Some(issue);
        ctx.state.phase = self.phase;
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
        built_for(Phase::Business, gh, force, context, refinement_dir)
    }

    fn built_for(
        phase: Phase,
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
                phase,
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
    async fn an_issue_that_is_neither_a_task_nor_a_milestone_is_refused() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(25, "open", &[labels::REFINEMENT], "")],
            ..FakeGitHub::default()
        });
        let round = built(&gh, false, "", dir("not-a-task"));
        let mut context = ctx(true);
        let err = round.execute(&mut context).await.expect_err("must stop");
        assert!(err.reason().contains("none of"));
    }

    #[tokio::test]
    async fn a_milestone_is_refined_under_its_roadmap() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![
                issue(2, "open", &[labels::ROADMAP], ""),
                issue(25, "open", &[labels::MILESTONE, labels::REFINEMENT], ""),
            ],
            subs: vec![(
                2,
                vec![issue(
                    25,
                    "open",
                    &[labels::MILESTONE, labels::REFINEMENT],
                    "",
                )],
            )],
            ..FakeGitHub::default()
        });
        let round = built(&gh, false, "", dir("milestone"));
        let mut context = ctx(true);
        round.execute(&mut context).await.expect("a success");
        assert!(context.state.hierarchy.contains("ROADMAP #2"));
        assert!(context.state.hierarchy.contains("is a **milestone**"));
    }

    #[tokio::test]
    async fn a_milestone_has_no_technical_refinement() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(
                25,
                "open",
                &[
                    labels::MILESTONE,
                    labels::TECH_REFINEMENT,
                    labels::SPEC_WRITTEN,
                ],
                "",
            )],
            ..FakeGitHub::default()
        });
        let round = built_for(Phase::Technical, &gh, false, "", dir("milestone-tech"));
        let mut context = ctx(true);
        let err = round.execute(&mut context).await.expect_err("must stop");
        assert!(err.reason().contains("is a milestone"));
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
    async fn the_business_round_one_wants_the_three_business_sections() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(25, "open", &[labels::AGENT, labels::REFINEMENT], "")],
            ..FakeGitHub::default()
        });
        let round = built(&gh, false, "", dir("round-one-wants"));
        let mut context = ctx(true);
        round.execute(&mut context).await.expect("a success");
        assert_eq!(context.state.wanted, Phase::Business.keys().to_vec());
    }

    #[tokio::test]
    async fn the_technical_round_wants_the_two_technical_sections() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(
                25,
                "open",
                &[labels::AGENT, labels::TECH_REFINEMENT, labels::SPEC_WRITTEN],
                "",
            )],
            ..FakeGitHub::default()
        });
        let round = built_for(Phase::Technical, &gh, false, "", dir("tech-wants"));
        let mut context = ctx(true);
        round.execute(&mut context).await.expect("a success");
        assert_eq!(context.state.wanted, Phase::Technical.keys().to_vec());
        assert_eq!(context.state.phase, Phase::Technical);
    }

    #[tokio::test]
    async fn the_technical_round_refuses_a_body_nobody_has_specified() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(
                25,
                "open",
                &[labels::AGENT, labels::TECH_REFINEMENT],
                "",
            )],
            ..FakeGitHub::default()
        });
        let round = built_for(Phase::Technical, &gh, false, "", dir("tech-no-spec"));
        let mut context = ctx(true);
        let err = round.execute(&mut context).await.expect_err("must stop");
        assert!(err.reason().contains(labels::SPEC_WRITTEN));
    }

    #[tokio::test]
    async fn the_technical_round_needs_its_own_label() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(
                25,
                "open",
                &[labels::AGENT, labels::REFINEMENT, labels::SPEC_WRITTEN],
                "",
            )],
            ..FakeGitHub::default()
        });
        let round = built_for(Phase::Technical, &gh, false, "", dir("tech-no-label"));
        let mut context = ctx(true);
        let err = round.execute(&mut context).await.expect_err("must stop");
        assert!(err.reason().contains(labels::TECH_REFINEMENT));
    }
}
