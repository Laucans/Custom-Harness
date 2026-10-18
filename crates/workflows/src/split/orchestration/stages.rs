//! Split: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** The order of entries *is*
//! the execution order: context, then the slice request, then publishing.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::prompts;
use crate::split::action::actions::{AskForSlice, ReadDossier, ReadExistingTasks, ReadInventory};
use crate::split::action::publish::Write;
use crate::split::checks::gates::{SliceParses, SlicesCarryASpec};
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
the `system` it belongs to (the bounded context, e.g. `credit` — for a
Capability or a DataCapability, its id's namespace: `campaign.saveEntity` is
`campaign`; a DataCapability's `owner` is `dataguard`, never its system), the
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
share no id run in parallel. **One Micro-UI is one slice**, whatever
components it realises — never two slices on the same Micro-UI, and every
Micro-UI or screen slice names in `produces` the Micro-UI (or screen) it
builds, as the mock-up names it. The mock-up already built it
(`mock/components/<MicroUi>.html`, `mock/src/`): its slice mostly takes it
into `apps/<system>/<name>/` and connects it to the Capabilities and
DataCapabilities it names. Never bundle "the data layer" into one slice:
two aggregates are two slices. A slice that produces nothing orders
nothing: a `tooling` scaffold a system needs first `produces` the system's
id (`monde`) and the aggregates `need` it — or it is not a slice, and its
work goes into the first aggregate's. `depends_on` (0-based indexes in
this array) is only for a wait `needs` cannot express, and should be rare.

**A new system starts with its wiring.** When this milestone is the first
to build Capabilities or DataCapabilities of a system the repository does
not serve yet, cut first one `tooling` slice, branch `chore/wiring-<system>`:
the system's read crate (a read-only connection, no write API), the
server's routes that serve its Capabilities and submit its
DataCapabilities, its `queries/` and fixtures folders — the plumbing every
unit of the system needs and no unit owns, with no Capability,
DataCapability or rule of its own. It `produces` the system's id
(`<system>`) and every other slice of that system `needs` it. Left to the
first unit, this work stopped it mid-task. A wiring that changes the
DataGuard engine or its invariants is `infrastructure` instead.

**The rules' tests are already written.** The milestone's `## Rules`
section names the rules (`R<id>`, their criteria in `rules.json`) whose
tests under `tests/rules/` turn green once it lands. Name in each slice's
`rules` those its work completes — a rule whose test needs several slices
goes to the last of them — so that every rule of the milestone falls in
one slice. A slice never writes or changes a rule's test. A skeleton
milestone cuts `crates/server` (the one binary serving the wire protocol,
`GET /health`, the fixtures loaded from `FIXTURES_DIR`) as an
`infrastructure` slice and `apps/shell` (the frontend's host, `npm run
build` and `npm run serve`) as a `tooling` slice, at exactly those paths:
`scripts/rules-runtime.sh` starts them from there.

{profile}

{existing}

{inventory}

**The design dossier** — the milestone's part of what the factory's
creation decided:
<dossier>
{dossier}
</dossier>
Ids in `produces` and `needs`, a write's `effect` and `touches`, are copied
from it, never invented. A micro-frontend slice names its fragment and its
fixtures (`mock/fixtures/<Capability>.json`, one per Capability it needs).
An aggregate slice installs its draft `mock/data/aggregates/<Aggregate>.aggregate.json`
as `crates/dataguard/aggregates/<Aggregate>/aggregate.json` and copies its
tables from `mock/data/model.json` into `data/model.json` with its
migration — `mock/data/model.json` is the design and stays as it is; the
architecture gate refuses a build that touches it. **Number every aggregate slice's migration in the plan itself**,
in its `technical`: `crates/data/migrations/NNNN_<aggregate_snake>.sql`,
NNNN counting on from the migrations the inventory shows, one number per
slice, in foreign-key order — a table comes after every table it
references, so an aggregate that `needs` another takes the later number.
Slices on distinct aggregates run side by side: two of them inventing the
same number would collide at the merge. A slice whose unit uses a port the milestone has not built yet
builds it first, or a slice before it does: `crates/ports/<port>/` with its
trait, its fake and contract tests, then its adapter as ports.json names it.
Its manifest names the port under `ports`.

**Each slice is the whole SPEC its session builds from** — no one refines
it after you. `goal`: three to six sentences on what the user gets.
`acceptance`: the checks that say it is done, each one a given/when/then a
person can answer yes or no to; every rule the slice completes restated
with its criterion and cited `(R<id>)`. `business_rules`: the invariants
and rule criteria it must hold, from the dossier. `technical`: the paths,
manifests and contracts it opens, as the inventory and the layout name
them — a sketch; the session plans.

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
[{"title": "a short, specific title", "brief": "what this slice covers and does not cover", "branch": "feat/a-slug", "needs_human": false, "unit": "capability", "system": "credit", "concept": "Risk@3", "effect": null, "touches": [], "aggregate": null, "produces": ["credit.calculateRisk"], "needs": ["Account", "Risk@3"], "depends_on": [], "rules": ["R3"], "goal": "what the user gets", "acceptance": ["given …, when …, then … (R3)"], "business_rules": ["…"], "technical": "crates/credit/capabilities/calculate-risk/ with its capability.json …"}]
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
                    cost_note: config.cost_note.clone(),
                }),
                Box::new(ReadDossier {
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
            checks: vec![
                Box::new(SliceParses {
                    stage: SLICE.to_string(),
                }),
                Box::new(SlicesCarryASpec {
                    stage: SLICE.to_string(),
                }),
            ],
        }),
        body: StageBody::Session {
            turn_cap: None,
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
    fn a_new_system_is_cut_with_its_wiring_first() {
        // ForgeDesMondes #39, #41: the first DataCapability and Capability of
        // `ai` had to build the read crate and the server's routes mid-task.
        assert!(SLICE_PROMPT.contains("**A new system starts with its wiring.**"));
        assert!(SLICE_PROMPT.contains("`tooling` slice, branch `chore/wiring-<system>`"));
    }

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
            "{dossier}",
        ] {
            assert!(
                SLICE_PROMPT.contains(placeholder),
                "{placeholder} missing from SLICE_PROMPT"
            );
        }
    }
}
