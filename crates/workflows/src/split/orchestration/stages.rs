//! Split: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** The order of entries *is*
//! the execution order: context, then the slice request, then publishing.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::split::action::actions::{AskForSlice, ReadExistingTasks, ReadInventory};
use crate::split::action::publish::Write;
use crate::split::checks::gates::SliceParses;
use crate::split::config::Config;
use crate::split::data::state::SplitState;
use crate::split::ports::Ports;

/// The free context-reading stage name.
pub const CONTEXT: &str = "context";
/// The paid slice-request stage name.
pub const SLICE: &str = "slice";
/// The publish local stage name.
pub const PUBLISH: &str = "publish";

const SLICE_PROMPT: &str = r#"Milestone #{num} — "{title}":
{body}

Decide the task slices that deliver this milestone — each one executable by
one fresh session and becoming exactly one SPEC, following
`.claude/skills/split/SKILL.md`'s own task-splitting rules. Give each
slice a short brief — what it covers, and what it must not take from the
next slice — and a branch name of the form `<type>/<slug>` (`feat`, `fix`,
`docs`, `refactor`, `test`, `chore`, `AIchore`). Mark a slice
`needs_human: true` only for account creation, an interactive login, a
payment decision, or anything else only a human in a browser can do —
everything else is `needs_human: false`.

The repository follows the agent-native architecture (`docs/ARCHITECTURE.md`
in the repository map): every slice is exactly one unit of it. Name the
`unit` — `concept`, `data-capability`, `invariant`, `migration`,
`infrastructure`, `capability`, `persisted-query`, `micro-ui`, `composition`
or `tooling` — the `system` it belongs to (the bounded context, e.g.
`credit`), the `concept` it implements when there is one (`Risk@3`), and for
a `data-capability` its `effect` (`insert`, `update`, `delete`, `upsert`) and
the existing fields it `touches` (empty for a pure insert). `infrastructure`
is the store's own plumbing (DataQueue, DataGuard, Resolver, schema); Rust
that touches no store — contract types, helpers, CI — is `tooling`.

Order the slices in three layers, in this order and no other:
1. The Concepts the milestone defines, if any (`concept`): everything else
   reads them.
2. The milestone's **data layer**, as ONE slice when one session can build
   it: its aggregates, invariants, migrations and DataCapabilities, and any
   `infrastructure`. Cut it into two or three slices only when it is too
   big for one session, each chained on the previous — they are serialized
   anyway, behind the DataGuard. This is where the data design is decided;
   the loop builds it on its strongest model.
3. Everything that reads: `capability`, `persisted-query`, `micro-ui`,
   `composition`, `tooling`, an insert-only `data-capability`. Independent
   by design, these run in parallel, each waiting on the data layer (added
   for you): give such a slice no other `depends_on` unless it really reads
   what another slice produces.

Make the last slice the one that proves the whole milestone runs end to
end. `depends_on` lists the 0-based indexes, in this same array, of the
slices one builds on.

{existing}

{inventory}

Every unit ships its manifest, validated in CI against `contracts/`; a slice
that opens one must say so in its brief, with the required fields:
`capability.json` (capability, system, version, description, reads),
`data-capability.json` (dataCapability, owner, version, effect, target,
touches, payload, mode, invariants, permissions, callableBy, idempotencyKey),
`aggregate.json` (aggregate, fields, invariants, relations, holds),
`micro-ui.json` (microUi, system, description, needs, props),
`concept.json` (concept, version, kind, definition, output, conformance).

Answer with a JSON array and nothing else, one object per slice, in the
order tasks must be taken:
```json
[{"title": "a short, specific title", "brief": "what this slice covers and does not cover", "branch": "feat/a-slug", "needs_human": false, "unit": "capability", "system": "credit", "concept": "Risk@3", "effect": null, "touches": [], "depends_on": []}]
```
An empty array `[]` means this milestone needs no further task."#;

/// The free context-reading stage: task slices already open. Costs nothing,
/// so no `pre`/`post` gate is needed.
#[must_use]
pub fn context(ports: &Ports, config: &Config) -> Stage<SplitState> {
    Stage {
        name: CONTEXT.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![
                Box::new(ReadExistingTasks {
                    gh: Rc::clone(&ports.gh),
                }),
                Box::new(ReadInventory {
                    disk: Rc::clone(&ports.disk),
                    root: config.root.clone(),
                }),
            ],
        },
    }
}

/// The paid slice-request stage: asks for the JSON plan, tolerating one
/// malformed reply (`AskForSlice`), then demands it parses (`SliceParses`).
#[must_use]
pub fn slice(ports: &Ports, config: &Config) -> Stage<SplitState> {
    Stage {
        name: SLICE.to_string(),
        pre: None,
        post: Some(Gate {
            name: "slice must parse",
            checks: vec![Box::new(SliceParses {
                stage: SLICE.to_string(),
            })],
        }),
        body: StageBody::Session {
            spec: config.slice.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskForSlice {
                stage: SLICE.to_string(),
                template: SLICE_PROMPT,
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// Publishing: a local stage, which costs nothing.
#[must_use]
pub fn publish(ports: &Ports, config: &Config) -> Stage<SplitState> {
    Stage {
        name: PUBLISH.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Write {
                gh: Rc::clone(&ports.gh),
                slice_stage: SLICE.to_string(),
                base_branch: config.base_branch.clone(),
            })],
        },
    }
}

/// Context, slice, publish — in order.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<SplitState>> {
    vec![
        context(ports, config),
        slice(ports, config),
        publish(ports, config),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::config::fake as config_fake;
    use crate::split::ports::fake as ports_fake;

    #[test]
    fn the_order_of_the_table_is_context_then_slice_then_publish() {
        let names: Vec<String> = table(&ports_fake::ports(), &config_fake::config())
            .iter()
            .map(|stage| stage.name.clone())
            .collect();
        assert_eq!(names, [CONTEXT, SLICE, PUBLISH]);
    }

    #[test]
    fn the_prompt_carries_every_placeholder_the_action_fills() {
        for placeholder in ["{num}", "{title}", "{body}", "{existing}", "{inventory}"] {
            assert!(
                SLICE_PROMPT.contains(placeholder),
                "{placeholder} missing from SLICE_PROMPT"
            );
        }
    }
}
