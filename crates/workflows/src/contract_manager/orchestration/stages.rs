//! An evolution: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** Read the request and the
//! graph, evolve the contract in one session, then write and carry it.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::contract_manager::action::actions::{AskManager, ReadRequest};
use crate::contract_manager::action::publish::Apply;
use crate::contract_manager::checks::gates::{ChangeParses, NeedsASession};
use crate::contract_manager::config::Config;
use crate::contract_manager::data::state::ManagerState;
use crate::contract_manager::ports::Ports;
use crate::prompts;

/// The free stage name: the request and the graph read.
pub const CONTEXT: &str = "context";
/// The paid stage name: the contract evolved.
pub const EVOLVE: &str = "evolve";
/// The free stage name: the new version written and carried.
pub const APPLY: &str = "apply";

pub(crate) const EVOLVE_PROMPT: &str = r#"Issue #{num} asks for a change to the contract of `{id}` — a {kind} at
version {version}, produced by milestone {producer}, read by milestones
{consumers}. Task #{task} (milestone {milestone}) stopped on it and needs:

<need>
{need}
</need>

You are the contract manager: the interface between milestones that build
side by side. A consumer never edits a contract another milestone produces;
you evolve it once, for every consumer, and the producer builds the new
version.

Read the repository (checked out on `{base}`): the manifest of `{id}` (a
`capability.json` or `data-capability.json` under its crate — `grep -rl
'"{id}"' crates apps`), its schema under `contracts/` (B for a Capability,
F for a DataCapability), the fixture that stands for it under
`mock/fixtures/`, the design dossier `mock/proof-of-concept.json`, and what
the consumers read of it (`needs:` / `actions:` in their `micro-ui.json`,
their code). Decide the smallest change that gives the consumer what it
needs and keeps every other consumer working: add before you rename, never
drop a field a consumer reads, keep the id. Bump `version` by one.

Answer with ONLY a JSON object, these keys and no prose around it:
- "path": the manifest's path, `crates/<system>/capabilities/<name>/capability.json`
  or `.../data-capabilities/<name>/data-capability.json`;
- "manifest": the whole manifest, its `version` at {version} + 1;
- "fixture_path": `mock/fixtures/{id}.json`, and "fixture": the whole fixture the
  mock server serves for the new version — or null when none is needed;
- "summary": the change, one sentence;
- "task_title": `{id} v<n>: <what changes>`;
- "task_brief": what the producer builds — fields, behaviour, tests;
- "task_architecture": the task's `## Architecture` lines, one per line:
  `unit: Capability` or `unit: DataCapability`, `id: {id}`, `system: <system>`,
  `side: harness:read-side` or `side: harness:write-side`;
- "breaking": true or false.

`breaking` is true when what was built against version {version} stops
working. Never ask; never answer "it depends"."#;

/// The free stage: read the request and the graph.
#[must_use]
pub fn context(ports: &Ports, config: &Config) -> Stage<ManagerState> {
    Stage {
        name: CONTEXT.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(ReadRequest {
                gh: Rc::clone(&ports.gh),
                base_branch: config.base_branch.clone(),
            })],
        },
    }
}

/// The paid stage: one session evolves the contract.
#[must_use]
pub fn evolve(ports: &Ports, config: &Config) -> Stage<ManagerState> {
    Stage {
        name: EVOLVE.to_string(),
        pre: Some(Gate {
            name: "evolve requires",
            checks: vec![Box::new(NeedsASession)],
        }),
        post: Some(Gate {
            name: "evolve must answer",
            checks: vec![Box::new(ChangeParses {
                stage: EVOLVE.to_string(),
            })],
        }),
        body: StageBody::Session {
            turn_cap: None,
            spec: config.evolve.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskManager {
                stage: EVOLVE.to_string(),
                template: config.prompts.text(prompts::CONTRACT_EVOLVE, EVOLVE_PROMPT),
                spending: Rc::clone(&ports.spending),
                base_branch: config.base_branch.clone(),
            })],
        },
    }
}

/// The free stage: write, open the task, tell, close.
#[must_use]
pub fn apply(ports: &Ports, config: &Config) -> Stage<ManagerState> {
    Stage {
        name: APPLY.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Apply {
                gh: Rc::clone(&ports.gh),
                evolve_stage: EVOLVE.to_string(),
                base_branch: config.base_branch.clone(),
            })],
        },
    }
}

/// Context, evolve, apply, in order.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<ManagerState>> {
    vec![
        context(ports, config),
        evolve(ports, config),
        apply(ports, config),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_manager::config::fake as config_fake;
    use crate::contract_manager::ports::fake as ports_fake;

    #[test]
    fn the_order_of_the_table_is_context_evolve_apply() {
        let table = table(&ports_fake::ports(), &config_fake::config());
        let names: Vec<&str> = table.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, [CONTEXT, EVOLVE, APPLY]);
        assert!(table[1].pre.is_some() && table[1].post.is_some());
    }

    #[test]
    fn the_prompt_names_everything_the_session_evolves_from() {
        for placeholder in [
            "{num}",
            "{id}",
            "{kind}",
            "{version}",
            "{producer}",
            "{consumers}",
            "{task}",
            "{milestone}",
            "{need}",
            "{base}",
        ] {
            assert!(EVOLVE_PROMPT.contains(placeholder), "{placeholder}");
        }
        for said in [
            "ONLY a JSON object",
            "never\ndrop a field",
            "Bump `version` by one",
            "Never ask",
            "\"breaking\"",
        ] {
            assert!(EVOLVE_PROMPT.contains(said), "{said}");
        }
    }
}
