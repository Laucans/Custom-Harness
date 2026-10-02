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
//! Distributed by where a wrong answer costs twice. `/business-analyst` writes
//! the SPEC — the issue body — that all following stages read; `code` plans
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
use crate::dev_loop::action::actions::{Ask, RecordSpecWritten};
use crate::dev_loop::checks::gates;
use crate::dev_loop::config::Config;
use crate::dev_loop::data::grounding;
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::ports::Ports;

const BUSINESS_ANALYST: &str =
    "Take issue #{num} (\"{title}\") — its body is below, under SCOPE. The
interview in step 3 of the skill cannot happen — no human is reachable.
Answer each question you would have asked from docs/PROJECT.md,
docs/ARCHITECTURE.md and the repo itself, and record every answer you had
to assume under an `## Assumptions (autonomous run)` heading. Write the SPEC
into the body of issue #{num} itself — that body is what the /code stage
reads, and nothing else is. Anything a human must do first becomes its own
`{human}` issue, a sub-issue of milestone #{milestone}, declared as a
dependency of #{num}: the loop will not pick #{num} up again until it is
closed. If an assumption would make the task useless or harmful when wrong —
a paid service, a schema decision the later tasks depend on, a credential
only the human holds — that is ambiguity, not a default: AGENT_LOOP_STOP
instead of guessing.";

const CODE: &str = "The task is issue #{num} (\"{title}\"); its body, below under SCOPE, is the
SPEC. Plan it as /tech-analyst: the pre-flight gate, the ordered checklist
against the real code, the stop line, the risks. Then, in this same session,
without waiting for a go-ahead and without /clear, carry out
.claude/skills/code/SKILL.md against your own plan — build, run every
Verification bullet with real output, /code-review, then branch -> PR ->
gh pr merge --rebase. The PR body MUST carry the line `Closes #{num}` on its
own: the loop reads that line off the merged PR to confirm the task shipped,
and without it the round stops rather than replay a task nothing marks as
delivered. The issue will stay open under `{waiting_merge}` until a
human merges the integration branch — that is expected, not a failure. Do
not close the issue yourself.
/code's steps 1-3 are what you just did as the tech analyst; adopt your own
findings instead of re-deriving them. Nothing outside this session can read
your plan, so a gate finding or a risk you do not act on now is lost — put it
in your reply.";

const PLANNER: &str = "Every `{agent}` issue of milestone #{milestone} is closed. Take the
open `{roadmap}` issue with the lowest number — do not ask which one.
Open one `{milestone_label}` issue for it, as a sub-issue of that roadmap
issue, and one `{agent}` sub-issue per task slice, each blocked_by the
one before it. Then close milestone #{milestone}, and close the roadmap issue
it hangs off if that item is now fully delivered: the loop works on the
LOWEST-numbered open milestone, so leaving the finished one open makes every
later round roll over again instead of picking up what you just planned. Do
not add `{ready}` to anything: the human opens the tap. Step 3 of the
skill applies in full: verify the ground truth in the repo, and if a
dependency the item builds on is not actually there, AGENT_LOOP_STOP with
what is missing rather than planning on top of it.";

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
    scoped: bool,
    then: Vec<Box<dyn SessionAction<Loop>>>,
}

impl Paid<'_> {
    fn body(self, ports: &Ports, config: &Config, turn: u32) -> StageBody<Loop> {
        let mut actions: Vec<Box<dyn SessionAction<Loop>>> = vec![Box::new(Ask {
            stage: self.stage.to_string(),
            lead: self.lead.to_string(),
            instructions: with_labels(self.instructions),
            scoped: self.scoped,
            round: turn,
            branch: config.integration_branch.clone(),
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

/// `/business-analyst`: writes the SPEC into the issue body.
#[must_use]
pub fn business_analyst(ports: &Ports, config: &Config, turn: u32) -> Stage<Loop> {
    let body = Paid {
        stage: "business-analyst",
        model: "opus",
        lead: "/business-analyst",
        instructions: BUSINESS_ANALYST,
        scoped: true,
        // Recording the reviewed SPEC is local work, and it goes *in* the
        // stage — not in a gate, which would have no right to write.
        then: vec![Box::new(Unpaid(RecordSpecWritten {
            gh: Rc::clone(&ports.gh),
        }))],
    }
    .body(ports, config, turn);
    let mut pre = always("business-analyst");
    pre.push(Box::new(gates::SpecAlreadyWritten));
    Stage {
        name: "business-analyst".to_string(),
        pre: Some(Gate {
            name: "business-analyst requires",
            checks: pre,
        }),
        post: Some(Gate {
            name: "business-analyst must achieve",
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
        // Nothing to verify after: what `code` must achieve is the round's
        // postcondition — a merged PR carrying `Closes #N` — and checking
        // it twice says nothing more.
        post: None,
        body: Paid {
            stage: "code",
            model: "opus",
            lead: "/tech-analyst",
            instructions: CODE,
            scoped: true,
            then: Vec::new(),
        }
        .body(ports, config, turn),
    }
}

/// `/create-test`: hermetic Vitest against an already-written spec.
///
/// No custom instructions or gate: it works against an existing spec, preamble
/// and scope are enough, and nothing it produces conditions what follows —
/// it's the final stage.
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
            instructions: "",
            scoped: true,
            then: Vec::new(),
        }
        .body(ports, config, turn),
    }
}

/// `/planner`: opens the next roadmap item when the milestone is done.
///
/// **Not in [`table`]**, by design: rollover is a branch, not another stage —
/// it runs when there are **no** tasks, so when the sequence has nothing to do.
/// The launcher decides whether to wire it; chaining unsupervised spends an opus
/// run and commits the project to a roadmap item nobody read.
#[must_use]
pub fn planner(ports: &Ports, config: &Config, turn: u32) -> Stage<Loop> {
    let business = ports
        .disk
        .read_to_string(&config.grill_dir.join("business-digest.md"));
    let technical = ports
        .disk
        .read_to_string(&config.grill_dir.join("technical-digest.md"));
    let instructions = planner_instructions(business.as_deref(), technical.as_deref());
    Stage {
        name: "planner".to_string(),
        pre: Some(Gate {
            name: "planner requires",
            checks: always("planner"),
        }),
        post: Some(Gate {
            name: "planner must achieve",
            checks: vec![Box::new(gates::PlannerOpenedATask {
                gh: Rc::clone(&ports.gh),
            })],
        }),
        body: Paid {
            stage: "planner",
            model: "opus",
            lead: "/planner",
            instructions: &instructions,
            // The only stage in this case: it works on no task, it opens one.
            scoped: false,
            then: Vec::new(),
        }
        .body(ports, config, turn),
    }
}

/// `PLANNER`, with the two grill digests appended when either exists.
///
/// Pulled out of [`planner`] so the splice itself is testable without
/// building a whole `Stage` and reaching into a type-erased `SessionAction`.
fn planner_instructions(business: Option<&str>, technical: Option<&str>) -> String {
    let gathered = grounding::combine(business, technical);
    if gathered.is_empty() {
        PLANNER.to_string()
    } else {
        format!(
            "{PLANNER}\n\nGathered ahead of time, before any human was \
             reachable — verify ground truth in the repo where it disagrees:\n\n{gathered}"
        )
    }
}

/// The sequence of a round, in execution order.
#[must_use]
pub fn table(ports: &Ports, config: &Config, turn: u32) -> Vec<Stage<Loop>> {
    vec![
        business_analyst(ports, config, turn),
        code(ports, config, turn),
        create_test(ports, config, turn),
    ]
}

/// `business-analyst(opus/high) -> code(opus/high) -> …`
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
        assert_eq!(names, ["business-analyst", "code", "create-test"]);
    }

    #[test]
    fn no_instruction_text_names_a_label_the_code_does_not_use() {
        // Failure mode avoided: the prompt asks for `pipeline:human`, the label
        // no longer exists, and the session creates a new one.
        for text in [BUSINESS_ANALYST, CODE, PLANNER] {
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
    fn every_label_the_prose_mentions_is_one_of_the_seven() {
        let said = format!(
            "{} {} {}",
            with_labels(BUSINESS_ANALYST),
            with_labels(CODE),
            with_labels(PLANNER)
        );
        for label in labels::LOOP {
            // `spec-written` is not mentioned: the harness sets it, not the
            // session. The others must appear as-is.
            if label == labels::SPEC_WRITTEN {
                continue;
            }
            assert!(said.contains(label), "{label} missing from prose");
        }
    }

    #[test]
    fn the_task_placeholders_survive_the_label_pass_for_the_scope_to_fill() {
        let said = with_labels(BUSINESS_ANALYST);
        assert!(said.contains("#{num}"), "the number remains to fill");
        assert!(said.contains("{title}"));
        assert!(said.contains("milestone #{milestone}"));
    }

    #[test]
    fn business_analyst_records_the_spec_before_declaring_itself_done() {
        // Order matters: declaring done before recording would leave a written
        // but unrecorded SPEC, so it would be rewritten on the next run.
        let stage = business_analyst(&ports_fake::ports(), &config_fake::config(), 1);
        let StageBody::Session { actions, .. } = &stage.body else {
            panic!("a paid stage");
        };
        assert_eq!(actions.len(), 3, "Ask, record, mark");
    }

    #[test]
    fn the_planner_is_not_in_the_sequence_because_rollover_is_a_branch() {
        assert!(
            !table(&ports_fake::ports(), &config_fake::config(), 1)
                .iter()
                .any(|stage| stage.name == "planner")
        );
    }

    #[test]
    fn with_neither_digest_the_planner_keeps_its_own_instructions_verbatim() {
        assert_eq!(planner_instructions(None, None), PLANNER);
    }

    #[test]
    fn a_digest_is_appended_after_the_planners_own_instructions() {
        let said = planner_instructions(Some("ship fast"), None);
        assert!(said.starts_with(PLANNER), "the original text leads");
        assert!(said.contains("Business constraints:\nship fast"));
    }

    #[test]
    fn the_planner_stage_reads_both_digests_through_the_disk_port() {
        use std::path::{Path, PathBuf};

        use harness_core::adapters::shell::disk::Disk;
        use harness_core::domain::Outcome;

        /// A disk answering only the two digest paths this test sets up —
        /// anything else panics, so a wrong path is caught immediately rather
        /// than silently reading as "no digest".
        struct OnlyDigests {
            dir: PathBuf,
        }
        impl Disk for OnlyDigests {
            fn read_to_string(&self, path: &Path) -> Option<String> {
                if path == self.dir.join("business-digest.md") {
                    Some("ship fast".to_string())
                } else if path == self.dir.join("technical-digest.md") {
                    None
                } else {
                    panic!("unexpected read: {}", path.display());
                }
            }
            fn exists(&self, _path: &Path) -> bool {
                unreachable!()
            }
            fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
                unreachable!()
            }
            fn remove_dir_all(&self, _path: &Path) -> Outcome<()> {
                unreachable!()
            }
            fn dir_names(&self, _path: &Path) -> Vec<String> {
                unreachable!()
            }
            fn write_to_string(&self, _path: &Path, _content: &str) -> Outcome<()> {
                unreachable!()
            }
        }

        let dir = PathBuf::from("/repo/.llocal/grill/o/r");
        let mut ports = ports_fake::ports();
        ports.disk = Rc::new(OnlyDigests { dir: dir.clone() });
        let mut config = config_fake::config();
        config.grill_dir = dir;

        let stage = planner(&ports, &config, 1);
        let StageBody::Session { actions, .. } = &stage.body else {
            panic!("a paid stage");
        };
        // `Ask` is type-erased here — what's checkable from outside is that
        // building the stage didn't panic on an unexpected path, i.e. it read
        // exactly the two digest paths this test wired up.
        assert_eq!(actions.len(), 2, "Ask, mark — planner has no `then`");
    }

    #[test]
    fn the_summary_reads_the_models_off_the_stages_that_will_run() {
        let said = summary(
            &table(&ports_fake::ports(), &config_fake::config(), 1),
            &settings(""),
        );
        assert_eq!(
            said,
            "business-analyst(opus/high) -> code(opus/high) -> create-test(sonnet/high)"
        );
    }

    #[test]
    fn a_forced_model_shows_up_in_the_summary_without_being_reapplied() {
        let mut config = config_fake::config();
        config.model = "sonnet".to_string();
        let said = summary(&table(&ports_fake::ports(), &config, 1), &settings(""));
        assert!(!said.contains("opus"), "forcing applies to the entire run");
    }

    #[test]
    fn what_stages_leaves_out_is_named_rather_than_silently_dropped() {
        let built = table(&ports_fake::ports(), &config_fake::config(), 1);
        let cfg = settings("code");
        assert_eq!(summary(&built, &cfg), "code(opus/high)");
        assert_eq!(
            filtered_out(&built, &cfg),
            ["business-analyst".to_string(), "create-test".to_string()]
        );
    }
}
