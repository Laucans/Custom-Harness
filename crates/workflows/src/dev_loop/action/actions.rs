//! What the round **does**: pick, ask, record, mark.
//!
//! The counterpart to `gates.rs`, and the reason the two files exist
//! separately. Decision #1 says a `Verification` judges and doesn't write; so
//! everything that writes is here, including the "write" halves of Python's
//! two hybrid guards:
//!
//! | Python | its judging half | its acting half |
//! | --- | --- | --- |
//! | `spec_is_in_the_issue` | gone: `split` writes the SPEC | gone |
//! | `task_is_delivered` | `gates::AMergedPrClosesTheTask` | [`MarkWaitingMerge`] |
//!
//! The split costs one issue re-read per pair: the action re-reads to write,
//! the gate re-reads to judge. That's the acknowledged price of "judging is
//! not doing" — a free local call against a session that costs dollars.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Named, Outcome, Scoped, Verdict, milestone_label, prompts};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::git::Repo;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;
use harness_core::traces::Event;

use crate::common::{delivery, hierarchy, labels, sections};
use crate::dev_loop::data::brief::{self, Cut};
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::data::{acceptance, board, dependency_api, signatures, tasks};
use harness_architecture::rules;
use harness_architecture::vocabulary::Declaration;

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
    /// The branch this run works on: a milestone's own names the board to
    /// read, anything else reads the lowest open milestone's.
    pub branch: String,
    /// What remembers a stage a previous run left interrupted on the task,
    /// so this run resumes its session rather than paying the turns again.
    pub spending: Rc<dyn Spending>,
    /// The lane's checkout: its head when the task is picked is what the
    /// round's changes are measured from.
    pub repo: Rc<dyn Repo>,
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
    /// The relay a light session of the task's system left, taken once the
    /// task is known (`chain`); `None` where no register is wired.
    pub chain: Option<crate::dev_loop::action::chain::TakeRelay>,
    /// What every stage of the round opens with when the task carries
    /// `harness:data-layer` — the strongest model, for the data design.
    pub data_layer: harness_core::ports::agent::SessionSpec,
}

/// The stages whose interrupted session is worth resuming: the paid ones.
const RESUMABLE: &[&str] = &["technical-refinement", "code", "create-test"];

#[async_trait(?Send)]
impl Action<Loop> for PickTask {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        // One board read per round. The launcher read the resume pointer, not
        // the board: so there aren't two reads that could give different answers.
        let here = board::read_on(self.gh.as_ref(), &self.branch).await?;
        ctx.state.milestone = Named {
            number: here.milestone.number.to_string(),
            title: here.milestone.title.clone(),
            body: here.milestone.body.clone(),
        };
        ctx.state.siblings = here.tasks.iter().map(hierarchy::sibling).collect();
        ctx.state.delivered = here
            .tasks
            .iter()
            .flat_map(hierarchy::delivered_ids)
            .collect();
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
        ctx.state.resumed = resumed.is_some();
        ctx.state.write_side = tasks::is_write_side(task);
        ctx.tags = tasks::tags(task);
        // The board read above names the milestone: the ledger keeps it on
        // every row of the round, so its cost survives the milestone closing.
        ctx.tags.milestone = milestone_label(here.milestone.number, &here.milestone.title);
        ctx.state.round_base = self.repo.head_sha().await.unwrap_or_default();
        // The sessions a quota cut on this task: one per stage, resumed by
        // the stage runner when it opens that stage.
        ctx.resume.clear();
        for stage in RESUMABLE {
            if let Some(id) = self.spending.interrupted(&ctx.state.task_key, stage)? {
                ctx.traces.say(&format!(
                    "task {}: stage {stage} was cut short last time — its session {id} will be resumed",
                    task.reference()
                ));
                ctx.resume.insert((*stage).to_string(), id);
            }
        }
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
        if let Some(chain) = &self.chain {
            chain.run(ctx).await?;
        }
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
        // What the write side had left open is merged: the review is done,
        // and the board says so.
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
        ctx.traces.event(&Event::TaskDelivered {
            task: number,
            pr: shipped.reference(),
            branch: self.integration_branch.clone(),
        });
        Ok(Verdict::Continue)
    }
}

impl MarkWaitingMerge {
    /// The delivery of a PR left open in the harness's cycle.
    ///
    /// No merged PR, but an open one carrying `harness:to-review` on the
    /// task's declared branch — the write side's own way, or a session of
    /// either side killed at its PR ([`Self::pr_stands`]). The task is then
    /// marked `review-pending` — the loop stops replaying it, the next task
    /// waits, and `milestone_merge` merges it once its review is clean and
    /// every check green. Nothing is asked of a human: they are told.
    ///
    /// Nothing is marked without the PR: a session that stopped before
    /// opening it leaves the round to its postcondition, which names it.
    async fn note_review_pending(
        &self,
        ctx: &Context<Loop>,
        here: &harness_core::domain::Issue,
        number: u64,
    ) -> Outcome<Verdict> {
        if tasks::review_pending(here) {
            return Ok(Verdict::Continue);
        }
        let branch = tasks::declared_branch(&here.body);
        let open = self.gh.open_prs_labelled(labels::TO_REVIEW).await?;
        let Some(pr) = open
            .iter()
            .find(|pr| branch.as_deref() == Some(pr.head.as_str()))
        else {
            // Both sides end at a PR carrying the review label: none left
            // open means the session stopped before it, and the
            // postcondition says what is missing.
            if ctx.state.write_side {
                ctx.traces.say(&format!(
                    "#{number} is on the write side and no open PR carrying {} is on its \
                     branch {} yet",
                    labels::TO_REVIEW,
                    branch.as_deref().unwrap_or("(undeclared)")
                ));
            }
            return Ok(Verdict::Continue);
        };
        self.mark_review_pending(ctx, number, pr).await?;
        Ok(Verdict::Continue)
    }

    /// Marks the task `review-pending` for `pr`, says so on the issue and
    /// in the journal, and records [`Event::PrOpened`].
    async fn mark_review_pending(
        &self,
        ctx: &Context<Loop>,
        number: u64,
        pr: &harness_core::domain::Pr,
    ) -> Outcome<()> {
        ctx.traces.say(&format!(
            "#{number}: PR {} is open, merged once reviewed and green — marked {}",
            pr.reference(),
            labels::REVIEW_PENDING
        ));
        self.gh
            .post_issue_comment(
                number,
                &format!(
                    "PR {} is open: `milestone_merge` merges it once its review has run \
                     and every check is green.",
                    pr.url
                ),
            )
            .await?;
        self.gh.add_label(number, labels::REVIEW_PENDING).await?;
        ctx.traces.event(&Event::PrOpened {
            task: number,
            pr: pr.reference(),
            base: pr.base.clone(),
        });
        Ok(())
    }

    /// A session killed at its open pull request delivered that PR.
    ///
    /// Killed for its silence ([`Halt::killed_as_hung`]) while its PR was
    /// already open: the PR is the delivery, not a failed round. Three
    /// write-side sessions pushed their PR, waited on its CI through a
    /// background watcher, and were killed twenty minutes later; each round
    /// was filed `FAILED`, the task never marked, and the scheduler sent a
    /// second session onto a branch the PR's own review and repair were
    /// working.
    ///
    /// The PR is handed to the harness's own cycle: labelled
    /// `harness:to-review` when it is not yet (a read-side session merges
    /// its own PR and labels nothing), the task marked `review-pending`,
    /// `pr_stands` set so the postcondition accepts the round. Whether it
    /// stood: `false` for any other halt, for a task with no declared
    /// branch, and for a branch with no open PR — the failure it was.
    ///
    /// # Errors
    /// The GitHub writes' — the label on the PR, the comment and label on
    /// the task.
    pub async fn pr_stands(&self, ctx: &mut Context<Loop>, halt: &Halt) -> Outcome<bool> {
        if !halt.killed_as_hung() || ctx.settings.dry_run || !ctx.state.has_task() {
            return Ok(false);
        }
        let number = number_of(ctx)?;
        let Some(branch) = tasks::declared_branch(&ctx.state.task.body) else {
            return Ok(false);
        };
        // No PR on the branch is among the failures `pr` answers with: the
        // session died before its push, and the round failed as it says.
        let Ok(pr) = self.gh.pr(&branch).await else {
            return Ok(false);
        };
        if !pr.is_open() {
            return Ok(false);
        }
        ctx.traces.warn(&format!(
            "#{number}: the session was killed while waiting on PR {}'s checks ({}) — the PR \
             stands on {branch}: the harness watches its CI from here, and merges it once \
             reviewed and green",
            pr.reference(),
            halt.reason()
        ));
        if !pr.has(labels::TO_REVIEW) {
            let pr_number = pr
                .num
                .parse::<u64>()
                .map_err(|_| Halt::Failed(format!("{:?} is not a PR number", pr.num)))?;
            self.gh.add_label(pr_number, labels::TO_REVIEW).await?;
        }
        let here = self.gh.issue(number).await?;
        if !tasks::review_pending(&here) {
            self.mark_review_pending(ctx, number, &pr).await?;
        }
        ctx.state.pr_stands = true;
        Ok(true)
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
    /// Whether the prompt carries the rule card of the unit the task builds
    /// ([`rules::card`]) — the stages that plan or build it do, the one that
    /// tests it does not.
    pub rules: bool,
}

#[async_trait(?Send)]
impl SessionAction<Loop> for Ask {
    async fn run(&self, open: &mut Open<'_, Loop>) -> Outcome<Verdict> {
        // The **whole** body decides the index slice, before the cut: a file
        // named only in a section this stage does not receive is still a file
        // the stage will touch, and its signature costs less than opening it.
        let files =
            signatures::carried(&self.signatures, &open.state.task.body, signatures::BUDGET);
        // The engine's API, for the tasks that build on it: every write-side
        // task, and every task that names it — in the same block, at the
        // same place, cached with the rest.
        let api = if dependency_api::carries(&open.state.task.body, open.state.write_side) {
            dependency_api::carried_dependencies(
                &self.signatures.dependencies,
                dependency_api::DEPENDENCY_BUDGET,
            )
        } else {
            String::new()
        };
        let carried = [files, api]
            .into_iter()
            .filter(|block| !block.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        // The declaration is read before the cut, like the index slice: the
        // card and the related neighbours come from `## Architecture`, which
        // no cut drops, but the reduction below needs it whole.
        let declaration = sections::parse(&open.state.task.body)
            .get(sections::ARCHITECTURE)
            .and_then(|text| Declaration::parse(text));
        let card = match (&declaration, self.rules) {
            (Some(declaration), true) => rules::card(declaration.unit),
            _ => "",
        };
        let mut scope = open.state.scope();
        scope.task.body = sections::without(&scope.task.body, self.cut.without());
        // Numbered here as the gate after the session counts them: `AC3`
        // is the same criterion at both ends.
        scope.task.body = acceptance::numbered_body(&scope.task.body);
        let criteria = acceptance::of_body(&scope.task.body).len();
        let scope = match self.cut {
            Cut::Executing(_) => brief::executing(scope, declaration.as_ref()),
            _ => scope,
        };
        let brief = match self.cut {
            Cut::Unscoped => prompts::Brief::Unscoped,
            Cut::Situated(_) => prompts::Brief::Situated(&scope),
            Cut::TaskOnly(_) => prompts::Brief::TaskOnly(&scope),
            Cut::Executing(_) => prompts::Brief::Executing(&scope),
        };
        let instructions = match (&self.write_side_instructions, open.state.write_side) {
            (Some(text), true) => text.as_str(),
            _ => self.instructions.as_str(),
        };
        let with_skeleton;
        let instructions = if open.state.skeleton.is_empty() {
            instructions
        } else {
            with_skeleton = format!("{instructions}\n\n{}", skeleton_block(&open.state.skeleton));
            with_skeleton.as_str()
        };
        let with_criteria;
        let instructions = if criteria == 0 {
            instructions
        } else {
            with_criteria = format!("{instructions}\n\n{}", criteria_block(criteria));
            with_criteria.as_str()
        };
        let extra = prompts::extra_for(
            instructions,
            brief,
            card,
            &open.state.tree_note,
            &self.stack,
            &carried,
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

/// What the prompt says of the unit's skeleton the harness wrote before
/// the session opened (`Scaffold`): where it is, and that it is the shape to
/// keep — never a neighbour's.
fn skeleton_block(files: &[String]) -> String {
    let marker = harness_architecture::skeleton::MARKER;
    format!(
        "--- SKELETON ---\n\
The harness wrote this unit's skeleton before you started, uncommitted in \
this checkout (your `git checkout -b` carries it to your branch):\n- {}\n\
Start from it. Its names, layout, registrations, manifest constants and \
test frames are the product's canonical shape: keep them. Fill every \
`{marker}` hole (`grep -rn \"{marker}\"` on these files) from this SPEC, the \
unit's rule card, its aggregate's aggregate.json, data/model.json, \
crates/data/migrations/ and mock/fixtures/<id>.json — none may be left when \
you open the PR, the harness checks after you. Never open another unit's \
folder or an archive branch to learn the structure: the skeleton is the \
structure, and the test helpers are crates/platform/test-kit's \
(platform_test_kit). Replace each `scaffold_*` test by the real tests of \
the acceptance criteria.",
        files.join("\n- ")
    )
}

/// What the prompt says of the task's acceptance criteria, numbered
/// `AC1..AC<count>` in its SCOPE: one test names each, the PR body points
/// to the existing test that already proves one, or says why none can —
/// the harness checks after the session.
fn criteria_block(count: usize) -> String {
    format!(
        "--- ACCEPTANCE CRITERIA ---\n\
The SPEC's acceptance criteria are numbered AC1..AC{count} under SCOPE. Each \
one is proved by a test, and the test that proves it carries a comment \
naming it, on the test or just above: `// AC2: <the criterion, short>` (`#` \
where the language comments so; one test may name several, `// AC2, AC3: \
…`). Only a name your change adds counts: one an earlier task left in a \
file proves nothing. A criterion an existing test already proves — or a \
rule under `tests/rules/` — is pointed to in the PR body, one line each: \
`AC4: {} <path>[::<test>] — <why>`; that test is never edited, nor \
duplicated. A criterion no test can prove here is declared in the PR body, \
one line each, with the reason: `AC3: {} — <why>`. After your session the \
harness reads the pull request: a criterion none of these covers fails the \
round and sends the PR to a repair.",
        acceptance::COVERED,
        acceptance::UNTESTABLE
    )
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
            branch: String::new(),
            spending: Rc::new(Recorded::default()),
            repo: test_repo(),
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
            chain: None,
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
            branch: String::new(),
            spending: Rc::new(Recorded::default()),
            repo: test_repo(),
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
            chain: None,
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
            branch: String::new(),
            spending: Rc::new(Recorded::default()),
            repo: test_repo(),
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
            chain: None,
        }
        .run(&mut context)
        .await
        .expect("a task");
        assert_eq!(context.state.milestone.number, "4");
        assert_eq!(context.state.task.number, "11");
        assert_eq!(context.state.task_key, "11");
        assert_eq!(context.state.kind, "auto");
        assert!(context.state.has_task());
        assert_eq!(
            context.tags.milestone, "#4 task 4",
            "every row of the round names the milestone it is spent on"
        );
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
            branch: String::new(),
            spending: Rc::new(Recorded::default()),
            repo: test_repo(),
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
            chain: None,
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
            branch: String::new(),
            spending: Rc::new(Recorded::default()),
            repo: test_repo(),
            resuming: Some("11".to_string()),
            wanted: None,
            data_layer: opus_high(),
            chain: None,
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
            branch: String::new(),
            spending: Rc::new(Recorded::default()),
            repo: test_repo(),
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
            chain: None,
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
            branch: String::new(),
            spending: Rc::new(Recorded::default()),
            repo: test_repo(),
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
            chain: None,
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
            branch: String::new(),
            spending: Rc::new(Recorded::default()),
            repo: test_repo(),
            resuming: None,
            wanted: None,
            data_layer: opus_high(),
            chain: None,
        }
        .run(&mut context)
        .await
        .expect_err("must fail");
        assert!(matches!(err, Halt::Unreadable(_)));
        assert!(!context.state.has_task());
    }

    // --- a task's state ------------------------------------------------------

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
            writes.len(),
            2,
            "never waiting-merge, never a decision asked of a human: {writes:?}"
        );
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
    async fn a_pr_stands_only_behind_a_killed_session_with_a_task_outside_a_dry_run() {
        // Each guard refuses on its own, the PR open on the declared branch
        // every time: any other halt is the failure it was, a dry run writes
        // nothing, and no task means nothing to mark.
        let killed = || {
            Halt::Failed(format!(
                "claude printed nothing for 1200s and {}",
                harness_core::domain::KILLED_AS_HUNG
            ))
        };
        let on_branch = || {
            let mut state = with_task("11");
            state.task.body = "## Scope\n\nbranch: feat/limit\n".to_string();
            state
        };
        let refused: [(&str, Halt, bool, Loop); 4] = [
            (
                "a stop, not a kill",
                Halt::Halted("AGENT_LOOP_STOP: wait for #20".to_string()),
                false,
                on_branch(),
            ),
            (
                "a plain failure, not a kill",
                Halt::Failed("claude exited with code 1".to_string()),
                false,
                on_branch(),
            ),
            ("a dry run", killed(), true, on_branch()),
            ("no task", killed(), false, Loop::default()),
        ];
        for (why, halt, dry_run, state) in refused {
            let gh = Rc::new(FakeGitHub {
                issues: vec![issue(11, &[labels::AGENT])],
                prs: vec![(
                    "feat/limit".to_string(),
                    harness_core::domain::Pr {
                        num: "77".to_string(),
                        head: "feat/limit".to_string(),
                        url: "https://github.com/o/r/pull/77".to_string(),
                        state: "OPEN".to_string(),
                        ..harness_core::domain::Pr::default()
                    },
                )],
                ..FakeGitHub::default()
            });
            let mut context = Context::new(
                Settings {
                    dry_run,
                    stages: String::new(),
                },
                state,
                Logbook::null(),
            );
            let stood = MarkWaitingMerge {
                gh: gh.clone(),
                integration_branch: "main_agent".to_string(),
            }
            .pr_stands(&mut context, &halt)
            .await
            .expect(why);
            assert!(!stood, "{why}");
            assert!(!context.state.pr_stands, "{why}");
            assert_eq!(gh.writes(), [] as [Wrote; 0], "{why}");
        }
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
                ids: vec!["credit.fence".to_string()],
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
            rules: true,
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

    fn test_repo() -> Rc<dyn harness_core::ports::shell::git::Repo> {
        use harness_core::ports::shell::git::Repos as _;
        crate::common::fake_git::FakeRepos::default().at(std::path::Path::new("/ws"))
    }

    const DECLARED: &str = "## Architecture\n\nunit: capability\nsystem: credit\n\
                            produces: credit.calculateRisk\n\n## Technical Implementation Plan\n\n1. build it\n";

    #[tokio::test]
    async fn a_declared_task_gets_the_rule_card_of_its_unit() {
        let mut state = scoped_state();
        state.task.body = DECLARED.to_string();
        let run = ask_about(
            state,
            Cut::Situated(NOTHING),
            Ok(answered("AGENT_LOOP_OK: ok")),
            false,
            String::new(),
        )
        .await;
        let prompt = &run.prompts[0];
        assert!(prompt.contains("--- ARCHITECTURE RULES"), "{prompt}");
        assert!(prompt.contains("UNIT: Capability"), "{prompt}");
    }

    #[tokio::test]
    async fn an_undeclared_task_gets_no_rule_card() {
        let run = ask_with(
            Cut::Situated(NOTHING),
            Ok(answered("AGENT_LOOP_OK: ok")),
            false,
        )
        .await;
        assert!(
            !run.prompts[0].contains("ARCHITECTURE RULES"),
            "{}",
            run.prompts[0]
        );
    }

    #[tokio::test]
    async fn an_executing_cut_drops_the_roadmap_and_keeps_the_task() {
        let mut state = scoped_state();
        state.task.body = DECLARED.to_string();
        state.roadmap = Some(Named {
            number: "5".to_string(),
            title: "the plan".to_string(),
            body: "a long story nobody executes".to_string(),
        });
        let run = ask_about(
            state,
            Cut::Executing(NOTHING),
            Ok(answered("AGENT_LOOP_OK: ok")),
            false,
            String::new(),
        )
        .await;
        let prompt = &run.prompts[0];
        assert!(!prompt.contains("a long story nobody executes"), "{prompt}");
        assert!(
            prompt.contains("NEIGHBOURS THIS TASK BUILDS ON OR DELIVERS TO"),
            "{prompt}"
        );
        assert!(prompt.contains("1. build it"), "{prompt}");
    }

    #[tokio::test]
    async fn a_narrowed_tree_is_said_in_the_prompt() {
        let mut state = scoped_state();
        state.tree_note = "- /crates/credit/".to_string();
        let run = ask_about(
            state,
            Cut::Situated(NOTHING),
            Ok(answered("AGENT_LOOP_OK: ok")),
            false,
            String::new(),
        )
        .await;
        assert!(
            run.prompts[0].contains("--- WORKING TREE"),
            "{}",
            run.prompts[0]
        );
        assert!(run.prompts[0].contains("- /crates/credit/"));
    }

    #[tokio::test]
    async fn a_skeleton_is_said_in_the_prompt_only_when_one_was_written() {
        let mut state = scoped_state();
        state.skeleton = vec![
            "crates/credit/capabilities/risk/src/lib.rs".to_string(),
            "crates/credit/queries/risk.graphql".to_string(),
        ];
        let run = ask_about(
            state,
            Cut::Situated(NOTHING),
            Ok(answered("AGENT_LOOP_OK: ok")),
            false,
            String::new(),
        )
        .await;
        let prompt = &run.prompts[0];
        assert!(prompt.contains("--- SKELETON ---"), "{prompt}");
        assert!(
            prompt.contains("- crates/credit/queries/risk.graphql"),
            "{prompt}"
        );
        assert!(prompt.contains("`SCAFFOLD:` hole"), "{prompt}");
        assert!(prompt.contains("Never open another unit's"), "{prompt}");
        let bare = ask_about(
            scoped_state(),
            Cut::Situated(NOTHING),
            Ok(answered("AGENT_LOOP_OK: ok")),
            false,
            String::new(),
        )
        .await;
        assert!(!bare.prompts[0].contains("SKELETON"));
    }

    #[tokio::test]
    async fn the_criteria_are_numbered_and_their_tests_asked_for_only_when_there_are_some() {
        let state = with_body(
            "## Business Goal\n\nthe why\n\n## Acceptance Criteria\n\n\
             - given a, when b, then c\n- the count shows\n",
        );
        let run = ask_about(
            state,
            Cut::Executing(NOTHING),
            Ok(answered("AGENT_LOOP_OK: ok")),
            false,
            String::new(),
        )
        .await;
        let prompt = &run.prompts[0];
        assert!(
            prompt.contains("- AC1: given a, when b, then c\n- AC2: the count shows"),
            "{prompt}"
        );
        assert!(prompt.contains("--- ACCEPTANCE CRITERIA ---"), "{prompt}");
        assert!(prompt.contains("numbered AC1..AC2 under SCOPE"), "{prompt}");
        assert!(
            prompt.contains("`// AC2: <the criterion, short>`"),
            "{prompt}"
        );
        assert!(
            prompt.contains("`AC3: not testable here — <why>`"),
            "{prompt}"
        );
        assert!(
            prompt.contains("`AC4: covered by <path>[::<test>] — <why>`"),
            "{prompt}"
        );
        assert!(prompt.contains("that test is never edited"), "{prompt}");
        assert!(
            prompt.contains("Only a name your change adds counts"),
            "{prompt}"
        );
        let bare = ask_about(
            scoped_state(),
            Cut::Executing(NOTHING),
            Ok(answered("AGENT_LOOP_OK: ok")),
            false,
            String::new(),
        )
        .await;
        assert!(!bare.prompts[0].contains("ACCEPTANCE CRITERIA"));
        assert!(!bare.prompts[0].contains("AC1"));
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
        assert!(prompt.contains("- AC1: it works"));
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
        assert!(
            prompt.contains("- AC1: it works"),
            "the criteria it asserts"
        );
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
            rules: false,
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

    /// The prompt an `Ask` sends for `body`, the engine's API in its index.
    async fn prompt_with_the_engine(body: &str, write_side: bool) -> String {
        let index = signatures::Index {
            dependencies: vec![dependency_api::Dependency {
                name: dependency_api::DATAGUARD.to_string(),
                source: "dataguard-v5 @ 56cb6be".to_string(),
                modules: vec![dependency_api::Module {
                    name: "plugins".to_string(),
                    path: "src/plugins.rs".to_string(),
                    shape: "pub trait Prepare: Send + Sync {".to_string(),
                }],
            }],
            ..signatures::Index::default()
        };
        let mut state = with_body(body);
        state.write_side = write_side;
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
            stage: "code".to_string(),
            lead: "/code".to_string(),
            instructions: String::new(),
            write_side_instructions: None,
            cut: Cut::TaskOnly(&[]),
            round: 1,
            branch: "main_agent".to_string(),
            stack: String::new(),
            signatures: Rc::new(index),
            injector: "test",
            spending: Rc::new(Recorded::default()) as Rc<dyn Spending>,
            rules: false,
        };
        {
            let mut open = Open {
                ctx: &mut context,
                session: &mut session,
            };
            ask.run(&mut open).await.expect("asked");
        }
        session.seen.borrow()[0].clone()
    }

    #[tokio::test]
    async fn the_engine_s_api_reaches_the_write_side_and_the_tasks_naming_it() {
        let carried = "crate=\"harness-dataguard\" source=\"dataguard-v5 @ 56cb6be\"";
        let write = prompt_with_the_engine("## Technical\n\na new aggregate\n", true).await;
        assert!(write.contains(carried), "{write}");
        assert!(write.contains("PUBLIC SIGNATURES"), "{write}");
        let named = prompt_with_the_engine("## Technical\n\nthrough the DataGuard\n", false).await;
        assert!(named.contains(carried), "{named}");
        let screen = prompt_with_the_engine("## Technical\n\na screen\n", false).await;
        assert!(!screen.contains("harness-dataguard"), "{screen}");
        assert!(!screen.contains("PUBLIC SIGNATURES"), "{screen}");
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
