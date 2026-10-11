//! Split: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** The order of entries *is*
//! the execution order: context, then the slice request, then publishing.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::prompts;
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

pub(crate) const SLICE_PROMPT: &str = r#"Milestone #{num} — "{title}":
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
`unit` — `concept`, `aggregate`, `data-capability`, `infrastructure`,
`capability`, `persisted-query`, `micro-ui`, `composition` or `tooling` —
the `system` it belongs to (the bounded context, e.g. `credit`), the
`concept` it implements when there is one (`Risk@3`), and for the data
layer the `aggregate` it writes or shapes (as its manifest's
`target.aggregate` names it) — a `data-capability` also its `effect`
(`insert`, `update`, `delete`, `upsert`) and the existing fields it
`touches` (empty for a pure insert). An `aggregate` slice is one
aggregate's structure whole — its `aggregate.json`, its invariants, the
migration that shapes its tables; `infrastructure` is the store's own
plumbing (DataQueue, DataGuard, Resolver); Rust that touches no store —
contract types, helpers, CI — is `tooling`.

**What a slice produces and needs is what orders the plan.** Every slice
lists in `produces` the ids its manifests will carry — an aggregate (`PJ`),
a DataCapability (`campagne.ajouterPj`), a Capability
(`campagne.listerPjs`), a Concept (`Risk@3`), a Micro-UI (`Pjs`), a screen
— and in `needs` the ids it builds on: the aggregate(s) a command writes or
a Capability reads, the Capabilities a Micro-UI reads and the
DataCapabilities it triggers, the Micro-UIs a screen composes, the Concept
a Capability implements. The loop chains a slice on the slice of this plan
that produces each id it needs, and on nothing else: an id no slice
produces is already in the repository. So cut for the graph, not for
layers — one slice per aggregate, one per DataCapability, one per
Capability action, one per Micro-UI, the screen last — and two slices that
share no id run in parallel. Never bundle "the data layer" into one slice:
two aggregates are two slices. `depends_on` (0-based indexes in this array)
is only for a wait `needs` cannot express, and should be rare.

{profile}

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
[{"title": "a short, specific title", "brief": "what this slice covers and does not cover", "branch": "feat/a-slug", "needs_human": false, "unit": "capability", "system": "credit", "concept": "Risk@3", "effect": null, "touches": [], "aggregate": null, "produces": ["credit.calculateRisk"], "needs": ["Account", "Risk@3"], "depends_on": []}]
```
An empty array `[]` means this milestone needs no further task."#;

/// What a profile asks of the cut: how much one slice may hold, for the
/// session that will build it.
#[must_use]
pub fn profile_block(profile: &str) -> String {
    match profile {
        "strong" => "The sessions that build these slices hold a lot (profile `strong`): an \
                     aggregate with its invariants and migration is one slice; a Micro-UI with \
                     its micro-frontend and its code is one slice; a Capability with its \
                     persisted query is one slice."
            .to_string(),
        "small" => "The sessions that build these slices hold little (profile `small`): cut \
                    finer, each slice verifiable on its own and chained by `needs` — an \
                    aggregate as its structure first, then its invariants, then its \
                    migration (each `produces` the same aggregate id suffixed: `PJ`, \
                    `PJ.invariants`, `PJ.migration`, the next `needs` the previous); a \
                    Micro-UI as its mocked micro-frontend first, then its code (the code \
                    `needs` the micro-frontend); a Capability as its persisted query first, \
                    then the Capability. More slices, more parallelism, more merges."
            .to_string(),
        _ => "The sessions that build these slices hold a fair amount (profile `mid`): an \
              aggregate with its invariants and migration is one slice, a Micro-UI with its \
              micro-frontend and its code is one slice, a Capability with its persisted \
              query is one slice — cut one of them in two only when its brief would not \
              fit one session."
            .to_string(),
    }
}

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
                template: config.prompts.text(prompts::SLICE, SLICE_PROMPT),
                spending: Rc::clone(&ports.spending),
                profile: config.profile.clone(),
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
                profile: config.profile.clone(),
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
        for placeholder in [
            "{num}",
            "{title}",
            "{body}",
            "{existing}",
            "{inventory}",
            "{profile}",
        ] {
            assert!(
                SLICE_PROMPT.contains(placeholder),
                "{placeholder} missing from SLICE_PROMPT"
            );
        }
    }
}
