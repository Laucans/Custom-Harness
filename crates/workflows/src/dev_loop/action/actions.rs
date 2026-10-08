//! What the round **does**: pick, ask, record, mark.
//!
//! The counterpart to `gates.rs`, and the reason the two files exist
//! separately. Decision #1 says a `Verification` judges and doesn't write; so
//! everything that writes is here, including the "write" halves of Python's
//! two hybrid guards:
//!
//! | Python | its judging half | its acting half |
//! | --- | --- | --- |
//! | `spec_is_in_the_issue` | `gates::IssueBodyIsNotEmpty` | [`RecordTechWritten`] |
//! | `task_is_delivered` | `gates::AMergedPrClosesTheTask` | [`MarkWaitingMerge`] |
//!
//! The split costs one issue re-read per pair: the action re-reads to write,
//! the gate re-reads to judge. That's the acknowledged price of "judging is
//! not doing" — a free local call against a session that costs dollars.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Named, Outcome, Scoped, Verdict, prompts};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::common::{delivery, hierarchy, labels, sections};
use crate::dev_loop::data::brief::Cut;
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::data::{board, signatures, tasks};

/// The number of the current task, or failure to read it.
fn number_of(ctx: &Context<Loop>) -> Outcome<u64> {
    ctx.state.task.number.parse().map_err(|_| {
        Halt::Failed(format!(
            "unreadable task number: {:?}",
            ctx.state.task.number
        ))
    })
}

/// Picks the round's task, or finds none.
///
/// The first action of every round, and the only one with the right to find
/// nothing. Its two outputs are what replaced a three-branch router in a
/// graph engine — the third branch, rollover to `/planner`, is gone: that is
/// now a separate workflow, triggered on its own:
///
/// - a task (the resumed one, else the one from the table) → round continues;
/// - no playable task **but open tasks** → stop, naming the unblocking
///   gesture — a `harness:ready` box no one checked, never a reason to stop
///   issuing milestones;
/// - no open tasks at all → leaves `ctx.state.task` empty. The round reads
///   that via `has_task()` and ends with `Verdict::NothingLeft`.
pub struct PickTask {
    /// What reads the board.
    pub gh: Rc<dyn GitHub>,
    /// The task the resume point designates, if there is one.
    ///
    /// It **takes precedence** over the board's choice: a round interrupted
    /// after `/code` merges works on an already-closed issue that nothing else
    /// would offer, and `/create-test` would be lost.
    pub resuming: Option<String>,
    /// The one task this run is for (`--task`), if any: a lane of a parallel
    /// watch. Picked if runnable, refused by name otherwise — never traded
    /// for the board's own choice, which another lane may be running.
    pub wanted: Option<u64>,
    /// What every stage of the round opens with when the task carries
    /// `harness:data-layer` — the strongest model, for the data design.
    pub data_layer: harness_core::ports::agent::SessionSpec,
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
        ctx.state.siblings = here.tasks.iter().map(hierarchy::sibling).collect();
        ctx.state.roadmap = hierarchy::roadmap_of(self.gh.as_ref(), here.milestone.number).await?;
        ctx.traces.say(&format!(
            "milestone {}: {}",
            here.milestone.reference(),
            here.milestone.title
        ));
        let resumed = self.resuming.as_deref().and_then(|key| here.find(key));
        let chosen = match (resumed, self.wanted) {
            (Some(task), _) => Some(task),
            (None, Some(number)) => {
                let Some(task) = here.wanted(number) else {
                    let why = here.tasks.iter().find(|t| t.number == number).map_or_else(
                        || {
                            format!(
                                "#{number} is not a task of milestone {}",
                                here.milestone.reference()
                            )
                        },
                        tasks::why_not,
                    );
                    return Err(Halt::Halted(format!(
                        "asked for #{number}, which cannot run: {why}"
                    )));
                };
                Some(task)
            }
            (None, None) => here.next(),
        };
        let Some(task) = chosen else {
            if !here.open_agents().is_empty() {
                return Err(Halt::Halted(here.stuck()));
            }
            ctx.traces.say(&format!(
                "milestone {} has no open {} sub-issue left",
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
        ctx.state.tech_written = tasks::tech_written(task);
        ctx.state.resumed = resumed.is_some();
        ctx.state.write_side = tasks::is_write_side(task);
        // Set on every pick, cleared otherwise: the next round's task may
        // not be one.
        ctx.session_spec = task
            .has(labels::DATA_LAYER)
            .then(|| self.data_layer.clone());
        ctx.traces.say(&format!(
            "task {}: {} [{}, {}]",
            task.reference(),
            task.title,
            tasks::kind(task),
            if ctx.state.write_side {
                labels::WRITE_SIDE
            } else {
                labels::READ_SIDE
            }
        ));
        if let Some(spec) = &ctx.session_spec {
            ctx.traces.say(&format!(
                "data layer — every session of this round opens on {}/{}",
                spec.model, spec.effort
            ));
        }
        Ok(Verdict::Continue)
    }
}

/// Records that the technical sections are in the issue's body.
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
pub struct RecordTechWritten {
    /// What re-reads the issue and labels it.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<Loop> for RecordTechWritten {
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
        self.gh.add_label(number, labels::TECH_WRITTEN).await?;
        ctx.state.tech_written = true;
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
///
/// It also comments the landing on the issue — see
/// [`delivery::merged_note`](crate::common::delivery::merged_note). The comment
/// goes **before** the label, and the label is what makes this run once: if the
/// comment fails the round halts having written nothing and the next run
/// retries both; if the label fails after the comment landed, the next run
/// posts a second comment — visible noise against no state damage, which is
/// the right way round.
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
            return self.note_review_pending(ctx, &here, number).await;
        };
        // The human merged what the write side had left open: the review
        // is done, and the board says so.
        if tasks::review_pending(&here) {
            self.gh.remove_label(number, labels::REVIEW_PENDING).await?;
            if here.has(labels::NEEDS_DECISION) {
                self.gh.remove_label(number, labels::NEEDS_DECISION).await?;
            }
        }
        // Merged into its milestone is done: the task closes, and closing it
        // is what unblocks the tasks that wait on it. GitHub would not do it
        // — its closing keywords fire only on the default branch.
        ctx.traces.say(&format!(
            "#{number} delivered by PR {}, merged on {} — closed",
            shipped.reference(),
            self.integration_branch,
        ));
        self.gh
            .post_issue_comment(
                number,
                &delivery::merged_note(&self.integration_branch, &shipped.reference()),
            )
            .await?;
        self.gh.close_issue(number).await?;
        Ok(Verdict::Continue)
    }
}

impl MarkWaitingMerge {
    /// The write side's own delivery: no merged PR, but an open one carrying
    /// `harness:to-review` on the task's declared branch. The task is then
    /// marked `review-pending` — the loop stops replaying it, the next task
    /// waits, and `needs-decision` tells the human what is theirs to do.
    ///
    /// Nothing is marked without the PR: a session that stopped before
    /// opening it leaves the round to its postcondition, which names it.
    async fn note_review_pending(
        &self,
        ctx: &Context<Loop>,
        here: &harness_core::domain::Issue,
        number: u64,
    ) -> Outcome<Verdict> {
        if !ctx.state.write_side || tasks::review_pending(here) {
            return Ok(Verdict::Continue);
        }
        let branch = tasks::declared_branch(&here.body);
        let open = self.gh.open_prs_labelled(labels::TO_REVIEW).await?;
        let Some(pr) = open
            .iter()
            .find(|pr| branch.as_deref() == Some(pr.head.as_str()))
        else {
            ctx.traces.say(&format!(
                "#{number} is on the write side and no open PR carrying {} is on its \
                 branch {} yet",
                labels::TO_REVIEW,
                branch.as_deref().unwrap_or("(undeclared)")
            ));
            return Ok(Verdict::Continue);
        };
        ctx.traces.say(&format!(
            "#{number} is on the write side: PR {} is open and waits for a human \
             merge — marked {}",
            pr.reference(),
            labels::REVIEW_PENDING
        ));
        self.gh
            .post_issue_comment(
                number,
                &format!(
                    "PR {} is open on the write side of the architecture: `milestone_merge` \
                     merges it once its review has run and every check is green.",
                    pr.url
                ),
            )
            .await?;
        self.gh.add_label(number, labels::REVIEW_PENDING).await?;
        self.gh.add_label(number, labels::NEEDS_DECISION).await?;
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
    /// The instructions when the task is on the write side, if the stage
    /// has different ones — `code` opens a PR and stops instead of merging.
    pub write_side_instructions: Option<String>,
    /// How much of the brief this stage's prompt carries — see
    /// [`Cut`](crate::dev_loop::data::brief::Cut).
    pub cut: Cut,
    /// The round number, for the `round` column of the registry.
    pub round: u32,
    /// The integration branch cited in the preamble.
    pub branch: String,
    /// The repository's own configuration, verbatim — see
    /// [`crate::dev_loop::data::stack`]. Empty when the stack was not
    /// recognised, and then no block is injected.
    ///
    /// Carried here, not in [`Loop`]: the state is serialized into the resume
    /// point, and a digest of config files has no business growing that file.
    /// It is read once per run and never changes within it.
    pub stack: String,
    /// The public shape of every indexed file in the checkout — see
    /// [`crate::dev_loop::data::signatures`].
    ///
    /// The whole index, not this task's slice: which files matter depends on the
    /// task, and the task is picked inside the round. Built once per run because
    /// its TypeScript half costs a `tsc` subprocess; the selection below is pure.
    pub signatures: Rc<signatures::Index>,
    /// The name cited as the injector.
    pub injector: &'static str,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<Loop> for Ask {
    async fn run(&self, open: &mut Open<'_, Loop>) -> Outcome<Verdict> {
        // The **whole** body decides the index slice, before the cut: a file
        // named only in a section this stage does not receive is still a file
        // the stage will touch, and its signature costs less than opening it.
        let carried =
            signatures::carried(&self.signatures, &open.state.task.body, signatures::BUDGET);
        let mut scope = open.state.scope();
        scope.task.body = sections::without(&scope.task.body, self.cut.without());
        let brief = match self.cut {
            Cut::Unscoped => prompts::Brief::Unscoped,
            Cut::Situated(_) => prompts::Brief::Situated(&scope),
            Cut::TaskOnly(_) => prompts::Brief::TaskOnly(&scope),
        };
        let instructions = match (&self.write_side_instructions, open.state.write_side) {
            (Some(text), true) => text.as_str(),
            _ => self.instructions.as_str(),
        };
        let extra = prompts::extra_for(instructions, brief, &self.stack, &carried, self.injector);
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
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::{Reply, Session};
    use harness_core::ports::store::spending::Entry;
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

    fn opus_high() -> harness_core::ports::agent::SessionSpec {
        harness_core::ports::agent::SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        }
    }

    #[tokio::test]
    async fn a_data_layer_task_opens_every_session_on_the_strongest_model() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(
                4,
                vec![issue(
                    11,
                    &[labels::AGENT, labels::READY, labels::DATA_LAYER],
                )],
            )],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        PickTask {
            gh,
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
        }
        .run(&mut context)
        .await
        .expect("picked");
        assert_eq!(context.session_spec, Some(opus_high()));
    }

    #[tokio::test]
    async fn a_plain_task_leaves_every_stage_on_its_own_model() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![task(11)])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        context.session_spec = Some(opus_high());
        PickTask {
            gh,
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
        }
        .run(&mut context)
        .await
        .expect("picked");
        assert_eq!(context.session_spec, None, "cleared on every pick");
    }

    #[tokio::test]
    async fn picking_fills_the_scope_so_no_session_starts_blind() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![task(11)])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        PickTask {
            gh,
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
        }
        .run(&mut context)
        .await
        .expect("a task");
        assert_eq!(context.state.milestone.number, "4");
        assert_eq!(context.state.task.number, "11");
        assert_eq!(context.state.task_key, "11");
        assert_eq!(context.state.kind, "auto");
        assert!(context.state.has_task());
    }

    #[tokio::test]
    async fn picking_also_loads_the_roadmap_and_the_sibling_titles() {
        let mut roadmap = issue(2, &[labels::ROADMAP]);
        roadmap.body = "the plan".to_string();
        let mut shipped = task(10);
        shipped.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![roadmap, milestone(4)],
            subs: vec![(2, vec![milestone(4)]), (4, vec![shipped, task(11)])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        PickTask {
            gh,
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
        }
        .run(&mut context)
        .await
        .expect("a task");
        let found = context.state.roadmap.expect("the roadmap");
        assert_eq!(
            (found.number.as_str(), found.body.as_str()),
            ("2", "the plan")
        );
        let seen: Vec<_> = context
            .state
            .siblings
            .iter()
            .map(|s| (s.number.as_str(), s.status.as_str()))
            .collect();
        assert_eq!(seen, [("10", "done"), ("11", "open")]);
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
            wanted: None,
            data_layer: opus_high(),
        }
        .run(&mut context)
        .await
        .expect("une task");
        assert_eq!(context.state.task_key, "11");
        assert!(context.state.resumed);
    }

    #[tokio::test]
    async fn no_open_task_at_all_leaves_the_task_empty() {
        let mut done = task(11);
        done.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![done])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        PickTask {
            gh,
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
        }
        .run(&mut context)
        .await
        .expect("nothing to pick, not a failure");
        assert!(!context.state.has_task());
    }

    #[tokio::test]
    async fn open_but_unplayable_tasks_halt_rather_than_say_nothing_is_left() {
        // Mistaking the two would leave a `harness:ready` box no one
        // checked read as "the milestone is finished".
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![issue(11, &[labels::AGENT])])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        let err = PickTask {
            gh,
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
        }
        .run(&mut context)
        .await
        .expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(!context.state.has_task(), "especially not picked");
        assert!(err.reason().contains(labels::READY));
    }

    #[tokio::test]
    async fn an_unreadable_board_is_a_failure_not_an_empty_milestone() {
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Unreadable("expired token".to_string())),
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        let err = PickTask {
            gh,
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
        }
        .run(&mut context)
        .await
        .expect_err("must fail");
        assert!(matches!(err, Halt::Unreadable(_)));
        assert!(!context.state.has_task());
    }

    // --- RecordTechWritten -------------------------------------------------

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
    async fn recording_the_technical_sections_copies_the_reread_body_into_the_scope() {
        let mut written = issue(11, &[labels::AGENT]);
        written.body = "the SPEC, with its technical sections".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![written],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        RecordTechWritten { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("recorded");
        assert!(context.state.task.body.contains("the SPEC"));
        assert!(context.state.tech_written);
        assert_eq!(
            gh.writes(),
            vec![Wrote::Label(11, labels::TECH_WRITTEN.to_string())]
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
        RecordTechWritten { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("nothing to record");
        assert!(!context.state.tech_written);
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing_to_github() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        context.settings.dry_run = true;
        RecordTechWritten { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("nothing");
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
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
        // Commented, then closed: merged into its milestone is done, and the
        // close is what unblocks the tasks waiting on it.
        assert_eq!(
            gh.writes(),
            vec![
                Wrote::Comment(11, delivery::merged_note("main_agent", "#99")),
                Wrote::Closed(11),
            ]
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
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn a_write_side_task_with_an_open_reviewed_pr_is_marked_review_pending() {
        let mut task = issue(11, &[labels::AGENT, labels::WRITE_SIDE]);
        task.body = "## Scope\n\nbranch: feat/limit\n\nraise limits".to_string();
        let pr = harness_core::domain::Pr {
            num: "77".to_string(),
            head: "feat/limit".to_string(),
            url: "https://github.com/o/r/pull/77".to_string(),
            labels: vec![labels::TO_REVIEW.to_string()],
            ..harness_core::domain::Pr::default()
        };
        let gh = Rc::new(FakeGitHub {
            issues: vec![task],
            prs_labelled: std::collections::HashMap::from([(
                labels::TO_REVIEW.to_string(),
                vec![pr],
            )]),
            ..FakeGitHub::default()
        });
        let mut state = with_task("11");
        state.write_side = true;
        let mut context = ctx(state);
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("marked");
        let writes = gh.writes();
        assert!(matches!(&writes[0], Wrote::Comment(11, text) if text.contains("pull/77")));
        assert_eq!(
            writes[1],
            Wrote::Label(11, labels::REVIEW_PENDING.to_string())
        );
        assert_eq!(
            writes[2],
            Wrote::Label(11, labels::NEEDS_DECISION.to_string())
        );
        assert_eq!(writes.len(), 3, "never waiting-merge: nothing merged yet");
    }

    #[tokio::test]
    async fn a_write_side_task_without_its_pr_yet_is_left_to_the_gate() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT, labels::WRITE_SIDE])],
            prs_labelled: std::collections::HashMap::from([(
                labels::TO_REVIEW.to_string(),
                Vec::new(),
            )]),
            ..FakeGitHub::default()
        });
        let mut state = with_task("11");
        state.write_side = true;
        let mut context = ctx(state);
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("nothing to mark");
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn the_merge_of_a_write_side_pr_drops_review_pending_and_closes_the_task() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(
                11,
                &[
                    labels::AGENT,
                    labels::WRITE_SIDE,
                    labels::REVIEW_PENDING,
                    labels::NEEDS_DECISION,
                ],
            )],
            merged: vec![delivered_by(77, 11)],
            ..FakeGitHub::default()
        });
        let mut state = with_task("11");
        state.write_side = true;
        let mut context = ctx(state);
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("marked");
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::Unlabelled(11, labels::REVIEW_PENDING.to_string())));
        assert!(writes.contains(&Wrote::Unlabelled(11, labels::NEEDS_DECISION.to_string())));
        assert!(writes.contains(&Wrote::Closed(11)));
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
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn a_round_with_no_task_picked_has_nothing_to_mark() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(Loop::default());
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("nothing to mark");
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
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
        with_body("the SPEC")
    }

    fn with_body(body: &str) -> Loop {
        Loop {
            milestone: Named {
                number: "4".to_string(),
                title: "The chat".to_string(),
                body: "what the milestone says".to_string(),
            },
            siblings: vec![harness_core::domain::Sibling {
                number: "12".to_string(),
                title: "The fence".to_string(),
                status: "open".to_string(),
                gist: "covers the fence".to_string(),
            }],
            task: Named {
                number: "11".to_string(),
                title: "The grid".to_string(),
                body: body.to_string(),
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

    const NOTHING: &[&str] = &[];

    async fn ask_with(cut: Cut, reply: Result<Reply, Halt>, dry_run: bool) -> Asked {
        ask_carrying(cut, reply, dry_run, String::new()).await
    }

    async fn ask_carrying(
        cut: Cut,
        reply: Result<Reply, Halt>,
        dry_run: bool,
        stack: String,
    ) -> Asked {
        ask_about(scoped_state(), cut, reply, dry_run, stack).await
    }

    async fn ask_about(
        state: Loop,
        cut: Cut,
        reply: Result<Reply, Halt>,
        dry_run: bool,
        stack: String,
    ) -> Asked {
        let spending = Rc::new(Recorded::default());
        let capture = Rc::new(Capture::default());
        let mut context = Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            state,
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
            write_side_instructions: None,
            cut,
            round: 3,
            branch: "main_agent".to_string(),
            stack,
            signatures: Rc::new(signatures::Index::default()),
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
        let run = ask_with(
            Cut::Situated(NOTHING),
            Ok(answered("AGENT_LOOP_OK: delivered")),
            false,
        )
        .await;
        let prompt = &run.prompts[0];
        assert!(prompt.starts_with("/tech-analyst\n"));
        assert!(prompt.contains("main_agent"), "the integration branch");
        assert!(prompt.contains("the task is #11 (\"The grid\")"));
        assert!(prompt.contains("ISSUE #11 — The grid"));
        assert!(prompt.contains("MILESTONE #4"));
    }

    // --- what each stage's prompt leaves out -------------------------------

    const REFINED: &str = "## Business Goal\n\nthe why\n\n\
                           ## Acceptance Criteria\n\n- it works\n\n\
                           ## Technical\n\nthe design\n\n\
                           ## Technical Implementation Plan\n\n1. do it\n\n\
                           ## Assumptions (autonomous run)\n\nguessed\n";

    #[tokio::test]
    async fn a_stage_does_not_receive_the_sections_it_is_about_to_write() {
        // `technical-refinement`'s own cut: 26 167 characters of #65's body were
        // the two sections and the assumptions the stage exists to produce.
        let run = ask_about(
            with_body(REFINED),
            Cut::Situated(&["Technical", "Technical Implementation Plan", "Assumptions"]),
            Ok(answered("AGENT_LOOP_OK: written")),
            false,
            String::new(),
        )
        .await;
        let prompt = &run.prompts[0];
        assert!(prompt.contains("the why"), "{prompt}");
        assert!(prompt.contains("- it works"));
        assert!(!prompt.contains("the design"), "{prompt}");
        assert!(!prompt.contains("1. do it"), "{prompt}");
        assert!(!prompt.contains("guessed"), "{prompt}");
        // Still situated: where the task sits is half of what it decides.
        assert!(prompt.contains("MILESTONE #4"));
    }

    #[tokio::test]
    async fn a_stage_working_against_a_written_spec_gets_the_task_alone() {
        // `/create-test`: the hierarchy was 19 397 of #65's 60 363 SCOPE
        // characters, re-read on every turn to say what the plan had settled.
        let run = ask_about(
            with_body(REFINED),
            Cut::TaskOnly(&["Business Goal", "Technical", "Assumptions"]),
            Ok(answered("AGENT_LOOP_OK: tested")),
            false,
            String::new(),
        )
        .await;
        let prompt = &run.prompts[0];
        assert!(prompt.contains("- it works"), "the criteria it asserts");
        assert!(prompt.contains("1. do it"), "and the plan's bullets");
        assert!(!prompt.contains("the design"), "{prompt}");
        assert!(!prompt.contains("the why"), "{prompt}");
        assert!(!prompt.contains("MILESTONE #4 —"), "{prompt}");
        assert!(!prompt.contains("covers the fence"), "{prompt}");
        // The way back, so the cut is a default rather than a removal.
        assert!(prompt.contains("gh issue view"), "{prompt}");
    }

    #[tokio::test]
    async fn the_index_slice_is_chosen_from_the_whole_body_not_the_cut_one() {
        // A file named only in a section this stage does not receive is still a
        // file the stage will touch, and a signature costs less than opening it.
        let mut index = signatures::Index {
            ecosystem: signatures::Ecosystem::TypeScript,
            ..signatures::Index::default()
        };
        index.tracked.push("src/core/place.ts".to_string());
        index.by_path.insert(
            "src/core/place.ts".to_string(),
            "export declare function place(): void;".to_string(),
        );
        let state =
            with_body("## Business Goal\n\nthe why\n\n## Technical\n\nextend src/core/place.ts\n");
        let spending = Rc::new(Recorded::default());
        let mut context = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            state,
            Logbook::null(),
        );
        let mut session = Scripted {
            reply: Ok(answered("AGENT_LOOP_OK: ok")),
            seen: RefCell::new(Vec::new()),
        };
        let ask = Ask {
            stage: "create-test".to_string(),
            lead: "/create-test".to_string(),
            instructions: String::new(),
            write_side_instructions: None,
            cut: Cut::TaskOnly(&["Technical"]),
            round: 1,
            branch: "main_agent".to_string(),
            stack: String::new(),
            signatures: Rc::new(index),
            injector: "test",
            spending: Rc::clone(&spending) as Rc<dyn Spending>,
        };
        {
            let mut open = Open {
                ctx: &mut context,
                session: &mut session,
            };
            ask.run(&mut open).await.expect("asked");
        }
        let prompt = &session.seen.borrow()[0];
        assert!(
            !prompt.contains("extend src/core/place.ts"),
            "cut: {prompt}"
        );
        assert!(prompt.contains("export declare function place"), "{prompt}");
    }

    #[tokio::test]
    async fn an_unscoped_ask_gets_no_issue_block_to_hunt_for() {
        let run = ask_with(Cut::Unscoped, Ok(answered("AGENT_LOOP_OK: planned")), false).await;
        assert!(!run.prompts[0].contains("ISSUE #"));
    }

    #[tokio::test]
    async fn the_repository_configuration_reaches_the_prompt_verbatim() {
        let run = ask_carrying(
            Cut::Situated(NOTHING),
            Ok(answered("AGENT_LOOP_OK: delivered")),
            false,
            "<file path=\"package.json\">{ \"test\": \"vitest run\" }</file>".to_string(),
        )
        .await;
        let prompt = &run.prompts[0];
        assert!(prompt.contains("--- REPOSITORY CONFIGURATION"), "{prompt}");
        // Byte for byte: the exact command is the whole point of carrying it.
        assert!(prompt.contains("{ \"test\": \"vitest run\" }"), "{prompt}");
    }

    #[tokio::test]
    async fn an_unrecognised_stack_adds_nothing_to_the_prompt() {
        let run = ask_with(
            Cut::Situated(NOTHING),
            Ok(answered("AGENT_LOOP_OK: delivered")),
            false,
        )
        .await;
        assert!(!run.prompts[0].contains("REPOSITORY CONFIGURATION"));
    }

    #[tokio::test]
    async fn the_ok_line_is_echoed_into_the_journal_under_the_stage_tag() {
        let run = ask_with(
            Cut::Situated(NOTHING),
            Ok(answered("AGENT_LOOP_OK: delivered")),
            false,
        )
        .await;
        assert!(run.said.contains("[code] AGENT_LOOP_OK: delivered"));
    }

    #[tokio::test]
    async fn a_missing_ok_marker_is_said_rather_than_treated_as_a_failure() {
        let run = ask_with(Cut::Situated(NOTHING), Ok(answered("I'm done")), false).await;
        assert!(run.verdict.is_ok());
        assert!(run.said.contains("no AGENT_LOOP_OK marker"));
    }

    #[tokio::test]
    async fn a_stop_marker_halts_and_keeps_the_sessions_own_reason() {
        let run = ask_with(
            Cut::Situated(NOTHING),
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
        let run = ask_with(
            Cut::Situated(NOTHING),
            Ok(answered("AGENT_LOOP_OK: delivered")),
            false,
        )
        .await;
        assert_eq!(run.rows, vec![("code".to_string(), "ok".to_string(), 0.42)]);
    }

    #[tokio::test]
    async fn a_dead_stage_still_gets_its_line_and_it_says_nothing_was_observed() {
        // It's the dead stage whose traces we want. And a zero line would
        // be read as a free session: -1.0 is the witness of `None`.
        let run = ask_with(
            Cut::Situated(NOTHING),
            Err(Halt::Quota("window exhausted".into())),
            false,
        )
        .await;
        assert!(matches!(run.verdict, Err(Halt::Quota(_))));
        assert_eq!(
            run.rows,
            vec![("code".to_string(), "QUOTA".to_string(), -1.0)]
        );
    }

    #[tokio::test]
    async fn a_dry_run_records_no_spend_because_none_was_made() {
        let run = ask_with(Cut::Situated(NOTHING), Ok(answered("")), true).await;
        assert_eq!(
            run.rows,
            [] as [(std::string::String, std::string::String, f64); 0]
        );
        // And it doesn't ask for a marker from a session nobody opened.
        assert!(!run.said.contains("no AGENT_LOOP_OK marker"));
    }
}
