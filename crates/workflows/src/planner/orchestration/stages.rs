//! Planner: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** The order of entries *is*
//! the execution order: context, then the plan request, then publishing.
//!
//! The repo map (`ground` + `explore`) is **not** wired here: it comes from
//! [`crate::common::explore::entries`], which
//! [`crate::planner::orchestration::round::build`] puts at the head of the
//! sequence — this table itself has no way to read the repo, and doesn't
//! need to.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::planner::action::actions::{AskForPlan, ReadContext};
use crate::planner::action::publish::Write;
use crate::planner::checks::gates::PlanParses;
use crate::planner::config::Config;
use crate::planner::data::state::PlannerState;
use crate::planner::ports::Ports;

/// The free context-reading stage name.
pub const CONTEXT: &str = "context";
/// The paid plan-request stage name.
pub const PLAN: &str = "plan";
/// The publish local stage name.
pub const PUBLISH: &str = "publish";

const PLAN_PROMPT: &str = r#"Roadmap item #{num} — "{title}":
{body}

Decide the milestones that deliver this roadmap item — ordered, each one
deliverable and reviewable on its own, following
`.claude/skills/planner/SKILL.md`'s own method for sequencing and scope.
Verify the ground truth in the repo rather than the issue's own claims about
it; if a dependency this item builds on is not actually there, say so in
your own words and answer with an empty JSON array `[]` — do not plan on top
of a missing dependency.

The repository follows the agent-native architecture (`docs/ARCHITECTURE.md`
in the repository map): a read side of independent units — Capabilities,
Micro-UIs, Concepts, persisted queries, compositions — and a write side
behind one DataGuard — DataCapabilities that mutate, invariants, migrations.
Draw each milestone inside one system (bounded context) where possible, and
name the `systems` it works in and the `concepts` (versioned, `Risk@3`) it
defines or implements. Set `writes: true` on a milestone whose work touches
the write side, and order those before the read-side milestones that depend
on them: a human merges every write-side task, while read-side tasks run in
parallel without review.

{existing}

{grounding}

Answer with a JSON array and nothing else, one object per milestone, in
delivery order:
```json
[{"title": "a short, specific title", "goal": "a few sentences on what this milestone achieves", "systems": ["credit"], "concepts": ["Risk@3"], "writes": false}]
```
An empty array `[]` means this roadmap item needs no further milestone right
now."#;

/// The free context-reading stage: milestones already open, grilling
/// digests. Costs nothing, so no `pre`/`post` gate is needed.
#[must_use]
pub fn context(ports: &Ports, config: &Config) -> Stage<PlannerState> {
    Stage {
        name: CONTEXT.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(ReadContext {
                gh: Rc::clone(&ports.gh),
                disk: Rc::clone(&ports.disk),
                grill_dir: config.grill_dir.clone(),
            })],
        },
    }
}

/// The paid plan-request stage: asks for the JSON plan, tolerating one
/// malformed reply (`AskForPlan`), then demands it parses (`PlanParses`).
#[must_use]
pub fn plan(ports: &Ports, config: &Config) -> Stage<PlannerState> {
    Stage {
        name: PLAN.to_string(),
        pre: None,
        post: Some(Gate {
            name: "plan must parse",
            checks: vec![Box::new(PlanParses {
                stage: PLAN.to_string(),
            })],
        }),
        body: StageBody::Session {
            spec: config.plan.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskForPlan {
                stage: PLAN.to_string(),
                template: PLAN_PROMPT,
                artifacts_dir: config.artifacts_dir.clone(),
                explore: config.explore,
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// Publishing: a local stage, which costs nothing.
#[must_use]
pub fn publish(ports: &Ports) -> Stage<PlannerState> {
    Stage {
        name: PUBLISH.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Write {
                gh: Rc::clone(&ports.gh),
                plan_stage: PLAN.to_string(),
            })],
        },
    }
}

/// Context, plan, publish — in order. The map (`ground`/`explore`) is not
/// here: see the module.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<PlannerState>> {
    vec![context(ports, config), plan(ports, config), publish(ports)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::config::fake as config_fake;
    use crate::planner::ports::fake as ports_fake;

    #[test]
    fn the_order_of_the_table_is_context_then_plan_then_publish() {
        let names: Vec<String> = table(&ports_fake::ports(), &config_fake::config())
            .iter()
            .map(|stage| stage.name.clone())
            .collect();
        assert_eq!(names, [CONTEXT, PLAN, PUBLISH]);
    }

    #[test]
    fn the_prompt_carries_every_placeholder_the_action_fills() {
        for placeholder in ["{num}", "{title}", "{body}", "{existing}", "{grounding}"] {
            assert!(
                PLAN_PROMPT.contains(placeholder),
                "{placeholder} missing from PLAN_PROMPT"
            );
        }
    }
}
