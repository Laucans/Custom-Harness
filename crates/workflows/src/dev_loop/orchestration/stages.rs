//! The round sequence, and everything known about each stage.
//!
//! **The only design surface of the workflow.** The order of entries in
//! [`table`] *is* the execution order — the round iterates through this `Vec`, so
//! reordering two lines reorders the round. Adding an entry adds a stage,
//! removing one removes it.
//!
//! This was not always true. The sequence once lived in `@listen` decorators
//! on a graph, and the table was only a registry of settings indexed by skill name:
//! you could invert it entirely without the round changing by an iota, and the
//! test verifying execution order would still pass.
//!
//! An entry carries everything you need to know about a stage without opening
//! another file — **what runs it** (model, effort), **what it says** (instructions),
//! **what skips it**, **what it requires**, and **what it must achieve**.
//!
//! # Labels named in prose
//!
//! Instructions **name** labels — "open an issue with `harness:human`",
//! "the issue will stay under `harness:waiting-merge`". In Python they were
//! hardcoded in the text alongside the constants the code read: renaming
//! `pipeline:*` → `harness:*` would have left prompts asking for labels that
//! no longer exist, and a session would obey by creating new ones. They are
//! therefore **spliced from [`labels`]**, a pass during table assembly. A future
//! rename cannot desynchronize the prompt and code.
//!
//! # Models
//!
//! Distributed by where a wrong answer costs twice. The technical refinement writes
//! the technical half of the SPEC — the issue body — that all following stages read; `code` plans
//! then writes the change itself: an error comes back as rework. `/create-test`
//! writes hermetic Vitest against an already-existing spec, which is not a
//! reasoning problem. Tune to the cost registry, not intuition.
//!
//! There is no archival stage anymore. A task once closed by moving two files;
//! it now closes because `/code`'s PR merged with `Closes #N`.

use std::rc::Rc;

use harness_core::domain::prompts::splice;
use harness_core::execution::{
    Gate, InThisRun, MarkDone, SessionAction, Settings, Stage, StageAlreadyDone, StageBody, Unpaid,
    Verification,
};

use crate::common::labels;
use crate::dev_loop::action::actions::{Ask, RecordTechWritten};
use crate::dev_loop::checks::architecture::{ArchitectureHolds, ConceptsDocumented};
use crate::dev_loop::checks::gates;
use crate::dev_loop::config::Config;
use crate::dev_loop::data::brief::Cut;
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::ports::Ports;

/// What `technical-refinement` leaves out: the three sections it writes.
///
/// 26 167 characters of #65's body, re-read on every turn of the stage whose
/// whole output they are. A previous run's draft also invites the session to
/// edit what is there rather than write the section the gate asks for.
const REFINEMENT_WRITES: &[&str] = &["Technical", "Technical Implementation Plan", "Assumptions"];

/// What `code` leaves out: the business goal.
///
/// 1 557 characters, and the only section `code` cannot act on — it executes a
/// plan that was derived from it, by the stage above. The acceptance criteria
/// and the business rules stay: those it verifies against, bullet by bullet.
///
/// The smallest of the three cuts, and the most arguable: the goal is also the
/// cheapest insurance against a session drifting out of scope. One line to
/// restore if a run ever reads as though it lost the point.
const CODE_SKIPS: &[&str] = &["Business Goal"];

/// What `/create-test` leaves out: everything but the spec it asserts against.
///
/// It writes hermetic tests from the acceptance criteria, the business rules and
/// the plan's verification bullets. The technical design, the goal and another
/// session's assumptions are not things a test asserts — and neither is the
/// hierarchy, which is why this is the one stage on
/// [`Cut::TaskOnly`](crate::dev_loop::data::brief::Cut::TaskOnly).
const TEST_SKIPS: &[&str] = &["Business Goal", "Technical", "Assumptions"];

/// How `code` and `/create-test` run the Rust tests of a repository under the
/// architecture: through the script `init-repo` installs, which the CI runs
/// too. A task that stays inside Capability crates then waits for their
/// tests alone instead of the whole workspace's.
const TEST_SCOPE: &str =
    "Run the Rust tests with `bash scripts/test-scope.sh origin/<the branch your
PR targets>` whenever the repository has that script, never a bare
`cargo test --workspace`: a change that stays inside Capability crates
(`crates/<system>/capabilities/<name>/`) then runs their tests alone, and
anything else the whole workspace — the CI runs the same script on the PR.
Arguments after `--` go to `cargo test`, to run one test while you work.";

const TECHNICAL_REFINEMENT: &str =
    "Take issue #{num} (\"{title}\") — its body is below, under SCOPE, and already
carries its business sections. Read the real code, then write the two missing
sections, `## Technical` and `## Technical Implementation Plan`, into the body
of issue #{num} itself — that body is what the /code stage reads, and nothing
else is. Keep every other section verbatim. No human is reachable: record
every answer you had to assume under an `## Assumptions (autonomous run)`
heading. Anything a human must do first becomes its own `{human}` issue, a
sub-issue of milestone #{milestone}, declared as a dependency of #{num}. If an
assumption would make the task useless or harmful when wrong — a paid service,
a schema decision the later tasks depend on, a credential only the human
holds — that is ambiguity, not a default: AGENT_LOOP_STOP instead of
guessing.";

const CODE: &str = "The task is issue #{num} (\"{title}\"); its body, below under SCOPE, is the
SPEC. Plan it as /tech-analyst: the pre-flight gate, the ordered checklist
against the real code, the stop line, the risks. Then, in this same session,
without waiting for a go-ahead and without /clear, carry out
.claude/skills/code/SKILL.md against your own plan — build, run every
Verification bullet with real output, /code-review, run the architecture
gates of `.github/workflows/gates.yml` locally and fix what fails (the loop
checks the same rules after you), then branch -> PR ->
gh pr merge --squash. The PR body MUST carry the line `Closes #{num}` on its
own: the loop reads that line off the merged PR to confirm the task shipped,
and without it the round stops rather than replay a task nothing marks as
delivered. The issue will stay open under `{waiting_merge}` until a
human merges the integration branch — that is expected, not a failure. Do
not close the issue yourself.
/code's steps 1-3 are what you just did as the tech analyst; adopt your own
findings instead of re-deriving them. Nothing outside this session can read
your plan, so a gate finding or a risk you do not act on now is lost — put it
in your reply.";

/// `code`, on the write side: the same work, and a PR left open for a human.
///
/// The architecture asks a human to review every mutation of existing data,
/// so the session never merges: it opens the PR with `harness:to-review` —
/// the agent review runs on it — and stops. The loop then marks the task
/// `review-pending` and waits for the merge.
const CODE_WRITE_SIDE: &str =
    "The task is issue #{num} (\"{title}\"); its body, below under SCOPE, is the
SPEC. It is on the **write side** of the architecture ({write_side}): it
mutates existing data or the rules that guard it, and a human merges it.
Plan it as /tech-analyst: the pre-flight gate, the ordered checklist against
the real code, the stop line, the risks. Then, in this same session, without
waiting for a go-ahead and without /clear, carry out
.claude/skills/code/SKILL.md against your own plan — build, run every
Verification bullet with real output, /code-review, run the architecture
gates of `.github/workflows/gates.yml` locally and fix what fails (the loop
checks the same rules after you), then branch -> PR, and
STOP THERE: open the pull request against the milestone branch with
`gh pr create --label {to_review}` (the agent review runs on it), wait for
its CI with `gh pr checks`, and do NOT run `gh pr merge` — a write-side PR
is merged by `milestone_merge` once the review has run and every check is
green. The PR body MUST carry the line `Closes #{num}` on its own: the loop
reads that line off the PR to know the task is delivered once it is merged.
The issue will carry `{review_pending}` until then —
that is expected, not a failure. Do not close the issue yourself.
/code's steps 1-3 are what you just did as the tech analyst; adopt your own
findings instead of re-deriving them. Nothing outside this session can read
your plan, so a gate finding or a risk you do not act on now is lost — put it
in your reply.";

/// Instructions for a stage, with labels spliced from [`labels`].
///
/// A single pass, before scope: inserted values are code constants, so
/// without braces, and `{num}`/`{title}`/`{milestone}` remain literal for
/// `prompts::extra_for` to fill in later.
fn with_labels(text: &str) -> String {
    splice(
        text,
        &[
            ("agent", labels::AGENT),
            ("human", labels::HUMAN),
            ("ready", labels::READY),
            ("roadmap", labels::ROADMAP),
            ("milestone_label", labels::MILESTONE),
            ("waiting_merge", labels::WAITING_MERGE),
            ("to_review", labels::TO_REVIEW),
            ("review_pending", labels::REVIEW_PENDING),
            ("write_side", labels::WRITE_SIDE),
        ],
    )
}

/// What every stage undergoes before paying: the filter, and recovery.
fn always(stage: &str) -> Vec<Box<dyn Verification<Loop>>> {
    vec![
        Box::new(InThisRun {
            stage: stage.to_string(),
        }),
        Box::new(StageAlreadyDone {
            stage: stage.to_string(),
        }),
    ]
}

/// What a paid stage demands from its session, and what comes after.
///
/// `then` is local work following the session — re-reading, labeling. It
/// comes **before** `MarkDone`: declaring done before recording would leave,
/// on a run interrupted between the two, a stage marked done with nothing recorded.
struct Paid<'a> {
    stage: &'a str,
    model: &'a str,
    lead: &'a str,
    instructions: &'a str,
    /// Other instructions for a write-side task, if the stage has any.
    write_side_instructions: Option<&'a str>,
    cut: Cut,
    then: Vec<Box<dyn SessionAction<Loop>>>,
}

impl Paid<'_> {
    fn body(self, ports: &Ports, config: &Config, turn: u32) -> StageBody<Loop> {
        let mut actions: Vec<Box<dyn SessionAction<Loop>>> = vec![Box::new(Ask {
            stage: self.stage.to_string(),
            lead: self.lead.to_string(),
            instructions: with_labels(self.instructions),
            write_side_instructions: self.write_side_instructions.map(with_labels),
            cut: self.cut,
            round: turn,
            branch: config.integration_branch.clone(),
            stack: config.stack.clone(),
            signatures: Rc::clone(&config.signatures),
            injector: config.injector(),
            spending: Rc::clone(&ports.spending),
        })];
        actions.extend(self.then);
        actions.push(Box::new(Unpaid(MarkDone {
            stage: self.stage.to_string(),
        })));
        StageBody::Session {
            spec: config.spec(self.model, "high"),
            sessions: Rc::clone(&ports.sessions),
            actions,
        }
    }
}

/// `technical-refinement`: writes the technical sections into the issue body.
///
/// Only runs when the technical refinement was not done on the issue
/// (`harness:tech-written`); the business half must already exist.
#[must_use]
pub fn technical_refinement(ports: &Ports, config: &Config, turn: u32) -> Stage<Loop> {
    let body = Paid {
        stage: "technical-refinement",
        model: "opus",
        lead: "/tech-analyst",
        instructions: TECHNICAL_REFINEMENT,
        write_side_instructions: None,
        cut: Cut::Situated(REFINEMENT_WRITES),
        // Recording is local work, and it goes *in* the stage — not in a
        // gate, which would have no right to write.
        then: vec![Box::new(Unpaid(RecordTechWritten {
            gh: Rc::clone(&ports.gh),
        }))],
    }
    .body(ports, config, turn);
    let mut pre = always("technical-refinement");
    pre.push(Box::new(gates::TechAlreadyWritten));
    pre.push(Box::new(gates::TaskHasABusinessSpec));
    Stage {
        name: "technical-refinement".to_string(),
        pre: Some(Gate {
            name: "technical-refinement requires",
            checks: pre,
        }),
        post: Some(Gate {
            name: "technical-refinement must achieve",
            checks: vec![Box::new(gates::IssueBodyIsNotEmpty {
                gh: Rc::clone(&ports.gh),
            })],
        }),
        body,
    }
}

/// `code`: plans in `/tech-analyst`, then builds and delivers.
///
/// One stage for two skills, one session: `/code` adopts the plan the session
/// just wrote. Nothing outside the session can re-read it, so splitting into
/// two stages would lose it.
#[must_use]
pub fn code(ports: &Ports, config: &Config, turn: u32) -> Stage<Loop> {
    let instructions = format!("{CODE}\n{TEST_SCOPE}");
    let write_side = format!("{CODE_WRITE_SIDE}\n{TEST_SCOPE}");
    let mut pre = always("code");
    pre.push(Box::new(gates::CodeAlreadyDelivered {
        gh: Rc::clone(&ports.gh),
        integration_branch: config.integration_branch.clone(),
        restart: config.restart,
    }));
    pre.push(Box::new(gates::CodeHasASpec));
    Stage {
        name: "code".to_string(),
        pre: Some(Gate {
            name: "code requires",
            checks: pre,
        }),
        // Delivery is the round's postcondition, not this gate's; what the
        // stage must leave behind is a checkout that still follows the
        // architecture, and Concepts every Capability can be checked against.
        post: Some(Gate {
            name: "code must keep the architecture",
            checks: vec![
                Box::new(ArchitectureHolds {
                    disk: Rc::clone(&ports.disk),
                    root: config.root.clone(),
                }),
                Box::new(ConceptsDocumented {
                    disk: Rc::clone(&ports.disk),
                    root: config.root.clone(),
                }),
            ],
        }),
        body: Paid {
            stage: "code",
            // Sonnet, not opus: the tasks reaching this stage are sliced by
            // `split` and specified by the refinement, so `code` executes a
            // decided plan rather than making the decision. Measured on
            // milestone 15, `code` was 62% of the spend while the stages that
            // actually decide — `plan`, `slice` — were cents.
            model: "sonnet",
            lead: "/tech-analyst",
            instructions: &instructions,
            write_side_instructions: Some(&write_side),
            cut: Cut::Situated(CODE_SKIPS),
            then: Vec::new(),
        }
        .body(ports, config, turn),
    }
}

/// `/create-test`: hermetic tests against an already-written spec.
///
/// No gate: it works against an existing spec, and nothing it produces
/// conditions what follows — it's the final stage. Its one instruction is how
/// to run the tests ([`TEST_SCOPE`]).
#[must_use]
pub fn create_test(ports: &Ports, config: &Config, turn: u32) -> Stage<Loop> {
    Stage {
        name: "create-test".to_string(),
        pre: Some(Gate {
            name: "create-test requires",
            checks: always("create-test"),
        }),
        post: None,
        body: Paid {
            stage: "create-test",
            model: "sonnet",
            lead: "/create-test",
            instructions: TEST_SCOPE,
            write_side_instructions: None,
            cut: Cut::TaskOnly(TEST_SKIPS),
            then: Vec::new(),
        }
        .body(ports, config, turn),
    }
}

/// The sequence of a round, in execution order.
#[must_use]
pub fn table(ports: &Ports, config: &Config, turn: u32) -> Vec<Stage<Loop>> {
    vec![
        technical_refinement(ports, config, turn),
        code(ports, config, turn),
        create_test(ports, config, turn),
    ]
}

/// `technical-refinement(opus/high) -> code(opus/high) -> …`
///
/// Derived from **built** stages, not a second list: the announced line and
/// what runs cannot diverge, and `MODEL`/`EFFORT` forcing shows up here
/// without being reapplied.
#[must_use]
pub fn summary(stages: &[Stage<Loop>], settings: &Settings) -> String {
    stages
        .iter()
        .filter(|stage| settings.runs(&stage.name))
        .map(|stage| match &stage.body {
            StageBody::Session { spec, .. } => {
                format!("{}({}/{})", stage.name, spec.model, spec.effort)
            }
            StageBody::Local { .. } => format!("{}(local)", stage.name),
        })
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// Stages that `--stages` leaves out — never silently.
#[must_use]
pub fn filtered_out(stages: &[Stage<Loop>], settings: &Settings) -> Vec<String> {
    stages
        .iter()
        .filter(|stage| !settings.runs(&stage.name))
        .map(|stage| stage.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_loop::config::fake as config_fake;
    use crate::dev_loop::ports::fake as ports_fake;

    fn settings(stages: &str) -> Settings {
        Settings {
            dry_run: false,
            stages: stages.to_string(),
        }
    }

    #[test]
    fn the_order_of_the_table_is_the_order_of_the_round() {
        let names: Vec<String> = table(&ports_fake::ports(), &config_fake::config(), 1)
            .iter()
            .map(|stage| stage.name.clone())
            .collect();
        assert_eq!(names, ["technical-refinement", "code", "create-test"]);
    }

    #[test]
    fn no_instruction_text_names_a_label_the_code_does_not_use() {
        // Failure mode avoided: the prompt asks for `pipeline:human`, the label
        // no longer exists, and the session creates a new one.
        for text in [TECHNICAL_REFINEMENT, CODE] {
            let said = with_labels(text);
            assert!(
                !said.contains("pipeline:"),
                "a `pipeline:*` label remains in the prose"
            );
            assert!(
                !said.contains("{human}") && !said.contains("{agent}"),
                "a label template was not spliced"
            );
        }
    }

    #[test]
    fn every_label_dev_loops_own_prose_mentions_appears_as_written() {
        let said = format!(
            "{} {}",
            with_labels(TECHNICAL_REFINEMENT),
            with_labels(CODE)
        );
        // Only the labels dev_loop's own two prompts talk about: opening a
        // human blocker, and the merge-wait state. `ROADMAP`/`MILESTONE`/
        // `READY`/`AGENT` are the planner/split workflows' concern now, not
        // something a technical-refinement or `/code` session needs explained
        // to it. `SPEC_WRITTEN` is set by the harness, never mentioned to the
        // session either.
        for label in [labels::HUMAN, labels::WAITING_MERGE] {
            assert!(said.contains(label), "{label} missing from prose");
        }
    }

    #[test]
    fn the_task_placeholders_survive_the_label_pass_for_the_scope_to_fill() {
        let said = with_labels(TECHNICAL_REFINEMENT);
        assert!(said.contains("#{num}"), "the number remains to fill");
        assert!(said.contains("{title}"));
        assert!(said.contains("milestone #{milestone}"));
    }

    #[test]
    fn the_write_side_code_prompt_opens_a_pr_and_never_merges() {
        let text = with_labels(CODE_WRITE_SIDE);
        assert!(text.contains("gh pr create --label harness:to-review"));
        assert!(text.contains("do NOT run `gh pr merge`"));
        assert!(text.contains("harness:review-pending"));
        assert!(text.contains("harness:write-side"));
        assert!(!text.contains("{to_review}") && !text.contains("{review_pending}"));
        for placeholder in ["{num}", "{title}"] {
            assert!(
                text.contains(placeholder),
                "{placeholder} is filled per task"
            );
        }
        let read = with_labels(CODE);
        assert!(
            read.contains("gh pr merge --squash"),
            "the read side still merges"
        );
    }

    #[test]
    fn the_test_scope_names_the_script_init_repo_installs() {
        // The prompt and the installer agree on one path, or the session
        // falls back to the whole workspace without anyone noticing.
        let installed = crate::init_repo::data::install::FILES
            .iter()
            .any(|asset| asset.path.starts_with("scripts/") && TEST_SCOPE.contains(asset.path));
        assert!(installed, "TEST_SCOPE names a script init-repo installs");
        assert_eq!(with_labels(TEST_SCOPE), TEST_SCOPE, "nothing to splice");
    }

    #[test]
    fn technical_refinement_records_the_sections_before_declaring_itself_done() {
        // Order matters: declaring done before recording would leave a written
        // but unrecorded SPEC, so it would be rewritten on the next run.
        let stage = technical_refinement(&ports_fake::ports(), &config_fake::config(), 1);
        let StageBody::Session { actions, .. } = &stage.body else {
            panic!("a paid stage");
        };
        assert_eq!(actions.len(), 3, "Ask, record, mark");
    }

    #[test]
    fn no_stage_drops_a_section_that_is_not_part_of_an_issue_body() {
        // The failure mode: a drop-list naming `Technical Design`, nothing
        // matching it, and a stage still paying for the section on every turn
        // with nothing to show that the filter did nothing.
        //
        // `Assumptions` is the one entry no canonical table carries: a session
        // writes it, under the heading `TECHNICAL_REFINEMENT` asks for.
        for list in [REFINEMENT_WRITES, CODE_SKIPS, TEST_SKIPS] {
            for heading in list {
                let known = crate::common::sections::SECTIONS
                    .iter()
                    .any(|s| s.heading == *heading);
                assert!(
                    known || TECHNICAL_REFINEMENT.contains(heading),
                    "no issue body ever carries `## {heading}`"
                );
            }
        }
    }

    #[test]
    fn the_refinement_does_not_receive_the_sections_its_own_prose_asks_it_to_write() {
        // Two halves of one decision: the prompt names the sections to write,
        // the cut names the sections left out. A section asked for but injected
        // invites the session to edit a previous run's draft instead.
        for heading in ["Technical", "Technical Implementation Plan", "Assumptions"] {
            assert!(
                TECHNICAL_REFINEMENT.contains(heading),
                "the prose must ask for `{heading}`"
            );
            assert!(
                REFINEMENT_WRITES.contains(&heading),
                "and the cut must leave it out"
            );
        }
    }

    #[test]
    fn create_test_keeps_the_plan_it_asserts_against() {
        // `Technical` and `Technical Implementation Plan` are two sections, and
        // `/create-test` needs exactly one of them. A prefix-matching drop-list
        // would take both — see `sections::without`.
        assert!(TEST_SKIPS.contains(&"Technical"));
        assert!(!TEST_SKIPS.contains(&"Technical Implementation Plan"));
    }

    #[test]
    fn the_summary_reads_the_models_off_the_stages_that_will_run() {
        let said = summary(
            &table(&ports_fake::ports(), &config_fake::config(), 1),
            &settings(""),
        );
        assert_eq!(
            said,
            "technical-refinement(opus/high) -> code(sonnet/high) -> \
             create-test(sonnet/high)"
        );
    }

    #[test]
    fn a_forced_model_shows_up_in_the_summary_without_being_reapplied() {
        let mut config = config_fake::config();
        config.model = "haiku".to_string();
        let said = summary(&table(&ports_fake::ports(), &config, 1), &settings(""));
        assert!(!said.contains("opus"), "forcing applies to the entire run");
        assert!(
            !said.contains("sonnet"),
            "including each stage's own default"
        );
    }

    #[test]
    fn what_stages_leaves_out_is_named_rather_than_silently_dropped() {
        let built = table(&ports_fake::ports(), &config_fake::config(), 1);
        let cfg = settings("code");
        assert_eq!(summary(&built, &cfg), "code(sonnet/high)");
        assert_eq!(
            filtered_out(&built, &cfg),
            [
                "technical-refinement".to_string(),
                "create-test".to_string()
            ]
        );
    }
}
