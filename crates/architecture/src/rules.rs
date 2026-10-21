//! The rule card of each unit: what a session building that one unit must
//! know of the architecture, and nothing else.
//!
//! A task builds one unit. The whole architecture document is eight hundred
//! lines, most of them about units the task does not touch, and a session
//! that reads it pays for it on every turn — or skips it, and the gates
//! tell it afterwards. A card is the forty lines that apply: where the unit
//! lives, what its manifest says, what it may know and must not, and the
//! gate that judges it (`assets/gates.yml`, by job name). The loop injects
//! the card of the declared unit into the prompt of the stages that plan
//! and build it (`--- ARCHITECTURE RULES ---`).
//!
//! The cards say what the gates check, in the gates' words: a rule a card
//! states and no gate checks is advice, and the card says so.

use crate::vocabulary::Unit;

const CAPABILITY: &str = "\
UNIT: Capability — a read-only unit that collects, transforms or enriches
data for one system (bounded context).

Where it lives: crates/<system>/capabilities/<name>/, one Cargo crate named
after its folder, with capability.json beside Cargo.toml (contract B,
contracts/b-capability-manifest.schema.json). The workspace lists it by
glob. Adding one edits no shared file: write the folder, then run
`bash scripts/register.sh`, which rewrites crates/dataguard/registry/
Cargo.toml (never edited by hand); commit it and Cargo.lock with the unit.

Its manifest (capability.json): `capability` (`<system>.<name>`, the id a
Micro-UI's `needs` cites), `system` (its folder), `version`, `description`
(what the composer reads), `implements` (`Risk@3`), `input`, `output`,
`reads` (its persisted queries, contract C, under crates/<system>/queries/),
`cache`.

Its crate exposes four items, which the registry's glue turns into the
server's read: `pub const CAPABILITY: &str` (its id), `pub const SQL: &str`
(its query, run in a READ ONLY transaction), `pub const VARIABLES: &[&str]`
(the request's variables, bound to $1, $2, … in order) and `pub fn
answer(rows: &Value, variables: &BTreeMap<String, Value>) -> Value`. No
engine dependency: it never names harness-dataguard.

What it may know: the data it reads, and the Concepts it implements. Its
logic is local: duplicate a computation rather than share a library. What
it must not know: any other Capability, any DataCapability, the DataQueue,
the DataGuard, the Resolver — no import, no call, no `invokes` field, no
dependency on their crates. It depends only on its own system's plumbing,
reads through its system's read crate (`<system>-read`, on crates/data),
never a database driver of its own (sqlx, tokio-postgres, postgres, diesel,
sea-orm, rusqlite, mysql), no write statement, no DATABASE_URL, no `#[path]`,
`include!` or symlink that leaves its folder.

Its reads are registered: after adding or changing a query file (first line
`# fields: <entity>.<field>, …`), run `bash scripts/queries.sh` and commit
queries/registry.json. Teach the mock server its answer (mock/src/reads.js).
A Concept it implements is documented (concepts/<Concept>/v<n>/concept.json,
fixtures.json beside it) and proved by a test calling
`platform_conformance::check(\"<Concept>@<n>\", |input| …)`.

Gates that judge it: manifests-conform-to-contracts (B), capability-isolation
(scripts/isolation.sh on the cargo metadata graph, and no `invokes`),
units-registered (named in the registry the script writes),
concepts-documented, semantic-conformance, queries-registered, read-only,
contracts-untouched (its manifest is its milestone's to change: a consumer
that needs it changed stops with `AGENT_LOOP_STOP: contract <id> — <what it
needs>`).
The loop checks the same after the round, and that the round touched
nothing outside the unit's closure.
Run them before the PR: `npx --yes ajv-cli@5 validate --spec=draft2020 -s
contracts/b-capability-manifest.schema.json -d <manifest>`,
`bash scripts/isolation.sh`, `bash scripts/register.sh --check` and
`bash scripts/queries.sh --check`.
Outside the product, through a port only: ports-declared (never a stateful
port) and externals-behind-ports. The rules (contract P): rule-tests-untouched
(a development conforms or writes CONTEST.md) and rules-hold.";

const MICRO_UI: &str = "\
UNIT: Micro-UI — a screen fragment (TypeScript) that declares what it reads
and what it can trigger. It never reads the Data directly.

Where it lives: apps/<system>/<micro-ui>/, with micro-ui.json beside the
code (contract D, contracts/d-micro-ui-manifest.schema.json).

Its manifest (micro-ui.json): `microUi` (its id), `system`, `description`,
`needs` (Capability ids of any system, by reference — never by importing
their code; a Capability others use that must behave differently is
duplicated into a new id, never changed), `actions` (DataCapability ids it may trigger), `props`.

What it renders names its parts: every actuator (button, form, toggle that
writes) carries data-writes=\"<DataCapability>\"; every part that shows what
a Capability answers carries data-reads=\"<Capability>\" — one at least per
entry of `actions` and of `needs`. The real code names the unit through its
identifiers, never a literal outside them.

It is mocked before it is built, and stays mocked (contract N,
mock/pages.json): a micro-frontend entry under `components` with the same
needs/actions/props, a hand-written fragment mock/components/<MicroUi>.html
before it is built, then its real code mounted on the mock server
(mock/src/<MicroUi>.js, bundled by mock/build.sh into
mock/components/<MicroUi>.js, committed with its builtFrom digest). Open the
mock page under mock/pages/ first: it is the screen as the human wants it (a
`mockup/*` branch may carry a newer one).

It styles itself through the design system only: mock/design/tokens.css
(one CSS variable per token, never a literal colour, font, spacing, radius
or shadow) and the kit's classes (mock/kit/<Name>.html, contract O). Both
are read-only on your branch: a kit component that cannot serve is recoded
on `design-system`, never here.

Gates that judge it: manifests-conform-to-contracts (D), system-frontier
(every `needs` is a Capability that exists or the plan declares), write-actions (every
action is an existing DataCapability whose callableBy admits it),
pages-mocked (listed, fragment present, parts named, listing in sync,
bundle rebuilt), design-system-single-writer.
No network call from a Micro-UI: externals-behind-ports. The rules
(contract P): rule-tests-untouched (rules.json and tests/rules/ are the
rules' task's, a development conforms or writes CONTEST.md) and rules-hold (a journey rule's test passes once every unit
it needs is built).";

const CONCEPT: &str = "\
UNIT: Concept — a versioned definition shared across systems: the meaning,
the rules, the examples, and executable conformance fixtures.

Where it lives: concepts/<Concept>/v<n>/concept.json (contract A,
contracts/a-concept.schema.json) and fixtures.json beside it.

Its document: `concept` (the name), `version`, `kind` (derived |
authoritative), `definition`, `output` (the fields and their rules),
`inputs`, `conformance` (the fixtures file), `deprecates` (the version it
replaces, kept available while a Capability still declares it).

What is shared is meaning, not code: a Capability that implements
`Risk@3` passes these fixtures with its own local logic. The fixtures file
is `{\"concept\": \"Risk\", \"version\": 3, \"cases\": [{\"name\", \"input\",
\"output\"}, …]}`: every case is an input and the output every
implementation must answer, compared as JSON, run by
crates/platform/conformance from a test of the Capability. Concepts change
rarely; a new version is a new folder, never an edit of the old one.

What a Concept must not do: name a Capability, a table, a crate, or a
mutation. It is read by everything and depends on nothing.

Gates that judge it: manifests-conform-to-contracts (A),
concepts-documented (every `implements: <Concept>@<n>` of a Capability
finds this file), semantic-conformance (the fixtures run against each
implementation, from its own test).";

const DATA_CAPABILITY: &str = "\
UNIT: DataCapability — a typed command, the only thing that writes, always
behind the DataGuard and through the DataQueue.

Where it lives: crates/<system>/data-capabilities/<name>/ — one Cargo crate
whose package and library are named after its folder (`save-entity`,
`save_entity`), with data-capability.json beside Cargo.toml (contract F).
Its system is its id's namespace (campaign.saveEntity sits in
crates/campaign/data-capabilities/save-entity/); its `owner` is always
`dataguard`, which runs it and houses it nowhere. Adding one edits no
shared file: write the folder, then run `bash scripts/register.sh`, which
rewrites crates/dataguard/registry/Cargo.toml (never edited by hand: a
folder it does not name stops the build); commit it and Cargo.lock with
the unit. When no server runs it (`host: hostless`, `\"host\":
\"hostless\"` in the manifest), it is compiled to WebAssembly for its host
and never registered.

Its code: `pub fn plugin() -> Plugin` returning `Plugin::new().prepare(\"<id>\",
YourPrepare)`. Its `Prepare` (harness_dataguard::plugins) turns the
submission into the payload and row it writes (`Prepared`), or refuses it
before anything is queued (`Refusal::not_found`, 404; `Refusal::unprocessable`,
422). What it reads from the server's `Shared` it declares with
`.needs::<T>()`: the server refuses to start while a need is unfilled.
Its dependencies: the engine from the workspace, `harness-dataguard = {
workspace = true }` (never a git line or tag of its own: one engine, one
`Plugin` type), and the aggregate crate its `target.aggregate` names. No
other unit.

Its manifest: `dataCapability` (`<system>.<name>`), `owner: dataguard`,
`version`, `effect` (insert | update | delete | upsert), `target` (the
aggregate and its id), `touches` (the existing fields it changes — empty
with `insert` is the fast path; never a derived field), `payload`, `mode`
(confirm_on_stale | overwrite | relative), `invariants` (ids its aggregate
declares), `permissions`, `callableBy` (Micro-UI ids, a system, or `*`),
`idempotencyKey`.

Fast path or not: an `insert` touching nothing is a pure addition, merged on
green gates, and its code holds no UPDATE, ON CONFLICT, UPSERT, DELETE or
CREATE TRIGGER. Anything else is the write side: its PR is reviewed by an
agent, and milestone_merge merges it once the review is clean and every
check green. Teach the mock server its judgement (mock/src/commands.js). A
port that keeps state is written through a DataCapability only. What it
must not know: any Capability, any other DataCapability, any other system's
crates.

Gates that judge it: manifests-conform-to-contracts (F),
data-capability-placed (its folder is its id's namespace, never
crates/dataguard/), capability-isolation (its target aggregate's crate only),
units-registered (named in the registry, its library named after its folder,
the engine from the workspace), effect-classification, ownership (its
aggregate declares every field it touches, none derived), invariants-tested,
write-actions (the Micro-UIs that cite it are admitted by callableBy),
contracts-untouched (a consumer that needs its manifest changed stops with
`AGENT_LOOP_STOP: contract <id> — <what it needs>`).
Run `bash scripts/register.sh --check` and `bash scripts/isolation.sh` before
the PR. Outside the product, through a port only: ports-declared and
externals-behind-ports. The rules (contract P): rule-tests-untouched and
rules-hold.";

const PERSISTED_QUERY: &str = "\
UNIT: Persisted query — a declared, hashed read a Capability goes through.
No free-form query exists anywhere.

Where it lives: crates/<system>/queries/<name>.graphql (or .sql), named in
the `reads` of the Capability's manifest, registered in
queries/registry.json (contract C, contracts/c-persisted-query.schema.json)
under the Capability's id with its hash.

What it is for: the registry says which Capability reads which table and
field, so a schema change knows its readers (expand/contract migrations:
add the new field, migrate the readers, remove the old one) and the
Resolver knows who depends structurally on a row.

What it must not do: write, or read through anything but the read-only
credentials. It belongs to one Capability; two Capabilities needing the
same read each declare their own. Its first line declares what it reads:
`# fields: customer.outstandingBalance, payment.dueAt` (`-- fields:` in
SQL); `bash scripts/queries.sh` then writes the registry entry — the
operation name, the file, its sha256, the fields — and the registry is
committed with the query.

Gates that judge it: manifests-conform-to-contracts (C, on
queries/registry.json), queries-registered (the registry is what the
script writes: every read a file, every file registered, every hash
current). queries-valid (valid against the current schema and the next)
and schema-impact are declared in gates.yml and switched on with the
DataGuard engine.";

const COMPOSITION: &str = "\
UNIT: Screen composition — Micro-UIs composed for one user goal, frozen in
a versioned manifest and replayed as is.

Where it lives: apps/screens/<screen>.json (contract E,
contracts/e-screen-composition.schema.json): `screen`, `generatedFrom`
(the goal), `version` (frozen once validated), `layout` (the Micro-UIs and
their props, `$ctx.<name>` for what the screen host provides).

It is mocked before it is built: one page mock/pages/<screen>.html, static
HTML plus the micro-frontends it injects
(`<div data-component=\"<MicroUi>\" data-props='{…}'></div>`), listed in
mock/pages.json with `screen: <screen>` (contract N). Open the page first:
it is the screen as the human wants it, and a `mockup/*` branch may carry a
newer one with mock/drafts/<page>.json saying what they changed. Build the
layout the page shows with clean code, not the draft's operations one by
one; a forked micro-frontend (`variantOf`) stays a variant of its origin.

What it may know: the Micro-UIs it lays out, by id, in the version it
names. What it must not do: reach a Capability or a DataCapability itself —
the Micro-UIs do — or restyle anything outside the design system's
variables and the kit's classes.

Gates that judge it: manifests-conform-to-contracts (E),
compositions-resolve (every Micro-UI it lays out exists), pages-mocked
(a page names it in mock/pages.json), design-system-single-writer.";

const AGGREGATE: &str = "\
UNIT: Aggregate — one aggregate's structure and code: its aggregate.json,
its invariants as code with their tests, its projection, and the migration
that shapes its tables. Write side: its PR is reviewed before it merges.

Where it lives: crates/<system>/aggregates/<Aggregate>/, a folder named
after it and one Cargo crate: package `aggregate-<snake>`, library
`aggregate_<snake>` (AiSettings: `aggregate-ai-settings`,
`aggregate_ai_settings`). It holds aggregate.json (contract I,
contracts/i-aggregate.schema.json), its code in src/, its tests in the
crate's tests/. Its system is the one the DataCapabilities that target it
share, never crates/dataguard/. Adding one edits no shared file: write the
folder, then run `bash scripts/register.sh` (it rewrites
crates/dataguard/registry/Cargo.toml, never edited by hand); commit it and
Cargo.lock with the unit. The migration goes under the DataGuard's
migrations, and data/model.json is updated in the same PR (contract M).

Its code: `pub fn plugin() -> Plugin` returning
`Plugin::new().judge(\"<Aggregate>\", YourJudge)`, plus `.project(…)` (a
`Project`, what follows a write in memory) or `.project_pg(…)` (a
`ProjectPg`, its PostgreSQL tables written inside the transaction that
applies the write; the engine's `postgres` feature). `Judge::violated`
answers the ids of the invariants the row after the command breaks;
`ProjectionError::Refused` rejects the command and rolls all of it back.
The engine comes from the workspace: `harness-dataguard = { workspace =
true }`, never a source of its own.

Its document: `aggregate`, `fields` (each `kind` derived | authoritative,
`criticality` critical | normal | low with the review role for a critical
one, `merge` set | last-writer), `invariants` (each an `id`, a `rule`, an
`onViolation` reject | human_review), `relations` (each `from`, `onDelete`
restrict | cascade | nullify | archive, `semantic` when the link is outside
a foreign key), `holds` (maxTtl, renewable).

Rules: every invariant has a test naming its id, in the crate's tests/;
every relation declares onDelete; deletion is a soft delete with a
tombstone unless a human approves otherwise; prefer a derived value to a
stored authoritative one that other rows could recompute. Two tasks on one
aggregate run one after the other: the loop holds it while you build.

What it must not do: compute a business value (that is a Concept), be read
by a Micro-UI (that is a Capability's job), or depend on another unit.

Gates that judge it: manifests-conform-to-contracts (I, M),
aggregate-placed (its system's folder, named after it), units-registered
(named in the registry, its library `aggregate_<snake>`), invariants-tested,
ownership (the DataCapabilities that mutate it name fields it declares, none
derived), capability-isolation. schema-impact is declared and switched on
with the engine. After the merge the harness dumps the real schema and
compares it with data/model.json: a divergence opens a diagnosis.";

const INFRASTRUCTURE: &str = "\
UNIT: Infrastructure — the plumbing itself: the DataQueue, the DataGuard,
the Resolver, the Data layer, a system's shared crate. Write side: its PR is
reviewed before it merges.

Where it lives: crates/dataguard/ for the write side's server
(crates/dataguard/server/, the engine planted with every unit of
crates/dataguard/registry/, whose Cargo.toml scripts/register.sh writes),
crates/server/ for the product's binary, crates/<system>/<name>/ for one
system's own plumbing (an adapter, a source, the Data layer's reading of its
tables). The engine itself, harness-dataguard, is the harness's: taken at a
release tag, never read or edited here.

What it holds: authorisation and permissions, validation, the business
invariants' execution, concurrency (one FIFO partition per aggregate, a
single applier, a global dataVersion), the classification of a command's
effect, the Resolver's impact plan, the audit journal, the messages to
authors (contracts G, H, J, K, L in contracts/). PostgreSQL alone: the
DataQueue is a table, LISTEN/NOTIFY carries invalidations, read models are
views. Locally it runs in Docker through DATABASE_URL /
DATABASE_URL_READONLY — never install or configure a database yourself.

A crate a slice depends on is plumbing, never a bridge: it reaches no
Capability, no DataCapability, and no crate of another system. Row- and
field-level read filtering lives here, never in a Capability; tombstones
are filtered here.

Gates that judge it: capability-isolation (scripts/isolation.sh: a shared
crate reaches no slice and stays in its system), read-only (a Capability
reaches no application role's connection), units-registered.
Keep scripts/dump-schema.sh printing the schema the migrations build.
A port is infrastructure: ports-declared (every port has its fake and its
contract tests, and lists the units that use it), externals-behind-ports
(a provider is reached from its adapter under crates/ports/<port>/<provider>/
only) and secrets-encrypted (its secrets committed encrypted with sops, or
not at all). Switching provider is a new adapter behind the same port, its
contract tests and the rules' tests green.";

const TOOLING: &str = "\
UNIT: Tooling — Rust that touches no store: contract types, shared helpers,
CI and build tooling. Read side: nothing it ships mutates data.

Where it lives: a crate under crates/<system>/<name>/ when it serves one
system, or under crates/platform/<name>/ when every system may use it;
scripts under scripts/, workflows under .github/workflows/.

What it may do: define types the contracts' JSON maps to, validate a
manifest, run a gate locally, build a bundle. What it must not do: open a
database connection, read or write business data, or become a bridge — a
crate a Capability depends on reaches no slice and no other system
(scripts/isolation.sh).

A change to a contract schema (contracts/*.schema.json) is its own task
and lists the units it touches. A change to a gate (.github/workflows/
gates.yml) keeps the loop's own reading of the same rule true.

Gates that judge it: capability-isolation when a slice depends on it,
workspace-members-by-glob when it touches the root Cargo.toml (slices are
listed by glob, never by name); the project's own CI (format, lint,
test-scope, dead code, mutants) for the rest.
The rules' own task, never a development, answers to rules-covered
(every rule of rules.json has its test, its criterion copied as it is).";

/// The rule card of `unit`: the forty lines a session building it needs.
#[must_use]
pub const fn card(unit: Unit) -> &'static str {
    match unit {
        Unit::Capability => CAPABILITY,
        Unit::MicroUi => MICRO_UI,
        Unit::Concept => CONCEPT,
        Unit::DataCapability => DATA_CAPABILITY,
        Unit::PersistedQuery => PERSISTED_QUERY,
        Unit::Composition => COMPOSITION,
        Unit::Aggregate => AGGREGATE,
        Unit::Infrastructure => INFRASTRUCTURE,
        Unit::Tooling => TOOLING,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unit_has_a_card_that_names_it_and_a_gate() {
        for unit in Unit::ALL {
            let text = card(unit);
            assert!(text.starts_with("UNIT: "), "{unit:?}");
            assert!(text.contains("Gates that judge it"), "{unit:?}");
            assert!(text.contains("Where it lives"), "{unit:?}");
        }
    }

    #[test]
    fn the_cards_are_distinct_and_each_one_is_short() {
        let mut seen = std::collections::HashSet::new();
        for unit in Unit::ALL {
            let text = card(unit);
            assert!(seen.insert(text), "{unit:?} shares a card");
            let lines = text.lines().count();
            assert!(
                (20..=60).contains(&lines),
                "{unit:?}: {lines} lines — a card, not the document"
            );
        }
    }

    #[test]
    fn every_gate_a_card_names_is_a_job_of_the_installed_gates() {
        let gates = include_str!("../assets/gates.yml");
        let jobs: Vec<&str> = gates
            .lines()
            .filter(|line| {
                line.starts_with("  ") && !line.starts_with("   ") && line.ends_with(':')
            })
            .map(|line| line.trim().trim_end_matches(':'))
            .collect();
        assert!(jobs.contains(&"capability-isolation"), "{jobs:?}");
        for unit in Unit::ALL {
            let text = card(unit);
            let after = text
                .split("Gates that judge it:")
                .nth(1)
                .expect("the gates paragraph");
            // The first sentence names the gates; what follows is how to run
            // them, with flags and file names that look like job names.
            let end = [after.find(". "), after.find(".\n")]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(after.len());
            let after = &after[..end];
            let named: Vec<&str> = after
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .filter(|word| {
                    word.contains('-') && word.chars().all(|c| c.is_ascii_lowercase() || c == '-')
                })
                .collect();
            assert!(!named.is_empty(), "{unit:?} names no gate");
            for gate in named {
                let known = jobs.contains(&gate)
                    || matches!(
                        gate,
                        "test-scope"
                            | "cargo-mutants"
                            | "dead-code"
                            | "mutants-rotation"
                            | "expand-contract"
                    );
                assert!(
                    known,
                    "{unit:?} names `{gate}`, which is not a job of gates.yml"
                );
            }
        }
    }

    #[test]
    fn the_write_side_cards_say_the_pr_is_reviewed_before_it_merges() {
        for unit in [Unit::Aggregate, Unit::Infrastructure] {
            let flat = card(unit).split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                flat.contains("its PR is reviewed before it merges"),
                "{unit:?}"
            );
        }
        assert!(card(Unit::DataCapability).contains("milestone_merge"));
    }
}
