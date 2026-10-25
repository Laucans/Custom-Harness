//! A re-study: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** Read the case and take the
//! streak, walk every rule in one session, then open what it found.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::prompts;
use crate::restudy::action::actions::{AskStudy, ReadCase, TakeStreak};
use crate::restudy::action::publish::Publish;
use crate::restudy::checks::gates::{StudyParses, TheCheckoutIsUntouched};
use crate::restudy::config::Config;
use crate::restudy::data::state::StudyState;
use crate::restudy::ports::Ports;

/// The free stage name: the case read, the streak taken.
pub const CONTEXT: &str = "context";
/// The paid stage name: every rule walked.
pub const STUDY: &str = "study";
/// The free stage name: the slices opened, the task parked on them.
pub const PUBLISH: &str = "publish";

pub(crate) const RESTUDY_PROMPT: &str = r#"You are the re-study of task #{num} — {title}, milestone #{milestone}, its branch
{branch} checked out here. Its sessions stopped {discovered} times in a row, each declaring the
next missing units: a journey stops at its first failure, which hides the next one. Find
EVERYTHING still missing, at once, so the harness opens it in one go.

Read-only: never edit a tracked file, commit, push, open an issue or a pull request. You may
build and run the product:
  RULES_SERVER_PORT={server_port} RULES_APP_PORT={app_port} bash scripts/rules-runtime.sh serve
probe it with curl (POST /capabilities/<id>, POST /commands, GET /commands/<id>), run a rule's
journey with Playwright's output under target/, and `bash scripts/rules-runtime.sh stop` before
you answer.

Already open for this milestone — being built, never declare them as new (number, title,
branch, unit, produces, needs, state labels):
{open_slices}
Landed or closed: {closed_slices}

The task's SPEC:
<body>
{body}
</body>

Walk every rule the SPEC names (## Rules, rules.json, its test under tests/rules/), step by step,
and never stop at the first failure:
1. the screen's composition: each Micro-UI it mounts exists, with the props the rule needs;
2. each Micro-UI's needs and actions (micro-ui.json) against the manifests: the id exists, the
   payload and variables match, the roles let the command apply;
3. the fixtures the journey needs (mock/fixtures) against what the server loads into its store;
4. the running server, for every read and command the rule's steps use, with the fixtures' ids:
   what it answers.
Missing is only what no open task above builds. Put the probe that proves each gap in the slice's
brief. A slice that builds on another one names one of that slice's `produces` ids in its `needs`:
that is how the harness chains them. When an open task above needs a new slice, declare that open
task again with its exact branch and the new slice's id in its `needs`: it is found, not reopened,
and chained on the new slice.

A rule's test that asserts what its rule does not say (a position that depends on the scroll, a
timing) is a defect to contest, not a unit to build.

End your reply with these two lines, each on a line of its own:
RESTUDY: tasks [<one JSON object per missing unit, as /split writes a slice: title, branch
(feat/<slug>), brief, unit (one of: {unit_keys}; fixtures or seed data the server loads are
`infrastructure`), system, aggregate, produces, needs, goal, acceptance (given/when/then)>]
— `[]` when nothing is missing beyond the open tasks
RESTUDY_CONTEST: [{"rule": "R15", "test": "<path>:<line>", "why": "..."}] — `[]` when none"#;

/// The free stage: read the case, then take the streak.
#[must_use]
pub fn context(ports: &Ports) -> Stage<StudyState> {
    Stage {
        name: CONTEXT.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![
                Box::new(ReadCase {
                    gh: Rc::clone(&ports.gh),
                    git: Rc::clone(&ports.git),
                }),
                Box::new(TakeStreak {
                    gh: Rc::clone(&ports.gh),
                }),
            ],
        },
    }
}

/// The paid stage: one session walks every rule.
#[must_use]
pub fn study(ports: &Ports, config: &Config) -> Stage<StudyState> {
    Stage {
        name: STUDY.to_string(),
        pre: None,
        post: Some(Gate {
            name: "study must answer",
            checks: vec![
                Box::new(StudyParses {
                    stage: STUDY.to_string(),
                }),
                Box::new(TheCheckoutIsUntouched {
                    git: Rc::clone(&ports.git),
                }),
            ],
        }),
        body: StageBody::Session {
            turn_cap: None,
            spec: config.restudy.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskStudy {
                stage: STUDY.to_string(),
                template: config.prompts.text(prompts::RESTUDY, RESTUDY_PROMPT),
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// The free stage: open, chain, park, write, tell.
#[must_use]
pub fn publish(ports: &Ports) -> Stage<StudyState> {
    Stage {
        name: PUBLISH.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Publish {
                gh: Rc::clone(&ports.gh),
            })],
        },
    }
}

/// Context, study, publish, in order.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<StudyState>> {
    vec![context(ports), study(ports, config), publish(ports)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::restudy::config::fake as config_fake;
    use crate::restudy::ports::fake as ports_fake;

    #[test]
    fn the_order_of_the_table_is_context_study_publish() {
        let table = table(&ports_fake::ports(), &config_fake::config());
        let names: Vec<&str> = table.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, [CONTEXT, STUDY, PUBLISH]);
        assert!(table[1].pre.is_none() && table[1].post.is_some());
        assert_eq!(
            table[1].post.as_ref().map(|gate| gate.checks.len()),
            Some(2)
        );
    }

    #[test]
    fn the_prompt_walks_every_rule_read_only_and_ends_on_its_two_lines() {
        for placeholder in [
            "{num}",
            "{title}",
            "{milestone}",
            "{branch}",
            "{discovered}",
            "{server_port}",
            "{app_port}",
            "{open_slices}",
            "{closed_slices}",
            "{body}",
            "{unit_keys}",
        ] {
            assert!(RESTUDY_PROMPT.contains(placeholder), "{placeholder}");
        }
        for said in [
            "never stop at the first failure",
            "Read-only: never edit a tracked file, commit, push",
            "rules-runtime.sh serve",
            "rules-runtime.sh stop",
            "names one of that slice's `produces` ids in its `needs`",
            "a defect to contest, not a unit to build",
            "\nRESTUDY: tasks [",
            "\nRESTUDY_CONTEST: [",
            "never declare them as new",
            "declare that open\ntask again with its exact branch",
        ] {
            assert!(RESTUDY_PROMPT.contains(said), "{said}");
        }
        assert!(!RESTUDY_PROMPT.contains("AGENT_LOOP_STOP"));
    }
}
