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
//! One stage, `code`: it plans, builds and tests the task in one session, on
//! sonnet — `split` decided the SPEC — and on opus for the data layer, where
//! a wrong answer costs twice. Tune to the cost registry, not intuition.
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
use crate::dev_loop::action::actions::Ask;
use crate::dev_loop::checks::architecture::{ArchitectureHolds, ConceptsDocumented};
use crate::dev_loop::checks::gates;
use crate::dev_loop::config::Config;
use crate::dev_loop::data::brief::Cut;
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::ports::Ports;
use crate::prompts;

/// What `code` leaves out of the SPEC: nothing.
///
/// One session plans, builds and tests the task: the goal is what it plans
/// from, the acceptance criteria and the business rules what it verifies and
/// tests against, the technical sketch where it starts.
const CODE_SKIPS: &[&str] = &[];

/// How `code` runs the Rust tests of a repository under the
/// architecture: through the script `init-repo` installs, which the CI runs
/// too. A task that stays inside Capability crates then waits for their
/// tests alone instead of the whole workspace's.
pub(crate) const TEST_SCOPE: &str =
    "Run the Rust tests with `bash scripts/test-scope.sh origin/<the branch your
PR targets>` whenever the repository has that script, never a bare
`cargo test --workspace`: a change that stays inside Capability crates
(`crates/<system>/capabilities/<name>/`) then runs their tests alone, and
anything else the whole workspace — the CI runs the same script on the PR.
Arguments after `--` go to `cargo test`, to run one test while you work.";

pub(crate) const CODE: &str =
    "The task is issue #{num} (\"{title}\"); its body, below under SCOPE, is the
SPEC. Plan it as /tech-analyst: the pre-flight gate, the ordered checklist
against the real code, the stop line, the risks. Then, in this same session,
without waiting for a go-ahead and without /clear, carry out
.claude/skills/code/SKILL.md against your own plan — build, run every
Verification bullet with real output, /code-review, run the architecture
gates of `.github/workflows/gates.yml` locally and fix what fails (the loop
checks the same rules after you), then branch -> PR ->
gh pr merge --squash. A Micro-UI or a screen composition ships with its
mock-up: open the page under mock/pages/ and the micro-frontend under
mock/components/ first (they are the screen as the human wants it; a
`mockup/*` branch may carry a newer one), keep them true to what you build,
list them in mock/pages.json (contract N), name the parts of what a Micro-UI
renders (contract D: data-writes=\"<DataCapability>\" on every actuator,
data-reads=\"<Capability>\" on every part that shows a read, one at least per
entry of its actions and needs), and after any change under apps/ or
mock/src/ run mock/build.sh and commit the bundles — the gates refuse a
screen no page mocks, a Micro-UI with no micro-frontend, a listing out of
sync with the manifest, a micro-frontend that names no part for one of its
actions or needs, and a stale bundle. A frontend task reads the
product's design system first: mock/design/tokens.css (one CSS variable per
token — every colour, font, spacing, radius and shadow of a screen comes
from them, never a literal) and the kit, mock/kit/<Name>.html listed in
mock/kit/kit.json, its look gathered in mock/kit/kit.css (contract O). Both
reach your branch from the integration branch — the harness carries them
there from `design-system` on every publish — and they are read-only here:
the shell loads tokens.css and kit.css once, a screen or a Micro-UI writes
the kit's markup and classes (mock/kit/<Name>.html is the structure to
follow) with its own data, and a gate refuses a pull request that changes
mock/design/ or mock/kit/. When a kit component cannot serve as it is,
recode it for real in your code and write the corrected fragment back on
`design-system` (a worktree of that branch, commit, push) with one line
under `recoded` in kit.json saying what changed, so the mock-up matches
what shipped. The PR body MUST carry the line `Closes #{num}` on its
own: the loop reads that line off the merged PR to confirm the task shipped,
and without it the round stops rather than replay a task nothing marks as
delivered. The loop closes the issue once that PR is merged on the
milestone's branch; the milestone itself lands under `{waiting_merge}`. Do
not close the issue yourself.
If the SPEC has a `## Rules` section, the tests of those rules, under
tests/rules/ (contract P), are written already and are what this task
turns green: start the product with `bash scripts/rules-runtime.sh serve`
(export the KEY=value lines it prints), run `cargo test --manifest-path
tests/rules/business/Cargo.toml` and, for a journey rule, `npx playwright
test` in tests/rules/journey, then `bash scripts/rules-runtime.sh stop`. A
rule stays pending while a unit its test needs is not built. Never change
rules.json, tests/rules/ or scripts/rules-runtime.sh — a gate refuses it.
A rule's test you cannot meet is contested: write CONTEST.md at the root
(the rule, the test, why it cannot hold) and say so in your reply. The
server is crates/server, the frontend's host apps/shell, as the layout of
CLAUDE.md says: the runtime starts them from there.
No human is reachable while you work: every answer you had to assume goes
in the PR body under `## Assumptions (autonomous run)`. Anything a human
must do first becomes its own `{human}` issue, a sub-issue of
milestone #{milestone}, declared as a dependency of #{num}. If an assumption would make
the task useless or harmful when wrong — a paid service, a schema decision
later tasks depend on, a credential only the human holds — that is
ambiguity, not a default: AGENT_LOOP_STOP instead of guessing; the harness
then decides it and writes the answer under `## Decision`. Honour
`## Decision` and `## Owner Decisions` when the body has them: they
override the sections above.
Then, in this same session and without /clear, write the tests as
.claude/skills/create-test/SKILL.md says — triage what you changed, write
only the hermetic tests the change warrants, prove each red then green — and
ship them **in the same PR**, never a second one. The PR body carries your
plan (checklist, risks) under `## Plan` — the issue body is the SPEC, you
never rewrite it — and, when the SPEC has a `## Rules` section, the line
`Rules: R3, R5` naming them, which a gate checks.
/code's steps 1-3 are what you just did as the tech analyst; adopt your own
findings instead of re-deriving them. Nothing outside this session can read
your plan, so a gate finding or a risk you do not act on now is lost — put it
in your reply.";

/// `code`, on the write side: the same work, and a PR left open for its review.
///
/// The architecture reviews every mutation of existing data before it
/// merges, so the session never merges: it opens the PR with
/// `harness:to-review` — the agent review runs on it — and stops. The loop
/// then marks the task `review-pending`, and `milestone_merge` merges it once
/// the review is clean and every check green.
pub(crate) const CODE_WRITE_SIDE: &str =
    "The task is issue #{num} (\"{title}\"); its body, below under SCOPE, is the
SPEC. It is on the **write side** of the architecture ({write_side}): it
mutates existing data or the rules that guard it, and it merges only once
its review is clean and every check green.
Plan it as /tech-analyst: the pre-flight gate, the ordered checklist against
the real code, the stop line, the risks. Then, in this same session, without
waiting for a go-ahead and without /clear, carry out
.claude/skills/code/SKILL.md against your own plan — build, run every
Verification bullet with real output, /code-review, run the architecture
gates of `.github/workflows/gates.yml` locally and fix what fails (the loop
checks the same rules after you); a Micro-UI or a screen ships with its
mock-up under mock/, listed in mock/pages.json (contract N), its parts
named (data-writes / data-reads, contract D), bundles
rebuilt with mock/build.sh, styled through the design system's variables
(mock/design/tokens.css) and the kit's classes (mock/kit/kit.css, contract
O) — both carried onto the integration branch from `design-system`,
read-only on yours — a kit component that cannot serve as it is recoded on
`design-system` with a `recoded` line; then
branch -> PR, and
STOP THERE: open the pull request against the milestone branch with
`gh pr create --label {to_review}` (the agent review runs on it), wait for
its CI with `gh pr checks`, and do NOT run `gh pr merge` — a write-side PR
is merged by `milestone_merge` once the review has run and every check is
green. The PR body MUST carry the line `Closes #{num}` on its own: the loop
reads that line off the PR to know the task is delivered once it is merged.
The issue will carry `{review_pending}` until then —
that is expected, not a failure. Do not close the issue yourself.
If the SPEC has a `## Rules` section, the tests of those rules, under
tests/rules/ (contract P), are written already and are what this task
turns green: start the product with `bash scripts/rules-runtime.sh serve`
(export the KEY=value lines it prints), run `cargo test --manifest-path
tests/rules/business/Cargo.toml` and, for a journey rule, `npx playwright
test` in tests/rules/journey, then `bash scripts/rules-runtime.sh stop`. A
rule stays pending while a unit its test needs is not built. Never change
rules.json, tests/rules/ or scripts/rules-runtime.sh — a gate refuses it.
A rule's test you cannot meet is contested: write CONTEST.md at the root
(the rule, the test, why it cannot hold) and say so in your reply. The
server is crates/server, the frontend's host apps/shell, as the layout of
CLAUDE.md says: the runtime starts them from there.
No human is reachable while you work: every answer you had to assume goes
in the PR body under `## Assumptions (autonomous run)`. Anything a human
must do first becomes its own `{human}` issue, a sub-issue of
milestone #{milestone}, declared as a dependency of #{num}. If an assumption would make
the task useless or harmful when wrong — a paid service, a schema decision
later tasks depend on, a credential only the human holds — that is
ambiguity, not a default: AGENT_LOOP_STOP instead of guessing; the harness
then decides it and writes the answer under `## Decision`. Honour
`## Decision` and `## Owner Decisions` when the body has them: they
override the sections above.
Then, in this same session and without /clear, write the tests as
.claude/skills/create-test/SKILL.md says — triage what you changed, write
only the hermetic tests the change warrants, prove each red then green — and
ship them **in the same PR**, never a second one. The PR body carries your
plan (checklist, risks) under `## Plan` — the issue body is the SPEC, you
never rewrite it — and, when the SPEC has a `## Rules` section, the line
`Rules: R3, R5` naming them, which a gate checks.
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

/// `code`: plans in `/tech-analyst`, builds and delivers as `/code`, tests
/// as `/create-test` — the whole task, one session.
///
/// Three skills, one session: each adopts what the one before it just
/// decided. Nothing outside the session can re-read a plan, so splitting it
/// into stages lost it and paid for the SPEC to be read again each time —
/// there were three stages once (a technical refinement, `code`,
/// `create-test`).
#[must_use]
pub fn code(ports: &Ports, config: &Config, turn: u32) -> Stage<Loop> {
    let test_scope = config.prompts.text(prompts::TEST_SCOPE, TEST_SCOPE);
    let instructions = format!("{}\n{test_scope}", config.prompts.text(prompts::CODE, CODE));
    let write_side = format!(
        "{}\n{test_scope}",
        config
            .prompts
            .text(prompts::CODE_WRITE_SIDE, CODE_WRITE_SIDE)
    );
    let mut pre = always("code");
    pre.push(Box::new(gates::CodeAlreadyDelivered {
        gh: Rc::clone(&ports.gh),
        integration_branch: config.integration_branch.clone(),
        restart: config.restart,
    }));
    pre.push(Box::new(gates::CodeHasASpec));
    pre.push(Box::new(gates::TaskHasASpec));
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
            // Sonnet, not opus: the tasks reaching this stage are sliced and
            // specified by `split`, so `code` executes a decided SPEC rather
            // than making the decision. Measured on milestone 15, `code` was
            // 62% of the spend while the stages that actually decide —
            // `plan`, `slice` — were cents. A data-layer task opens on opus
            // (`Config::data_layer_spec`, applied where the round is built).
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

/// The sequence of a round, in execution order.
#[must_use]
pub fn table(ports: &Ports, config: &Config, turn: u32) -> Vec<Stage<Loop>> {
    vec![code(ports, config, turn)]
}

/// `code(sonnet/high)` — or `code(opus/high)` on the data layer.
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
    fn the_round_is_one_session_that_plans_builds_and_tests() {
        let built = table(&ports_fake::ports(), &config_fake::config(), 1);
        let names: Vec<&str> = built.iter().map(|stage| stage.name.as_str()).collect();
        assert_eq!(names, ["code"]);
        let pre = built[0].pre.as_ref().expect("gated");
        let purposes: Vec<String> = pre.checks.iter().map(|c| c.purpose()).collect();
        assert!(
            purposes.iter().any(|p| p.contains("Acceptance Criteria")),
            "the SPEC split wrote is required: {purposes:?}"
        );
    }

    #[test]
    fn no_instruction_text_names_a_label_the_code_does_not_use() {
        // Failure mode avoided: the prompt asks for `pipeline:human`, the label
        // no longer exists, and the session creates a new one.
        for text in [CODE, CODE_WRITE_SIDE] {
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
        // Only the labels the prompts talk about: opening a human blocker,
        // and the merge-wait state.
        let said = with_labels(CODE);
        for label in [labels::HUMAN, labels::WAITING_MERGE] {
            assert!(said.contains(label), "{label} missing from prose");
        }
    }

    #[test]
    fn the_task_placeholders_survive_the_label_pass_for_the_scope_to_fill() {
        for text in [CODE, CODE_WRITE_SIDE] {
            let said = with_labels(text);
            assert!(said.contains("#{num}"), "the number remains to fill");
            assert!(said.contains("{title}"));
            assert!(said.contains("milestone #{milestone}"));
        }
    }

    #[test]
    fn the_one_session_plans_builds_tests_and_never_waits_on_a_human() {
        for text in [CODE, CODE_WRITE_SIDE] {
            for said in [
                "/tech-analyst",
                ".claude/skills/create-test/SKILL.md",
                "in the same PR",
                "## Assumptions (autonomous run)",
                "AGENT_LOOP_STOP",
                "## Decision",
                "## Owner Decisions",
                "## Plan",
                "Rules: R3, R5",
            ] {
                assert!(text.contains(said), "{said}");
            }
        }
    }

    #[test]
    fn the_write_side_code_prompt_opens_a_pr_and_never_merges() {
        let text = with_labels(CODE_WRITE_SIDE);
        assert!(text.contains("gh pr create --label harness:to-review"));
        assert!(text.contains("do NOT run `gh pr merge`"));
        assert!(text.contains("harness:review-pending"));
        assert!(text.contains("harness:write-side"));
        assert!(!text.contains("{to_review}") && !text.contains("{review_pending}"));
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
        let installed = harness_architecture::assets::FILES
            .iter()
            .any(|asset| asset.path.starts_with("scripts/") && TEST_SCOPE.contains(asset.path));
        assert!(installed, "TEST_SCOPE names a script init-repo installs");
        assert_eq!(with_labels(TEST_SCOPE), TEST_SCOPE, "nothing to splice");
    }

    #[test]
    fn no_stage_drops_a_section_that_is_not_part_of_an_issue_body() {
        for heading in CODE_SKIPS {
            let known = crate::common::sections::SECTIONS
                .iter()
                .any(|s| s.heading == *heading);
            assert!(known, "no issue body ever carries `## {heading}`");
        }
    }

    #[test]
    fn the_summary_reads_the_models_off_the_stages_that_will_run() {
        let said = summary(
            &table(&ports_fake::ports(), &config_fake::config(), 1),
            &settings(""),
        );
        assert_eq!(said, "code(sonnet/high)");
    }

    #[test]
    fn a_forced_model_shows_up_in_the_summary_without_being_reapplied() {
        let mut config = config_fake::config();
        config.model = "haiku".to_string();
        let said = summary(&table(&ports_fake::ports(), &config, 1), &settings(""));
        assert!(
            !said.contains("sonnet"),
            "forcing applies to the stage's own default"
        );
    }

    #[test]
    fn what_stages_leaves_out_is_named_rather_than_silently_dropped() {
        let built = table(&ports_fake::ports(), &config_fake::config(), 1);
        let cfg = settings("pick");
        assert_eq!(summary(&built, &cfg), "");
        assert_eq!(filtered_out(&built, &cfg), ["code".to_string()]);
    }
}
