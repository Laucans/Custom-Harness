//! What the round **does**: pick, ask, record, mark.
//!
//! The counterpart to `gates.rs`, and the reason the two files exist
//! separately. Decision #1 says a `Verification` judges and doesn't write; so
//! everything that writes is here, including the "write" halves of Python's
//! two hybrid guards:
//!
//! | Python | its judging half | its acting half |
//! | --- | --- | --- |
//! | `spec_is_in_the_issue` | `gates::IssueBodyIsNotEmpty` | [`RecordSpecWritten`] |
//! | `task_is_delivered` | `gates::AMergedPrClosesTheTask` | [`MarkWaitingMerge`] |
//!
//! The split costs one issue re-read per pair: the action re-reads to write,
//! the gate re-reads to judge. That's the acknowledged price of "judging is
//! not doing" — a free local call against a session that costs dollars.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::store::spending::Spending;
use harness_core::domain::{Halt, Named, Outcome, Scoped, Verdict, prompts};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};

use crate::common::labels;
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::data::{board, tasks};

/// The number of the current task, or failure to read it.
fn number_of(ctx: &Context<Loop>) -> Outcome<u64> {
    ctx.state.task.number.parse().map_err(|_| {
        Halt::Failed(format!(
            "unreadable task number: {:?}",
            ctx.state.task.number
        ))
    })
}

/// Picks the round's task, or switches to rollover.
///
/// The first action of every round, and the only one with the right to find
/// nothing. Its three outputs are the `if/elif/else` that replaced a three-branch
/// router in a graph engine:
///
/// - a task (the resumed one, else the one from the table) → round continues;
/// - no playable task **but open tasks** → stop, naming the unblocking gesture.
///   Definitely not a rollover: confusing the two charges a `/planner` for a
///   `harness:ready` box no one checked;
/// - no open tasks at all → rollover.
pub struct PickTask {
    /// What reads the board.
    pub gh: Rc<dyn GitHub>,
    /// The task the resume point designates, if there is one.
    ///
    /// It **takes precedence** over the board's choice: a round interrupted
    /// after `/code` merges works on an already-closed issue that nothing else
    /// would offer, and `/create-test` would be lost.
    pub resuming: Option<String>,
}

#[async_trait(?Send)]
impl Action<Loop> for PickTask {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        // One board read per round. The launcher read the resume pointer, not
        // the board: so there aren't two reads that could give different answers.
        let here = board::read(self.gh.as_ref()).await?;
        ctx.state.milestone = Named {
            number: here.milestone.number.to_string(),
            title: here.milestone.title.clone(),
            body: here.milestone.body.clone(),
        };
        ctx.traces.say(&format!(
            "milestone {}: {}",
            here.milestone.reference(),
            here.milestone.title
        ));
        let resumed = self.resuming.as_deref().and_then(|key| here.find(key));
        let Some(task) = resumed.or_else(|| here.next()) else {
            if !here.open_agents().is_empty() {
                return Err(Halt::Halted(here.stuck()));
            }
            ctx.state.rollover = true;
            ctx.traces.say(&format!(
                "milestone {} has no open {} sub-issue — opening the next \
                 roadmap item",
                here.milestone.reference(),
                labels::AGENT
            ));
            return Ok(Verdict::Continue);
        };
        ctx.state.task = Named {
            number: task.number.to_string(),
            title: task.title.clone(),
            body: task.body.clone(),
        };
        ctx.state.task_key = task.key();
        ctx.state.kind = tasks::kind(task).to_string();
        ctx.state.spec_written = tasks::spec_written(task);
        ctx.state.resumed = resumed.is_some();
        ctx.traces.say(&format!(
            "task {}: {} [{}]",
            task.reference(),
            task.title,
            tasks::kind(task)
        ));
        Ok(Verdict::Continue)
    }
}

/// Records that the SPEC is in the issue's body.
///
/// The "write" half of the old `spec_is_in_the_issue`: it re-reads the body —
/// which **is** the SPEC, and that the next stage will receive in its scope —,
/// copies it into the state, and places the label.
///
/// It doesn't judge. An empty body is not an error here, it's simply that
/// there's nothing to record: it doesn't label and stays silent, and it's the
/// following gate — `gates::IssueBodyIsNotEmpty` — that stops the round by
/// saying so. Placing the label first would skip the write on the next run when
/// nothing was written.
pub struct RecordSpecWritten {
    /// What re-reads the issue and labels it.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<Loop> for RecordSpecWritten {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let number = number_of(ctx)?;
        let issue = self.gh.issue(number).await?;
        if issue.body.trim().is_empty() {
            return Ok(Verdict::Continue);
        }
        ctx.state.task.body = issue.body;
        self.gh.add_label(number, labels::SPEC_WRITTEN).await?;
        ctx.state.spec_written = true;
        Ok(Verdict::Continue)
    }
}

/// Marks the task as delivered when a merged PR proves it.
///
/// The "write" half of the old `task_is_delivered`, and the last thing a
/// round does.
///
/// **Delivered is not closed, and the difference is intentional.** `Closes #N`
/// closes nothing here: GitHub closes a linked issue only on a merge to the
/// **default** branch, and the loop merges to the integration branch. Rather
/// than close on its behalf — which would say "integrated in `main`" of work
/// that isn't — the round places `harness:waiting-merge`. The issue stays open,
/// will never be picked again, and doesn't block the next one; it's the human
/// that closes it by merging.
///
/// Without proof, it marks nothing and stays silent: it's the gate that says
/// why the round stops.
pub struct MarkWaitingMerge {
    /// What reads the issue and PRs, and places the label.
    pub gh: Rc<dyn GitHub>,
    /// The branch on which proof is sought.
    pub integration_branch: String,
}

#[async_trait(?Send)]
impl Action<Loop> for MarkWaitingMerge {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || !ctx.state.has_task() {
            return Ok(Verdict::Continue);
        }
        let number = number_of(ctx)?;
        let here = self.gh.issue(number).await?;
        // Already closed by GitHub — the day the integration branch becomes
        // the default branch — or already marked: nothing to re-place.
        if here.is_closed() || tasks::waiting_merge(&here) {
            return Ok(Verdict::Continue);
        }
        let merged = self.gh.merged_prs(&self.integration_branch).await?;
        let Some(shipped) = tasks::first_closing(&merged, number) else {
            return Ok(Verdict::Continue);
        };
        ctx.traces.say(&format!(
            "#{number} delivered by PR {}, merged on {} — marked {}, now you \
             close it by merging to the default branch",
            shipped.reference(),
            self.integration_branch,
            labels::WAITING_MERGE
        ));
        self.gh.add_label(number, labels::WAITING_MERGE).await?;
        Ok(Verdict::Continue)
    }
}

/// A command sent in the stage's open session.
///
/// The only action that charges, and thus the only one that records a line in
/// the registry. It composes the prompt — command, preamble, instructions,
/// scope —, sends it, re-reads the two markers, and translates what comes back.
///
/// Multiple `Ask` in the same stage is the normal case: on the Python side,
/// `StageSpec.lead` was used to stitch `/tech-analyst` and `/code` into a
/// single prompt because a stage could only speak once.
pub struct Ask {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The command that opens the prompt (`/code`, `/tech-analyst`, …).
    pub lead: String,
    /// Instructions specific to this stage. Empty: preamble and scope are
    /// enough, as for `/create-test`.
    pub instructions: String,
    /// Inject the SCOPE block.
    ///
    /// False for `/planner` and it alone: it doesn't work on any task, it
    /// opens one. Injecting an empty ISSUE block would give it a task to find.
    pub scoped: bool,
    /// The round number, for the `round` column of the registry.
    pub round: u32,
    /// The integration branch cited in the preamble.
    pub branch: String,
    /// The name cited as the injector.
    pub injector: &'static str,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<Loop> for Ask {
    async fn run(&self, open: &mut Open<'_, Loop>) -> Outcome<Verdict> {
        let scope = open.state.scope();
        let extra = prompts::extra_for(
            &self.instructions,
            if self.scoped { Some(&scope) } else { None },
            self.injector,
        );
        let prompt = prompts::build(&self.lead, &self.branch, &extra, self.injector);
        let task = open.state.task_key.clone();
        ask_and_record(
            open,
            &prompt,
            &self.stage,
            self.round,
            &task,
            self.spending.as_ref(),
        )
        .await?;
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::dev_loop::data::state::Loop;
    use harness_core::adapters::agent::{Reply, Session};
    use harness_core::adapters::store::spending::Entry;
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::traces::{Logbook, Sink, Verbosity};
    use std::cell::RefCell;

    fn ctx(state: Loop) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            state,
            Logbook::null(),
        )
    }

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("task {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn milestone(number: u64) -> Issue {
        issue(number, &[labels::MILESTONE])
    }

    fn task(number: u64) -> Issue {
        issue(number, &[labels::AGENT, labels::READY])
    }

    // --- PickTask ----------------------------------------------------------

    #[tokio::test]
    async fn picking_fills_the_scope_so_no_session_starts_blind() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![task(11)])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        PickTask { gh, resuming: None }
            .run(&mut context)
            .await
            .expect("a task");
        assert_eq!(context.state.milestone.number, "4");
        assert_eq!(context.state.task.number, "11");
        assert_eq!(context.state.task_key, "11");
        assert_eq!(context.state.kind, "auto");
        assert!(!context.state.rollover);
    }

    #[tokio::test]
    async fn the_resume_key_wins_over_what_the_board_would_offer() {
        // Real case: /code merged, the issue is closed, and /create-test
        // is left to do. Nothing would offer it.
        let mut shipped = task(11);
        shipped.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![shipped, task(12)])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        PickTask {
            gh,
            resuming: Some("11".to_string()),
        }
        .run(&mut context)
        .await
        .expect("une task");
        assert_eq!(context.state.task_key, "11");
        assert!(context.state.resumed);
    }

    #[tokio::test]
    async fn no_open_task_at_all_is_a_rollover() {
        let mut done = task(11);
        done.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![done])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        PickTask { gh, resuming: None }
            .run(&mut context)
            .await
            .expect("un rollover");
        assert!(context.state.rollover);
        assert!(!context.state.has_task());
    }

    #[tokio::test]
    async fn open_but_unplayable_tasks_halt_rather_than_roll_over() {
        // Mixing the two charges an opus /planner for a `harness:ready`
        // box no one checked.
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![issue(11, &[labels::AGENT])])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        let err = PickTask { gh, resuming: None }
            .run(&mut context)
            .await
            .expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(!context.state.rollover, "especially not a rollover");
        assert!(err.reason().contains(labels::READY));
    }

    #[tokio::test]
    async fn an_unreadable_board_never_becomes_a_rollover() {
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Unreadable("expired token".to_string())),
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        let err = PickTask { gh, resuming: None }
            .run(&mut context)
            .await
            .expect_err("must fail");
        assert!(matches!(err, Halt::Unreadable(_)));
        assert!(!context.state.rollover);
    }

    // --- RecordSpecWritten -------------------------------------------------

    fn with_task(number: &str) -> Loop {
        Loop {
            task: Named {
                number: number.to_string(),
                title: "a task".to_string(),
                body: String::new(),
            },
            task_key: number.to_string(),
            ..Loop::default()
        }
    }

    #[tokio::test]
    async fn recording_the_spec_copies_the_reread_body_into_the_scope() {
        let mut written = issue(11, &[labels::AGENT]);
        written.body = "the SPEC, written by /business-analyst".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![written],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        RecordSpecWritten { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("recorded");
        assert!(context.state.task.body.contains("the SPEC"));
        assert!(context.state.spec_written);
        assert_eq!(
            gh.writes(),
            vec![Wrote::Label(11, labels::SPEC_WRITTEN.to_string())]
        );
    }

    #[tokio::test]
    async fn an_empty_body_is_not_labelled_as_written() {
        // Placing the label would skip the write on the next run when
        // nothing was written. It's the gate that stops, not this one.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        RecordSpecWritten { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("nothing to record");
        assert!(!context.state.spec_written);
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing_to_github() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        context.settings.dry_run = true;
        RecordSpecWritten { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("nothing");
        assert!(gh.writes().is_empty());
    }

    // --- MarkWaitingMerge --------------------------------------------------

    fn delivered_by(pr: u64, closes: u64) -> Issue {
        Issue {
            number: pr,
            body: format!("Closes #{closes}"),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_merged_pr_marks_the_task_waiting_merge_without_closing_it() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT])],
            merged: vec![delivered_by(99, 11)],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("marked");
        // Labeled, never closed: closing would say "integrated in main".
        assert_eq!(
            gh.writes(),
            vec![Wrote::Label(11, labels::WAITING_MERGE.to_string())]
        );
    }

    #[tokio::test]
    async fn without_proof_it_marks_nothing_and_leaves_the_gate_to_speak() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("nothing to mark");
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn an_already_marked_task_is_not_relabelled() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT, labels::WAITING_MERGE])],
            merged: vec![delivered_by(99, 11)],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("already marked");
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn a_rollover_round_has_no_task_to_mark() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(Loop {
            rollover: true,
            ..Loop::default()
        });
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("nothing to mark");
        assert!(gh.writes().is_empty());
    }

    // --- Ask ---------------------------------------------------------------

    struct Scripted {
        reply: Result<Reply, Halt>,
        seen: RefCell<Vec<String>>,
    }

    #[async_trait(?Send)]
    impl Session for Scripted {
        async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
            self.seen.borrow_mut().push(prompt.to_string());
            self.reply.clone()
        }
    }

    #[derive(Default)]
    struct Recorded(RefCell<Vec<(String, String, f64)>>);

    impl Spending for Recorded {
        fn record(&self, entry: &Entry<'_>) -> Outcome<()> {
            self.0.borrow_mut().push((
                entry.stage.to_string(),
                entry.outcome.to_string(),
                entry.spend.cost_usd.unwrap_or(-1.0),
            ));
            Ok(())
        }
    }

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    fn answered(text: &str) -> Reply {
        Reply {
            text: text.to_string(),
            stop_line: None,
            spend: Spend {
                cost_usd: Some(0.42),
                ..Spend::default()
            },
        }
    }

    fn scoped_state() -> Loop {
        Loop {
            milestone: Named {
                number: "4".to_string(),
                title: "The chat".to_string(),
                body: "what the milestone says".to_string(),
            },
            task: Named {
                number: "11".to_string(),
                title: "The grid".to_string(),
                body: "the SPEC".to_string(),
            },
            task_key: "11".to_string(),
            ..Loop::default()
        }
    }

    struct Asked {
        verdict: Outcome<Verdict>,
        prompts: Vec<String>,
        rows: Vec<(String, String, f64)>,
        said: String,
    }

    async fn ask_with(ask_scoped: bool, reply: Result<Reply, Halt>, dry_run: bool) -> Asked {
        let spending = Rc::new(Recorded::default());
        let capture = Rc::new(Capture::default());
        let mut context = Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            scoped_state(),
            Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal),
        );
        let mut session = Scripted {
            reply,
            seen: RefCell::new(Vec::new()),
        };
        let ask = Ask {
            stage: "code".to_string(),
            lead: "/tech-analyst".to_string(),
            instructions: "the task is #{num} (\"{title}\")".to_string(),
            scoped: ask_scoped,
            round: 3,
            branch: "main_agent".to_string(),
            injector: "test",
            spending: Rc::clone(&spending) as Rc<dyn Spending>,
        };
        let verdict = {
            let mut open = Open {
                ctx: &mut context,
                session: &mut session,
            };
            ask.run(&mut open).await
        };
        Asked {
            verdict,
            prompts: session.seen.borrow().clone(),
            rows: spending.0.borrow().clone(),
            said: capture.0.borrow().join("\n"),
        }
    }

    #[tokio::test]
    async fn the_prompt_carries_the_command_the_preamble_and_the_scope() {
        let run = ask_with(true, Ok(answered("AGENT_LOOP_OK: delivered")), false).await;
        let prompt = &run.prompts[0];
        assert!(prompt.starts_with("/tech-analyst\n"));
        assert!(prompt.contains("main_agent"), "the integration branch");
        assert!(prompt.contains("the task is #11 (\"The grid\")"));
        assert!(prompt.contains("ISSUE #11 — The grid"));
        assert!(prompt.contains("MILESTONE #4"));
    }

    #[tokio::test]
    async fn an_unscoped_ask_gets_no_issue_block_to_hunt_for() {
        let run = ask_with(false, Ok(answered("AGENT_LOOP_OK: planned")), false).await;
        assert!(!run.prompts[0].contains("ISSUE #"));
    }

    #[tokio::test]
    async fn the_ok_line_is_echoed_into_the_journal_under_the_stage_tag() {
        let run = ask_with(true, Ok(answered("AGENT_LOOP_OK: delivered")), false).await;
        assert!(run.said.contains("[code] AGENT_LOOP_OK: delivered"));
    }

    #[tokio::test]
    async fn a_missing_ok_marker_is_said_rather_than_treated_as_a_failure() {
        let run = ask_with(true, Ok(answered("I'm done")), false).await;
        assert!(run.verdict.is_ok());
        assert!(run.said.contains("no AGENT_LOOP_OK marker"));
    }

    #[tokio::test]
    async fn a_stop_marker_halts_and_keeps_the_sessions_own_reason() {
        let run = ask_with(
            true,
            Ok(answered(
                "AGENT_LOOP_STOP: the SPEC requires a paid service",
            )),
            false,
        )
        .await;
        let err = run.verdict.expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(err.reason().contains("paid service"));
        assert!(err.reason().contains("(/code)"), "name the stage");
    }

    #[tokio::test]
    async fn a_successful_ask_records_what_it_cost() {
        let run = ask_with(true, Ok(answered("AGENT_LOOP_OK: delivered")), false).await;
        assert_eq!(run.rows, vec![("code".to_string(), "ok".to_string(), 0.42)]);
    }

    #[tokio::test]
    async fn a_dead_stage_still_gets_its_line_and_it_says_nothing_was_observed() {
        // It's the dead stage whose traces we want. And a zero line would
        // be read as a free session: -1.0 is the witness of `None`.
        let run = ask_with(true, Err(Halt::Quota("window exhausted".into())), false).await;
        assert!(matches!(run.verdict, Err(Halt::Quota(_))));
        assert_eq!(
            run.rows,
            vec![("code".to_string(), "QUOTA".to_string(), -1.0)]
        );
    }

    #[tokio::test]
    async fn a_dry_run_records_no_spend_because_none_was_made() {
        let run = ask_with(true, Ok(answered("")), true).await;
        assert!(run.rows.is_empty());
        // And it doesn't ask for a marker from a session nobody opened.
        assert!(!run.said.contains("no AGENT_LOOP_OK marker"));
    }
}
