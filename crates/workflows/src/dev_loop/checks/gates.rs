//! What a stage of the round requires before paying, and what it must obtain.
//!
//! Don't confuse with the **workflow** gates, verified once before and after
//! the run. These only make sense in the round.
//!
//! # The two hybrids, split
//!
//! Decision #1 says a `Verification` judges and doesn't write — and the borrow
//! checker holds that. Yet two Python guards **did write**:
//!
//! | Python | what it did | becomes here |
//! | --- | --- | --- |
//! | `spec_is_in_the_issue` | re-read, place `spec-written`, mutate state | gone: `split` writes the SPEC |
//! | `task_is_delivered` | re-read, place `waiting-merge` | [`AMergedPrClosesTheTask`] + `MarkWaitingMerge` |
//!
//! The actions live in `actions.rs`: that's the point of the split.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};
use harness_core::ports::shell::github::GitHub;

use crate::common::labels;
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::data::tasks;
use harness_architecture::vocabulary::{Declaration, Unit};

/// The session builds on the SPEC: without its goal and its acceptance
/// criteria it would plan a guess.
///
/// `split` writes them in every task it opens; a refinement a human asked
/// for writes them too (`harness:spec-written`). A task opened by hand
/// without them stops here, and says what to write — never which label to
/// ask for: nothing waits on a human gesture but the body itself.
///
/// A task whose `## Architecture` declares a unit an id names
/// (`Unit::has_an_id`) and nothing it `produces` stops here too, before a
/// session is paid: the checkout is narrowed to the closure of what it
/// produces, and the round would halt later on `ArchitectureHolds`, on
/// its own unit's files (`ForgeDesMondes`' #574 and #507). Never in
/// silence — tasks opened before split required it are still open: when
/// its own words name the id unambiguously ([`derived_id`]: its `id:`
/// line, its aggregate's name, or the one `<system>.<name>` its title
/// names), `produces:` (and `id:`) are written into its `## Architecture`
/// and the next round reads them; otherwise what to write is said once in
/// a comment on the task ([`NO_PRODUCES`]), for whoever edits its body.
pub struct TaskHasASpec {
    /// What writes the backfilled body, or the comment.
    pub gh: Rc<dyn GitHub>,
}

/// The marker of the comment [`TaskHasASpec`] posts once on a unit task
/// that names nothing it `produces` and whose id it cannot tell.
pub const NO_PRODUCES: &str = "<!-- harness-no-produces -->";

/// The id a unit task that names nothing it `produces` builds, when its
/// own words say it unambiguously — and where it was read: the `id:` line
/// of its `## Architecture` (`architecture`), its aggregate's name for an
/// aggregate, or, for a Capability, a `DataCapability` or a persisted
/// query, the one `<system>.<name>` of its own system its `title` names.
/// `None` when nothing, or more than one id, is named.
fn derived_id(
    title: &str,
    architecture: &str,
    declared: &Declaration,
) -> Option<(String, &'static str)> {
    if let Some(id) = architecture
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("id:"))
        .map(str::trim)
        .filter(|id| !id.is_empty() && *id != "-")
    {
        return Some((id.to_string(), "its `id:` line"));
    }
    if declared.unit == Unit::Aggregate
        && let Some(name) = declared
            .aggregate
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
    {
        return Some((name.to_string(), "its `aggregate:` line"));
    }
    if !matches!(
        declared.unit,
        Unit::Capability | Unit::DataCapability | Unit::PersistedQuery
    ) || declared.system.trim().is_empty()
    {
        return None;
    }
    let prefix = format!("{}.", declared.system.trim());
    let mut named: Vec<&str> = title
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'))
        .map(|word| word.trim_end_matches('.'))
        .filter(|word| {
            word.strip_prefix(&prefix).is_some_and(|name| {
                name.starts_with(|c: char| c.is_ascii_lowercase())
                    && name.chars().all(|c| c.is_ascii_alphanumeric())
            })
        })
        .collect();
    named.dedup();
    match named.as_slice() {
        [id] => Some(((*id).to_string(), "its title")),
        _ => None,
    }
}

/// `body` with `produces: <id>` at the end of its `## Architecture`, and
/// `id: <id>` under its `unit:` line when it has none.
fn with_produces(body: &str, architecture: &str, id: &str) -> String {
    let named = architecture
        .lines()
        .any(|line| line.trim_start().starts_with("id:"));
    let written = if named {
        architecture.trim_end().to_string()
    } else {
        crate::dev_loop::data::missing::with_id(architecture.trim_end(), id)
    };
    crate::common::sections::replace(
        body,
        &[(
            crate::common::sections::ARCHITECTURE,
            &format!("{written}\nproduces: {id}"),
        )],
    )
}

#[async_trait(?Send)]
impl Verification<Loop> for TaskHasASpec {
    fn purpose(&self) -> String {
        "the issue body carries its SPEC (Business Goal, Acceptance Criteria) before it is built"
            .to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let task = &ctx.state.task;
        let sections = crate::common::sections::parse(&task.body);
        if let Some(architecture) = sections.get(crate::common::sections::ARCHITECTURE)
            && let Some(declared) = Declaration::parse(architecture)
            && declared.unit.has_an_id()
            && declared.produces.is_empty()
        {
            let number = task.number.parse::<u64>().ok();
            if let (Some(number), Some((id, source))) =
                (number, derived_id(&task.title, architecture, &declared))
            {
                self.gh
                    .set_body(number, &with_produces(&task.body, architecture, &id))
                    .await?;
                return Err(Halt::Halted(format!(
                    "issue #{number} declared a `{}` under `## Architecture` with nothing it \
                     `produces`: `produces: {id}` written there, read from {source} — the next \
                     round reads it",
                    declared.unit.key()
                )));
            }
            let reason = format!(
                "issue #{} declares a `{}` under `## Architecture` but names nothing it \
                 `produces` — write `produces: <id>` (and `id: <id>` under `unit:`), the id it \
                 builds or evolves as its manifest names it: the checkout is narrowed to what the \
                 task produces, and without it the unit's own folder is left out",
                task.number,
                declared.unit.key()
            );
            if let Some(number) = number
                && !self
                    .gh
                    .issue_comments(number)
                    .await?
                    .iter()
                    .any(|said| said.contains(NO_PRODUCES))
            {
                self.gh
                    .post_issue_comment(number, &format!("{NO_PRODUCES}\n{reason}"))
                    .await?;
            }
            return Err(Halt::Halted(reason));
        }
        if ctx.state.spec_written {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "issue #{} carries no SPEC — write its `## Business Goal` and \
             `## Acceptance Criteria` in the body, and the loop takes it on the next tick",
            ctx.state.task.number
        )))
    }
}

/// The scope of the `code` stage: without an issue body, there's no SPEC.
pub struct CodeHasASpec;

#[async_trait(?Send)]
impl Verification<Loop> for CodeHasASpec {
    fn purpose(&self) -> String {
        "the issue body is not empty — there is a SPEC to build".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || !ctx.state.task.body.trim().is_empty() {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "issue #{} has an empty body — there is no SPEC to build from. Run \
             business refinement on it first (label {}).",
            ctx.state.task.number,
            labels::REFINEMENT
        )))
    }
}

/// Has this `/code`'s PR already merged? — we don't re-charge it.
///
/// **Only on a resumed round.** On a new round, `/code` never ran, and
/// searching for the PR would cost an API call per round for an answer
/// known in advance.
///
/// The same proof [`AMergedPrClosesTheTask`] requires, placed *before*
/// charging rather than after: it's the only guard that says "it's already
/// done" rather than "this won't work".
pub struct CodeAlreadyDelivered {
    /// What reads the issue and PRs.
    pub gh: Rc<dyn GitHub>,
    /// The branch on which proof is sought.
    pub integration_branch: String,
    /// `--restart` replays the stage even if proof is there.
    pub restart: bool,
}

#[async_trait(?Send)]
impl Verification<Loop> for CodeAlreadyDelivered {
    fn purpose(&self) -> String {
        "on a resumed round, no merged pull request already closes this task — /code is not paid for twice".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if !ctx.state.resumed || ctx.settings.dry_run || self.restart {
            return Ok(Verdict::Continue);
        }
        let number: u64 = ctx.state.task.number.parse().map_err(|_| {
            Halt::Failed(format!(
                "unreadable task number: {:?}",
                ctx.state.task.number
            ))
        })?;
        let here = self.gh.issue(number).await?;
        // `review-pending` counts: the write side's PR is open and a human
        // merges it; paying `/code` again would open a second one.
        let shipped = here.is_closed()
            || tasks::waiting_merge(&here)
            || tasks::review_pending(&here)
            || tasks::first_closing(&self.gh.merged_prs(&self.integration_branch).await?, number)
                .is_some();
        if !shipped {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!(
            "#{number} is already delivered on {} — /code skips rather than \
             being paid again (--restart to replay it)",
            self.integration_branch
        )))
    }
}

/// Does a merged PR carry `Closes #N` — the "judge" half of the old `task_is_delivered`.
///
/// What we require is not a stage's assertion, but a **merged** PR declaring it.
/// Without this check, a run that left `/code` out (`--stages`) would be read
/// as a task delivered.
pub struct AMergedPrClosesTheTask {
    /// What reads the issue and PRs.
    pub gh: Rc<dyn GitHub>,
    /// The branch on which proof is sought.
    pub integration_branch: String,
    /// True if `--stages` left `code` out — changes the message, not the rule.
    pub code_runs: bool,
    /// What `--stages` was, to say it in the message.
    pub stages: String,
}

#[async_trait(?Send)]
impl Verification<Loop> for AMergedPrClosesTheTask {
    fn purpose(&self) -> String {
        "a merged pull request on the integration branch carries `Closes #N` for this task"
            .to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        // Handed over — a decision asked, or a task opened for a human — is
        // what a round that stopped could achieve; a PR left standing by a
        // killed session is what it delivered.
        if ctx.settings.dry_run
            || !ctx.state.has_task()
            || ctx.state.handed_over
            || ctx.state.pr_stands
        {
            return Ok(Verdict::Continue);
        }
        let number: u64 = ctx.state.task.number.parse().map_err(|_| {
            Halt::Failed(format!(
                "unreadable task number: {:?}",
                ctx.state.task.number
            ))
        })?;
        // Already closed or marked: proof is there, nothing to reassert.
        let here = self.gh.issue(number).await?;
        if here.is_closed() || tasks::waiting_merge(&here) {
            return Ok(Verdict::Continue);
        }
        // An open PR in the harness's own cycle is a delivery too: the write
        // side's, or a killed session's on either side — the round marked it
        // `review-pending`, and that is what it had to achieve.
        if tasks::review_pending(&here) {
            return Ok(Verdict::Continue);
        }
        let merged = self.gh.merged_prs(&self.integration_branch).await?;
        if tasks::first_closing(&merged, number).is_some() {
            return Ok(Verdict::Continue);
        }
        let left_out = if self.code_runs {
            String::new()
        } else {
            format!(
                " --stages ({}) left /code out, and nothing else delivers a \
                 task.",
                self.stages
            )
        };
        Err(Halt::Halted(format!(
            "issue #{number} ended the round and nothing marks it delivered — \
             no merged PR on {} carries `Closes #{number}` on its own \
             line.{left_out} Stopping rather than looping on the same task.",
            self.integration_branch
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::{Issue, Named};
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

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

    fn with_task(number: &str, body: &str) -> Loop {
        Loop {
            task: Named {
                number: number.to_string(),
                title: "a task".to_string(),
                body: body.to_string(),
            },
            task_key: number.to_string(),
            ..Loop::default()
        }
    }

    fn issue(number: u64, labels: &[&str], body: &str) -> Issue {
        Issue {
            number,
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: body.to_string(),
            ..Issue::default()
        }
    }

    /// The SPEC gate, writing to `gh`.
    fn spec_gate(gh: &Rc<FakeGitHub>) -> TaskHasASpec {
        TaskHasASpec {
            gh: Rc::clone(gh) as Rc<dyn GitHub>,
        }
    }

    #[tokio::test]
    async fn the_technical_stage_refuses_a_task_without_a_business_spec() {
        let gh = Rc::new(FakeGitHub::default());
        let err = spec_gate(&gh)
            .verify(&ctx(with_task("34", "raw request")))
            .await
            .expect_err("must stop");
        assert!(err.reason().contains("## Acceptance Criteria"));
        assert!(
            !err.reason().contains("label"),
            "no gesture asked of a human"
        );
        let mut state = with_task("34", "raw request");
        state.spec_written = true;
        assert_eq!(
            spec_gate(&gh).verify(&ctx(state)).await.expect("a verdict"),
            Verdict::Continue
        );
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    }

    /// `ForgeDesMondes` #574's `## Architecture`: a `DataCapability`
    /// evolving `campaign.exportCampaign`, no `produces`.
    const EVOLVING: &str = "## Architecture\n\nunit: data-capability\nsystem: campaign\n\
                            effect: update\ntouches: Campaign.archive\nside: harness:write-side\n";

    /// A task with a whole SPEC, titled `title`, its body `body`.
    fn spec_written(title: &str, body: &str) -> Loop {
        let mut state = with_task("574", body);
        state.task.title = title.to_string();
        state.spec_written = true;
        state
    }

    #[tokio::test]
    async fn a_unit_task_that_produces_nothing_and_names_no_id_is_told_once_and_stops() {
        let gh = Rc::new(FakeGitHub::default());
        let err = spec_gate(&gh)
            .verify(&ctx(spec_written("Export media and sessions", EVOLVING)))
            .await
            .expect_err("must stop");
        let Halt::Halted(said) = err else {
            panic!("a voluntary stop");
        };
        assert!(
            said.contains("issue #574 declares a `data-capability`")
                && said.contains("`produces: <id>`"),
            "{said}"
        );
        // Said on the task, once: never a silent halt at every retry.
        assert_eq!(
            gh.writes(),
            [Wrote::Comment(574, format!("{NO_PRODUCES}\n{said}"))]
        );
        let told = Rc::new(FakeGitHub {
            issue_comments: vec![format!("{NO_PRODUCES}\n{said}")],
            ..FakeGitHub::default()
        });
        spec_gate(&told)
            .verify(&ctx(spec_written("Export media and sessions", EVOLVING)))
            .await
            .expect_err("still stops");
        assert!(told.writes().is_empty(), "{:?}", told.writes());
        // Two ids of its system in the title: which one is not its to guess.
        let gh = Rc::new(FakeGitHub::default());
        spec_gate(&gh)
            .verify(&ctx(spec_written(
                "campaign.exportCampaign reads campaign.listScenes",
                EVOLVING,
            )))
            .await
            .expect_err("must stop");
        assert!(
            matches!(gh.writes().as_slice(), [Wrote::Comment(574, _)]),
            "{:?}",
            gh.writes()
        );
    }

    #[tokio::test]
    async fn a_unit_task_whose_words_name_its_id_gets_its_produces_written() {
        // The title names it (#574's own), or its `id:` line, or its
        // aggregate: written into `## Architecture`, the round stops once.
        for (title, body, id, source) in [
            (
                "campaign.exportCampaign carries media, sounds and sessions",
                EVOLVING.to_string(),
                "campaign.exportCampaign",
                "its title",
            ),
            (
                "Export",
                EVOLVING.replace(
                    "unit: data-capability\n",
                    "unit: data-capability\nid: campaign.exportCampaign\n",
                ),
                "campaign.exportCampaign",
                "its `id:` line",
            ),
            (
                "The PJ aggregate",
                "## Architecture\n\nunit: aggregate\nsystem: campagne\naggregate: PJ\n".to_string(),
                "PJ",
                "its `aggregate:` line",
            ),
        ] {
            let gh = Rc::new(FakeGitHub::default());
            let err = spec_gate(&gh)
                .verify(&ctx(spec_written(title, &body)))
                .await
                .expect_err("stops once");
            assert!(
                err.reason().contains(&format!(
                    "`produces: {id}` written there, read from {source}"
                )),
                "{}",
                err.reason()
            );
            let writes = gh.writes();
            let [Wrote::Body(574, written)] = writes.as_slice() else {
                panic!("one body written: {writes:?}");
            };
            let declared = tasks::declaration(&Issue {
                body: written.clone(),
                ..Issue::default()
            })
            .expect("still declared");
            assert_eq!(declared.produces, [id.to_string()], "{written}");
            assert_eq!(
                written.matches(&format!("id: {id}")).count(),
                1,
                "{written}"
            );
            // The next round reads it and goes on.
            assert_eq!(
                spec_gate(&gh)
                    .verify(&ctx(spec_written(title, written)))
                    .await
                    .expect("a verdict"),
                Verdict::Continue
            );
        }
    }

    #[test]
    fn an_id_is_derived_only_from_words_that_name_one() {
        let derive = |title: &str, architecture: &str| {
            let declared = Declaration::parse(architecture).expect("declared");
            derived_id(title, architecture, &declared).map(|(id, _)| id)
        };
        let capability = "unit: capability\nsystem: campaign";
        assert_eq!(
            derive("campaign.listScenes, sorted.", capability).as_deref(),
            Some("campaign.listScenes")
        );
        // An `id:` line saying nothing names nothing.
        for blank in ["id: -", "id:"] {
            assert_eq!(
                derive(
                    "Sort the scenes",
                    &format!("unit: capability\n{blank}\nsystem: campaign")
                ),
                None,
                "{blank}"
            );
        }
        // No system: no prefix to read an id by.
        assert_eq!(derive(".listScenes", "unit: capability\nsystem: "), None);
        // A word that runs on past the id is not the id.
        for title in [
            "campaign.listScenes-v2 rework",
            "campaign.listScenes_v2 rework",
        ] {
            assert_eq!(derive(title, capability), None, "{title}");
        }
        // A name that is no camelCase id: none.
        assert_eq!(derive("campaign.ListScenes", capability), None);
        // Another unit than the three a title may name: none.
        assert_eq!(
            derive("campaign.listScenes", "unit: micro-ui\nsystem: campaign"),
            None
        );
    }

    #[tokio::test]
    async fn a_unit_task_that_names_what_it_produces_or_needs_no_id_goes_on() {
        let gh = Rc::new(FakeGitHub::default());
        let named = EVOLVING.replace("side:", "produces: campaign.exportCampaign\nside:");
        let tooling = "## Architecture\n\nunit: tooling\nsystem: campaign\n";
        for body in [named.as_str(), tooling] {
            assert_eq!(
                spec_gate(&gh)
                    .verify(&ctx(spec_written("t", body)))
                    .await
                    .expect("a verdict"),
                Verdict::Continue
            );
        }
        let mut dry_ctx = ctx(spec_written("t", EVOLVING));
        dry_ctx.settings.dry_run = true;
        assert_eq!(
            spec_gate(&gh).verify(&dry_ctx).await.expect("a verdict"),
            Verdict::Continue
        );
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    }

    #[tokio::test]
    async fn code_refuses_to_improvise_on_an_empty_spec() {
        let err = CodeHasASpec
            .verify(&ctx(with_task("34", "   ")))
            .await
            .expect_err("must stop");
        let Halt::Halted(said) = err else {
            panic!("a voluntary stop");
        };
        assert!(said.contains("no SPEC to build from"));
    }

    #[tokio::test]
    async fn a_dry_run_lets_code_through_without_a_spec() {
        let mut context = ctx(with_task("34", ""));
        context.settings.dry_run = true;
        assert_eq!(
            CodeHasASpec.verify(&context).await.expect("a verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_fresh_round_never_looks_for_a_merged_pr_of_its_own_code() {
        // One API call per round for an answer known in advance: /code never
        // ran on a fresh round.
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Unreadable("must not be called".to_string())),
            ..FakeGitHub::default()
        });
        let gate = CodeAlreadyDelivered {
            gh,
            integration_branch: "main_agent".to_string(),
            restart: false,
        };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "the SPEC")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_resumed_round_whose_code_already_merged_skips_it() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            merged: vec![Issue {
                number: 99,
                body: "Closes #34".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        let gate = CodeAlreadyDelivered {
            gh,
            integration_branch: "main_agent".to_string(),
            restart: false,
        };
        let mut state = with_task("34", "the SPEC");
        state.resumed = true;
        let Verdict::Skip(why) = gate.verify(&ctx(state)).await.expect("verdict") else {
            panic!("a skip");
        };
        assert!(why.contains("--restart"), "say how to replay");
    }

    #[tokio::test]
    async fn restart_replays_code_even_when_the_proof_is_there() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[labels::WAITING_MERGE], "")],
            ..FakeGitHub::default()
        });
        let gate = CodeAlreadyDelivered {
            gh,
            integration_branch: "main_agent".to_string(),
            restart: true,
        };
        let mut state = with_task("34", "the SPEC");
        state.resumed = true;
        assert_eq!(
            gate.verify(&ctx(state)).await.expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_merged_pr_that_closes_the_task_is_the_proof() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            merged: vec![Issue {
                number: 99,
                body: "Closes #34".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn nothing_marking_delivery_stops_rather_than_looping() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        let err = gate
            .verify(&ctx(with_task("34", "")))
            .await
            .expect_err("must stop");
        let Halt::Halted(said) = err else {
            panic!("a voluntary stop");
        };
        assert!(said.contains("Closes #34"));
        assert!(said.contains("Stopping rather than looping"));
    }

    #[tokio::test]
    async fn leaving_code_out_of_stages_is_named_in_the_message() {
        // Without this, "nothing marks delivery" reads as a bug when `--stages`
        // actually removed the only stage that delivers.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: false,
            stages: "pick".to_string(),
        };
        let err = gate
            .verify(&ctx(with_task("34", "")))
            .await
            .expect_err("must stop");
        assert!(format!("{err}").contains("left /code out"));
    }

    #[tokio::test]
    async fn a_task_handed_over_needs_no_merged_pr() {
        let gh = Rc::new(FakeGitHub::default());
        let gate = AMergedPrClosesTheTask {
            gh: gh.clone(),
            integration_branch: "milestone/3-x".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        let mut state = with_task("34", "");
        state.handed_over = true;
        assert_eq!(
            gate.verify(&ctx(state)).await.expect("handed over"),
            Verdict::Continue
        );
        let seen = &gh.writes();
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[tokio::test]
    async fn a_pr_left_standing_by_a_killed_session_needs_no_merged_pr() {
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Unreadable("must not be called".to_string())),
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "milestone/3-x".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        let mut state = with_task("34", "");
        state.pr_stands = true;
        assert_eq!(
            gate.verify(&ctx(state)).await.expect("the PR stands"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_review_pending_task_is_delivered_on_either_side() {
        // The label is the harness's own mark: a read-side session killed at
        // its PR gets it too, and its round must end as delivered.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[labels::REVIEW_PENDING], "")],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        for write_side in [false, true] {
            let mut state = with_task("34", "");
            state.write_side = write_side;
            assert_eq!(
                gate.verify(&ctx(state)).await.expect("verdict"),
                Verdict::Continue,
                "write_side={write_side}"
            );
        }
    }

    #[tokio::test]
    async fn an_already_marked_task_needs_no_further_proof() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[labels::WAITING_MERGE], "")],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
    }
}
