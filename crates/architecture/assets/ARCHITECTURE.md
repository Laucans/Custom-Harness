# Agent-native application architecture

Reference document · installed by `harness init-repo` · version 2 — the layout and the contracts; the engine's manual is `docs/DATA-INTEGRITY.md`

<!-- harness:architecture v2 -->

## Intent and principles

This architecture optimises the **parallelism of development agents** rather
than the reuse of code. The world of reading is massively parallelisable; truth
and mutation are centralised behind one control frontier.

It combines known patterns — Vertical Slice Architecture, CQRS and a command
bus, Generative UI, optimistic concurrency, human-in-the-loop — but aims at a
more precise property: **units of reading that agents can develop
independently, and data protected by a single layer.**

### Fundamental principles

1. A **Micro-UI** describes a need to read and declares the actions it can
   trigger.
2. **Capabilities** collect, transform and enrich. They never modify data
   directly.
3. A Capability is strictly **read-only**: it knows neither the other
   Capabilities nor any mutation infrastructure. Mutations are triggered by
   Micro-UIs through **DataCapabilities**, behind the **DataGuard**.
4. Capabilities are as independent as possible.
5. Code is reused *inside* a system; duplication *between* systems is accepted
   when it reduces coupling.
6. Consistency is about **meaning** (versioned Concepts with conformance tests),
   not about implementation.
7. The **Data** is the source of truth; every read goes through a persisted,
   declared query.
8. Every mutation goes through the **DataQueue**, then the DataGuard.
9. A pure addition follows a **fast path**; any mutation of existing data goes
   through the **Resolver**.
10. A command is never rejected **for concurrency**: it waits for its turn,
    and its author confirms whether it overwrites a value declared ahead of
    it. It is rejected at the head, and only there, when an invariant of
    the aggregate does not hold on the real state.
11. AI may propose a reconciliation but never decides a critical change alone.
12. Contracts are verified automatically in CI; that is what lets many agents
    work without a human review of every change.
13. A Capability knows only the data it reads and the Concepts it uses. It
    knows neither the other Capabilities nor any mutation infrastructure
    (DataCapability, DataQueue, DataGuard, Resolver).

## Overview

The system is cut in two worlds. On the left, **reading**: many independent
units, developed in parallel. On the right, **writing**: a single controlled
frontier, which owns the truth.

Micro-UIs and Capabilities never write directly. Micro-UIs trigger
DataCapabilities, whose commands go through the DataQueue. The **Integrity
DataGuard** sends its messages (confirmations, parked, applied) to the authors
and to the infrastructure components concerned; a Capability does not depend on
the DataGuard.

### Existing code and hosts without a server

The architecture is adopted by repositories that already exist, and some of
them run nowhere but inside a host written in TypeScript — an Obsidian or
VS Code plugin, a static web app. Neither changes the rules. The existing
program is the Micro-UI shell; what it already does stays where it is until
a task moves it. Every unit added from now on is built where the layout says
and in the language the stack says: Rust for Capabilities, DataCapabilities,
the Data layer and infrastructure, TypeScript for Micro-UIs. When no server
can run a Rust unit, it is compiled to WebAssembly and the host loads it; the
build and the loading step are part of the unit's task, and its manifest sits
at the same place as on a server. Such a task says so in its `## Architecture`
section (`host: hostless`) and its manifest, a DataCapability's (F) or a
Capability's (B), carries `"host": "hostless"`: it is never registered,
and its closure holds no server. What stays: the reads, the Concepts, the
mock-up, every gate but the server's. What does not: the DataQueue, the
holds, the credentials — a hostless program writes through its host's own
store, behind the same manifests.

## UI / Micro-UI

A Micro-UI is the **specification of a need**: what it displays says what must
be read, computed or enriched. It consumes Capabilities and declares the actions
(commands) it can trigger.

- **Reading**: the Micro-UI declares the Capabilities it needs (`needs`). It
  never reads the Data directly.
- **Writing**: a user action produces a command toward a DataCapability
  (`actions`). The Micro-UI then displays the command's lifecycle: queued,
  awaiting confirmation, applied, rejected.
- **Composition**: a screen is composed on demand by an AI from a user goal.
  The AI picks the Micro-UIs from a registry, through their description,
  without knowing the rest of the application.
- **Reproducibility**: a validated composition is frozen in a versioned
  manifest, then replayed as is. The screen is not regenerated on every
  visit, which makes it testable and deterministic.

Example: the goal "assess whether we can raise this customer's limit"
produces a screen composed of `RiskBadge`, `LatePaymentsList` and
`LimitReviewForm`.

### The mock-up

Every screen exists twice: once built, under `apps/`, and once **mocked**,
under `mock/` — a static frontend with no backend, that any file server
runs. One HTML page per screen under `mock/pages/`, plain HTML and
JavaScript, its data read from JSON fixtures under `mock/fixtures/`; no
build step, no framework the page must compile. `mock/pages.json`
(contract N) lists every page: its file, its title, the objective it was
created from, the screen it mocks once that screen is frozen, the
micro-frontends it injects.

A **micro-frontend** is a Micro-UI as the mock shows it, declared in
`mock/pages.json` with the `needs`, `actions` and `props` its manifest
(contract D) carries. Once the Micro-UI is built, the micro-frontend is
**its real code**: an entry `mock/src/<MicroUi>.js` mounts it on the shell
client over the **mock server** `harness init-repo` installs
(`mock/src/server.js`), which answers the shell's wire protocol in memory
and behaves as the DataGuard engine does — reads from `mock/fixtures/` or the
product's `mock/src/reads.js`; commands judged by the aggregate's invariants
(`mock/fixtures/aggregates/<Aggregate>.json`) and by `mock/src/commands.js`,
one queue per partition with the states of contract G (`queued`,
`awaiting_confirmation`, `parked`, `awaiting_review`, …), the messages of
contract L, a DataCapability nobody handles **rejected**; applied,
`dataVersion` bumped so every micro-frontend on the page refetches — with
the installed **screen host** (`mock/src/screen.js`: the `$ctx.<name>`
context a frozen screen composes with, contract E); `mock/build.sh` bundles
it into `mock/components/<MicroUi>.js`, committed with a `builtFrom` digest
of the code it came from, which the gates recompute. Before it is built, the micro-frontend is
a hand-written fragment, drawn from its spec. A page is static HTML plus the
micro-frontends it injects, each a
`<div data-component="<MicroUi>" data-props='{…}'></div>` that `mock/mock.js`
fills from the fragment. A human designs micro-frontends in the factory
view and drops them on pages; the Micro-UI task then builds what the
fragment shows, and replaces the fragment with the real code.

The mock-up is where the screen is **designed before it is built**: the
human creates a page from an objective, annotates its components, asks for
a simulation — a request whose result is shown by editing the page — and
may save the pages as they want them on a `mockup/*` branch. A task that
builds a Micro-UI or a composition reads the mock-up first and keeps it
true to what it builds. A screen under `apps/screens/` with no page in
`mock/pages.json` fails the gates.

## Capabilities

A Capability is an autonomous unit that produces a value or a view: it
collects, transforms, enriches or combines data. It is strictly read-only with
respect to business data. It knows only the data it reads and the Concepts it
uses; it knows neither the other Capabilities, nor the DataCapabilities, nor
the DataQueue, nor the DataGuard, nor the Resolver.

### System

A system is a **bounded context**: one owner, one namespace (`credit.*`) and a
local registry of Capabilities. This frontier tells an agent whether it should
look for an existing Capability or write a new one.

| Situation | Rule |
|---|---|
| A Micro-UI needs a Capability that exists, in any system | It is reused, by reference (`needs:`), never by importing code into the Micro-UI — reading is free |
| A Micro-UI needs an existing Capability to behave differently, and others use it | It does not change it: the Capability is duplicated into a new one (a new id, in the Micro-UI's system), and the new one is changed. A Capability nobody else uses is changed in place, by a new version of its contract |
| A Capability would need the behaviour of another Capability | It neither invokes nor imports it; it implements locally what it needs, reusing the same Concepts if necessary |
| A business rule shared between systems | Shared as a Concept with conformance tests, not as a library |
| A write-side validation or invariant | Never in a Capability: it lives in the DataGuard |

### Duplication

The problem is not duplicated code but **divergent meaning**. Duplicating
display or enrichment is harmless. Duplicating a business computation is
acceptable only if each copy declares the Concept it implements and passes its
conformance tests.

### Expected properties

- Read-only on business data, **guaranteed by read-only credentials**, not by
  convention.
- Reads declared as persisted queries.
- Every important value is returned with the version of its Concept and the
  version of the data read (`asOf`).
- A Capability depends on no other Capability: every piece of logic it needs is
  local to it. When a shared behaviour must stay semantically coherent, it is
  defined by a Concept and its conformance tests.
- Its cache is invalidated by the Data infrastructure when the data it depends
  on changes; the Capability does not subscribe to the DataGuard. **A cache is
  keyed by the authorisation context it was filled under** (role, scope): a
  value computed for one viewer is never served to another with fewer rights.

### Semantic consistency

What is shared between systems is **meaning, not code**. A versioned Concept
(`Risk@3`) fixes the definition, the rules, the examples and executable
conformance fixtures. Each implementation stays independent but must pass
them.

Concepts change rarely: that makes them a good central point, unlike a shared
library that many agents would modify.

### Derived or authoritative

Every value is classified explicitly:

| Kind | Examples | Can be recomputed locally | Mutation |
|---|---|---|---|
| Derived | `riskScore`, `customerAge`, `displayName` | Yes, if the Concept is respected | None: it is recomputed |
| Authoritative | `creditLimit`, `accountBalance`, `contractStatus` | No, it is read as canonical data | Only through a DataCapability and the DataGuard |

A stored authoritative value that could be deduced from other rows (a balance
updated at every payment) turns every addition into a modification. When
possible, model it as derived: adding a payment then stays a pure addition.

### Traceability

An important value carries the version of its Concept:
`{ "riskScore": 83, "riskLevel": "HIGH", "riskModel": "v3", "asOf": 1042 }`.
A deprecated Concept stays available as long as a Capability declares that it
implements it.

## Data and reading

The Data is the source of truth. It is exposed for reading through persisted
queries over read models; the technology matters less than the rule: **every
read is declared**.

### Persisted queries

A Capability invokes no other Capability. It sends no free-form query. It
declares only persisted queries, hashed and registered under its own
identifier. This registry makes it possible:

- to know exactly which Capabilities use which field;
- to compute the impact of a schema change before making it;
- for the Resolver to know who depends structurally on a table or a field.

The schema evolves by **expand/contract** migrations: add the new field,
migrate the Capabilities, then remove the old one.

### Read authorisation

Rights are not only about writing. The Data layer applies **row-level and
field-level filtering** (personal data, customer scope, role). This filtering
is never delegated to the Capabilities.

### Consistent read after write

An applied command returns a `dataVersion`. A read may require a version at
least equal to it, which lets the author see their own change despite the
asynchronous write. A read may also target a precise version, for audit or for
tests. **`dataVersion` is one global sequence**, issued by the single applier
(see below): partitions advance independently, but every applied command
takes the next number in that one sequence.

## Writing: DataCapabilities and the DataGuard

Only DataCapabilities write, and always behind the DataGuard. A Micro-UI action
references a DataCapability, which produces a command in the DataQueue. A
Capability has no dependency on the mutation system and references no
DataCapability.

### DataCapability

A DataCapability is a **typed command** that declares its effect — `insert`,
`update`, `delete` or `upsert` — and the existing fields it touches
(`touches`). It belongs to the DataGuard (`owner: dataguard`), not to the
Micro-UI that triggers it — but its **system** is its id's namespace:
`campaign.saveEntity` is of system `campaign`, under
`crates/campaign/data-capabilities/save-entity/`.

| DataCapability | Who may create it | Review |
|---|---|---|
| `effect: insert`, no `touches` | A read agent may propose it | Auto-approved if the CI gates pass |
| `update`, `delete`, `upsert`, or with `touches` | The DataGuard agent | Agent review and CI gates, merged one at a time; the human is told and may revert |

This split keeps the fast path open to parallel agents without opening the
mutation of existing data.

### Fast path: pure addition

An `insert` is a pure addition if it meets all these conditions:

- a new row with a new identifier, no upsert;
- no collision on a unique key;
- no trigger or side effect that writes into an existing row;
- no stored authoritative value that changes;
- outgoing foreign keys toward existing, non-archived rows.

The fast path skips the Resolver, not the DataGuard: invariants are still
checked, because an addition can violate a rule over several rows (sum of
reservations ≤ stock).

The classification is static (what the DataCapability declares), then dynamic:
if a collision or an unforeseen effect appears at execution, the command is
routed to the Resolver instead of failing.

### Responsibilities of the DataGuard

Authorisation and permissions, validation, business invariants, concurrency,
classification of the effect, orchestration of the Resolver and the DataQueue,
audit. **No write bypasses it.**

### Where the write side lives

Each write-side unit is a folder, and a folder is all a task adds:

```
crates/<system>/aggregates/<Aggregate>/       aggregate.json (I) + a crate `aggregate-<snake>`: its Judge, its projection, `plugin()`
crates/<system>/data-capabilities/<name>/     data-capability.json (F) + a crate named after the folder: its Prepare, `plugin()`
crates/<system>/capabilities/<name>/          a Capability's crate: CAPABILITY, SQL, VARIABLES, `answer`
crates/dataguard/registry/                    every unit, gathered; its Cargo.toml is written by scripts/register.sh
crates/dataguard/server/                      the engine planted with the registry: routes, applier, migration
crates/server/                                the product's binary: what it shares with its units, /health, its subcommands
```

A unit is registered by running `bash scripts/register.sh`, which rewrites
`crates/dataguard/registry/Cargo.toml` from the folders; nobody edits that
file by hand. A unit folder the registry does not name stops the build. A
hostless unit (`"host": "hostless"`) is never registered.
`crates/server` is installed once and is the product's from then on;
`crates/dataguard/server` and the registry's code are the harness's.

**One engine.** The engine, `harness-dataguard`, is taken by git at a
release tag, written once, in `crates/dataguard/server/Cargo.toml`. The
script copies that line into the registry and into the root `Cargo.toml`'s
`[workspace.dependencies]`; a unit that needs the engine (an aggregate's
`Judge`, a DataCapability's `Prepare`) takes it from there:

```toml
[dependencies]
harness-dataguard = { workspace = true }
```

Never a git line or a tag of its own: two sources build two engines, and
the server's `Plugin` is not the unit's. `--check` refuses it.

**Generated files are never merged by hand.** Two units added in parallel
meet in two files only: the registry's `Cargo.toml` and `Cargo.lock`. A
merge that conflicts on nothing else is resolved by regenerating them: take
the base's `Cargo.lock`, run `bash scripts/register.sh`, then
`cargo metadata --format-version 1 >/dev/null`, stage both and commit the
merge. The harness does it itself when it catches a milestone's branch up
with its base; a conflict on any other file is a real conflict.

## Resolver, DataQueue and concurrency

How a mutation of existing data is planned, queued, confirmed, parked,
applied and told — the Resolver, the DataQueue, the holds, the
confirmations, the messages — is the engine's manual:
`docs/DATA-INTEGRITY.md`. It is read by whoever builds the engine or an
aggregate, never by a Capability or a Micro-UI: for them, principle 10 and
contracts F, H and I say everything they need.

## Ports: what the product reaches outside

Everything outside the product — a third-party API, a SaaS, payment, mail,
object storage, an LLM, identity, and the product's own PostgreSQL — is
reached through a **port**, and `ports.json` (contract Q) lists them all:
what is outside, who provides it, what wraps it, what it costs, which
secrets it needs, and which units use it. Seeing the dependencies is the
first purpose; switching a provider the second; refusing the calls that
bypass a port is how the list stays true.

- **Named by function, never by provider.** `payments`, `mail`,
  `object-storage`, `llm`, `identity`, `database`. Changing Stripe for
  Mollie renames nothing.
- **A port is a crate, `crates/ports/<port>/`**: the trait the product
  calls, its **contract tests** (one suite every adapter of the port must
  pass), and a **fake** the tests, the CI, the rules' runtime and the
  mock-up run on — no secret, no cost, no network. An **adapter** wraps one
  provider in `crates/ports/<port>/<provider>/`.
- **Stateful or not.** A provider that keeps state (a customer, a file, a
  user) is `stateful`: our Data stays the truth. Only a DataCapability
  writes through the port, after the DataGuard, and the Capabilities read
  our mirror, never the provider. A stateless port (an LLM, a geocoder) may
  be called by a Capability directly. Each unit names its ports in its
  manifest (`ports`), and each port lists them back (`usedBy`).
- **PostgreSQL is a port, listed, not abstracted.** The Data layer keeps
  what PostgreSQL gives (the DataQueue as a table, `LISTEN/NOTIFY`, views):
  the port makes it visible, not interchangeable.
- **Secrets by name.** A port lists the secrets it needs; their values live
  encrypted with sops in `secrets/<env>.enc.yaml`, provided by a human.
  No agent ever sees one, and the tests never need one.
- **A new port is a human's decision.** The factory's design proposes the
  ports and the human chooses the providers; a port a development finds it
  needs is approved by a human before it exists.
- **Switching provider** is one task: a new adapter behind the same port,
  green on the port's contract tests and the rules' tests; for a stateful
  port, our state replayed to the new provider.

## Interface contracts

Fourteen contracts define every frontier. They live as **JSON Schema (draft
2020-12) in `contracts/`**, one file per contract; the examples below are YAML
for readability. Contracts A, B, F and I fix the frontier between reading and
writing: they are the first to stabilise.

| Contract | Frontier | Owner |
|---|---|---|
| A. Concept | Shared semantics | Semantic agent + human |
| B. Capability manifest | Capability ↔ registry and composer | Read agent |
| C. Persisted query | Capability ↔ Data | Read agent |
| D. Micro-UI manifest | Micro-UI ↔ Capabilities and actions | Read agent |
| E. Screen composition | User goal ↔ Micro-UIs | Composer agent |
| F. DataCapability | DataCapability ↔ DataGuard | DataGuard agent |
| G. DataQueue entry | Queued command | DataGuard |
| H. Command result | DataGuard ↔ caller | DataGuard |
| I. Aggregate: criticality, invariants, relations | Integrity rules | DataGuard agent + human |
| J. Impact plan | Resolver ↔ DataGuard | Resolver |
| K. Reconciliation proposal | AI ↔ approver | Integrity DataGuard |
| L. Message | Integrity DataGuard ↔ users and infrastructure components | Integrity DataGuard |
| M. Data model | Data ↔ the agents' understanding of it | Read agents, migrations |
| N. Mock pages | Mock-up ↔ screens | Human, UI agents |
| O. Design system and kit | Tokens and visual components ↔ every screen | Human, UI agents |
| P. Rules | What the product keeps ↔ the tests that prove it | Human (in the factory's design), rules' agent |
| Q. Ports | The product ↔ everything outside it | Human (in the factory's design, and every new port), the agents that build adapters |
| R. Plan graph | The milestones ↔ what each produces and consumes | The planner (writes it), the watch (the critical path, what runs side by side), the merge train |

### A. Concept

```yaml
concept: Risk
version: 3
kind: derived                 # derived | authoritative
definition: >
  Probability of the customer defaulting within 90 days, on a 0-100 scale.
output:
  riskScore: { type: integer, min: 0, max: 100 }
  riskLevel: { enum: [LOW, MEDIUM, HIGH], rule: "HIGH if score >= 70" }
inputs: [customer.payments, customer.outstandingBalance]
conformance: ./risk-v3.fixtures.json
deprecates: 2
```

### B. Capability manifest

```yaml
capability: customer.calculateRisk
system: credit
version: 1.4.0
description: "Computes the customer risk per Risk v3"   # read by the composer
implements: Risk@3
input:  { customerId: ID }
output: Risk@3 & { asOf: DataVersion }
reads:  [queries/customerPayments.graphql]
cache:  { ttl: 5m }
host:   server                # server (the default) | hostless
```

`host` says where the Capability runs, as a DataCapability's does (F): on
the product's server, the default when it is absent, or `hostless`, inside
a host without one, compiled to WebAssembly (*Existing code and hosts
without a server*). A hostless Capability is never registered:
`scripts/register.sh` and the registry's build script leave it out, and
no server answers it.

### C. Persisted query

```graphql
query CustomerPayments($id: ID!) @capability(id: "customer.calculateRisk") {
  customer(id: $id) { outstandingBalance payments(last: 24) { dueAt paidAt amount } }
}
```

### D. Micro-UI manifest

```yaml
microUi: RiskBadge
description: "Shows a customer's risk score and level"
needs:   [customer.calculateRisk]
actions: [credit.requestLimitReview]
props:   { customerId: ID }
```

What a Micro-UI renders says which part does what, in the markup's own
words: every actuator — a button, a form, a toggle that triggers a write —
carries `data-writes="<DataCapability>"`, and every part that shows what a
Capability answers carries `data-reads="<Capability>"` (several, space
separated, when one part does both). Every entry of `actions` has at least
one `data-writes` and every entry of `needs` at least one `data-reads`: the
factory view ties each part to its rules — the invariants a write meets,
what a read returns — and a gate checks the micro-frontend carries them.
The real code names the unit through its identifiers, never a literal
outside them (`'data-writes': AJOUTER_PJ.dataCapability`).

### E. Screen composition

```yaml
screen: credit-review
generatedFrom: "Assess whether we can raise this customer's limit"
version: 7                    # frozen once validated, replayed as is
layout:
  - RiskBadge:        { customerId: $ctx.customerId }
  - LatePaymentsList: { customerId: $ctx.customerId }
  - LimitReviewForm:  { customerId: $ctx.customerId }
```

### F. DataCapability

A folder, `crates/<system>/data-capabilities/<name>/`: `data-capability.json`
and a crate whose package and library are named after the folder. The
crate exposes `pub fn plugin() -> Plugin`, its `Prepare` keyed by the id
(`docs/DATA-INTEGRITY.md` has the code), and depends on the engine from
the workspace and on its target aggregate's crate, on no other unit.

```yaml
dataCapability: credit.requestLimitChange
owner: dataguard
version: 2
effect: update                # insert | update | delete | upsert
target: { aggregate: Account, id: $accountId }
touches: [Account.creditLimit]   # empty + insert ⇒ fast-path candidate
payload: { newLimit: Money, reason: string }
mode: confirm_on_stale        # confirm_on_stale | overwrite | relative
invariants: [limit-under-ceiling, limit-change-requires-risk]
permissions: [credit.limit.write]
callableBy: [credit.*]
idempotencyKey: required
```

### G, H. DataQueue entry, command result

In `docs/DATA-INTEGRITY.md`. H is what every caller sees: five fields —
`commandId`, `status`, `dataVersion`, `violations`, `reviewId`.

### I. Aggregate: criticality, invariants, relations

A folder named after the aggregate, `crates/<system>/aggregates/<Aggregate>/`,
in the system its DataCapabilities share: `aggregate.json` and a crate,
package `aggregate-<snake>`, library `aggregate_<snake>` (`AiSettings`:
`aggregate_ai_settings`). The crate exposes `pub fn plugin() -> Plugin`
with its `Judge` (the invariants as code) and its `Project` or `ProjectPg`
(what follows a write: a read table, the product's own tables), and keeps
its tests in its `tests/`, one per invariant id.

```yaml
aggregate: Account
fields:
  creditLimit:  { kind: authoritative, criticality: critical, review: role:credit-manager }
  contactEmail: { kind: authoritative, criticality: normal }
  tags:         { kind: authoritative, criticality: low, merge: set }
invariants:
  - id: limit-under-ceiling
    rule: "creditLimit <= policy.maxLimit(riskLevel)"
    onViolation: reject
  - id: limit-change-requires-risk
    rule: "Risk@3 computed less than 24h ago"
    onViolation: human_review
relations:
  - { from: Payment.accountId, onDelete: restrict }
  - { from: Document.subject, semantic: true, onDelete: review }
holds: { maxTtl: 1m, renewable: true }
```

### J, K, L. Impact plan, reconciliation proposal, message

In `docs/DATA-INTEGRITY.md`.

### N. Mock pages

```yaml
version: 1
pages:
  - path: pages/credit-review.html
    title: Credit review
    objective: "Assess whether we can raise this customer's limit"
    screen: credit-review          # once the composition is frozen
    components: [RiskBadge, LatePaymentsList, LimitReviewForm]
components:
  - name: RiskBadge
    system: credit
    description: "Shows a customer's risk score and level"
    fragment: components/RiskBadge.html
    fixture: fixtures/RiskBadge.json
    needs: [credit.calculateRisk]
    actions: [credit.requestLimitReview]
    props: { customerId: ID }
```

### O. Design system and kit

On the `design-system` branch, one for the product. `mock/design/tokens.json`
holds the design tokens (W3C design tokens format); `mock/design/tokens.css`
is derived from it, one CSS variable per token (`--color-primary`), and every
screen, Micro-UI and kit component reads its colours, fonts, spacings, radii
and shadows through those variables, never through a literal. `mock/kit/`
holds the **kit**: visual primitives below the Micro-UI (a button, a menu, a
carousel), each a fragment `kit/<Name>.html` a page or a micro-frontend
injects with `<div data-kit="<Name>">`, with no data of its own and no
network — what loads from a server is a micro-frontend. A kit component is
reused as it is by the real frontend when it passes the kit's static check
(no network, no external script or style, colours only through `var(--…)`);
otherwise it is recoded for real and the fragment rewritten to match, with
one line under `recoded`.

The design system has **one writer**. It is written on `design-system` only,
and when the human publishes it the harness carries `mock/design/` and
`mock/kit/` whole onto `main_agent`, in one commit that touches nothing
else. A branch in progress finds them on its next sync with `main_agent`,
and since it never changes them, no merge ever conflicts on them — a gate
(`design-system-single-writer`) refuses a pull request that does. The real
frontend loads `mock/design/tokens.css` and `mock/kit/kit.css` (every
fragment's `<style>`, derived on publish, scoped under the fragment's root
class `kit-<name>`) once, from the shell; a Micro-UI writes the kit's
markup and classes with its own data, so a token changed or a component
generated again restyles the product without touching a screen.

```yaml
# mock/design/design.json
version: 1
state: validated                   # none | candidate | validated
source: generated                  # injected | generated (from a moodboard and a logo)
brief: "warm, artisanal, speaks to game masters"
moodboard: [moodboard/table.png, moodboard/ink.jpg]
logo: logo.svg
candidate: false                   # a candidate.json waits beside the validated tokens
validatedAt: 2026-10-10T18:00:00Z
issue: https://github.com/…/issues/42   # the roadmap task that installs it
# mock/kit/kit.json
version: 1
checked: [button, input, menu, card]   # the harness's bases ticked
components:
  - name: Menu
    base: menu
    title: Navigation menu
    what: "A horizontal navigation bar with a brand, links and the current one marked."
    state: generated               # pending | running | generated | failed
  - name: HeroBanner
    prompt: "A full-width banner with the campaign's title over its map"
    reference: HeroBanner.reference.png
    state: generated
    recoded: "the banner reads its map from the shell's image service"
```

### P. Rules

`rules.json`: the rules the product must keep, as the human validated them
in the factory's design — each a step of a user's **journey** or a
**business** rule (an invariant, a computation), with the **criterion** a
test proves it by, in three parts (given, when, then), and the pages and
micro-frontends that carry it.

```yaml
version: 1
rules:
  - id: R3
    title: A campaign archived leaves the list
    family: business                  # journey | business
    criterion:
      given: a campaign in the list
      when: the DM archives it
      then: it is no longer listed
    pages: [pages/campaigns.html]
    components: [CampaignList]
    # state: conform                  # the rule changed: its tests ahead of the code
```

**The rules are proved before they are built.** One task — the rules'
task, labelled `harness:rules`, the first of the roadmap — writes every
rule's tests before any development, from the criteria alone, and lands
them; every development after conforms to them:

- a **business** rule's test is Rust, in `tests/rules/business/tests/`; a
  **journey** rule's is a browser test (Playwright) in `tests/rules/journey/`.
  Both suites are their own, and reach the product only the way a Micro-UI
  does — **by identifier**, over the shell's wire protocol
  (`POST /capabilities/<id>`, `POST /commands`, `GET /commands/<id>`) and by
  screen name — through the kit installed beside them;
- every test names its rule (`rule("R3", needs, …)`), the units it needs
  (`data-capability:ArchiveCampaign`, `capability:ListCampaigns`,
  `micro-ui:…`, `screen:…`, `aggregate:…`) and copies the criterion as it is,
  in three comment lines `given: …`, `when: …`, `then: …`;
- a test is **pending** while a unit it needs is not built (no manifest names
  it) or the product is not served yet: it passes, and says so. Once
  everything it needs exists, it runs and must hold;
- the product is served by `scripts/rules-runtime.sh`, installed whole and
  changed by nobody (the gate *Rule tests untouched*). It starts what the
  layout fixes: `crates/server`, the one binary serving the wire protocol
  and `GET /health` (it reads `PORT`, `DATABASE_URL`, and `FIXTURES_DIR`,
  the mock-up's fixtures it loads as it starts), on a scratch database the
  migrations build; then `apps/shell`, the frontend's host (`npm run build`,
  `npm run serve` on `PORT`, reaching the server at `SERVER_URL`, a frozen
  screen at `/screens/<screen>`). Until `crates/server` exists it serves
  nothing; never the mock server, which would make every rule hold without
  the product;
- the data a test starts from is the mock-up's (`mock/fixtures/`), loaded by
  the runtime, plus what a `given` asks;
- a development never writes `rules.json` or `tests/rules/`: a test it cannot
  meet is **contested**, and the rules' task settles it. A rule changed after
  the start is rewritten with its tests by the rules' task and marked
  `conform` until a task brings the code to it.

### Q. Ports

```json
{"version": 1, "ports": [{
  "id": "payments", "purpose": "charge the subscription", "stateful": true,
  "provider": "Stripe", "tech": {"kind": "sdk", "name": "async-stripe", "version": "0.39"},
  "adapter": "crates/ports/payments/stripe", "fake": "crates/ports/payments",
  "secrets": ["STRIPE_SECRET_KEY"], "cost": "1.5% + 0.25€ per charge",
  "usedBy": ["billing.chargeSubscription"]
}]}
```

### R. Plan graph

`plan/graph.json`, written by the planner from the design dossier and
updated as the plan moves: every milestone with the ids it produces and
consumes, every id with its kind, its producer and the state of its
contract — `planned`, `mocked` (its manifest and fixture exist, consumers
build against them), `built`, `evolving`. A milestone that consumes what a
later milestone produces is a plan error the planner reports. The critical
path — the heaviest chain of remaining work — is read from it, and a free
lane serves it before anything else.

The graph is the **register of contracts**: an id a milestone of the plan
produces counts as declared for every gate — a Micro-UI needs it, triggers
it, a task's closure holds it — before its crate exists, and the fixture
`mock/fixtures/<id>.json` stands for it (the mock server answers a read from
it, applies a planned write with no effect). Two milestones thus build side
by side against the same interface. A milestone changes the contracts it
produces and no other's (gate `contracts-untouched`): a consumer that needs
a contract to change stops with `AGENT_LOOP_STOP: contract <id> — <what it
needs>`, and the contract manager evolves it for every consumer at once.

```json
{"version": 1,
 "milestones": [{"number": 20, "title": "Sessions and scenes", "systems": ["session"],
   "produces": ["Session", "session.addScene", "session.listScenes", "SceneBoard"],
   "consumes": ["campaign.listCharacters", "media.listMedia"], "depends_on": [18, 19]}],
 "ids": {"session.listScenes": {"kind": "capability", "producer": 20, "state": "mocked", "version": 1}}}
```

## CI gates

Contracts protect only if they are verified automatically. These gates block
any agent contribution that violates them. They live in
`.github/workflows/gates.yml`, and every one of them runs: a rule the product
cannot keep yet is a task like any other, never a gate turned off.

| Gate | Checks | Contracts |
|---|---|---|
| Semantic conformance | `implements: Risk@3` ⇒ the Concept's fixtures pass | A, B |
| Contracts untouched | A pull request changes the manifests of the ids its milestone produces, and no other's | B, F, R |
| Read-only | No Capability has write access: lint and read-only credentials | B |
| Capability isolation | No Capability imports, invokes or references another Capability or any mutation infrastructure | B |
| Queries | Every persisted query is valid against the current schema and the next | C |
| Schema impact | A schema change lists the Capabilities touched and blocks until each is migrated | C, I |
| System frontier | `needs:` — a Micro-UI references only Capabilities that exist or the plan declares, in any system; a behaviour change of a shared one is a duplicate, never an edit | B |
| Write actions | `actions:` references existing DataCapabilities, with compatible `callableBy` and `permissions`; no Capability has an `invokes` field | D, F |
| Effect classification | An `insert` DataCapability without `touches` contains no update, upsert or trigger | F |
| Ownership | An `update`, `delete`, `upsert` DataCapability, or one with `touches`, requires a DataGuard and the agent review of the write side | F, I |
| Invariants | Every declared invariant has tests; every DataCapability lists the ones it may affect | F, I |
| Units placed | A DataCapability sits under the system its id names, an aggregate under its system in a folder named after it, never under `crates/dataguard/` (`data-capability-placed`, `aggregate-placed`) | F, I |
| Units registered | The registry's `Cargo.toml` and the workspace's engine line are what `scripts/register.sh` writes; every unit is a workspace member by glob, its library named as the registry's glue calls it, the engine taken from the workspace (`units-registered`) | B, F, I |
| Relations | Every foreign key has a declared `onDelete` and `onUpdate` policy | I |
| Compositions | Every Micro-UI of a frozen screen exists in the referenced version | D, E |
| Pages mocked | `mock/pages.json` validates; every page and every micro-frontend it lists exists; every frozen screen has a page; every Micro-UI has a micro-frontend whose listing says its needs and actions; the bundles are rebuilt from the Micro-UIs' code | D, E, N |
| Ports declared | Every port a unit names is in `ports.json`, every port lists its users back, a Capability never reads a stateful port, every port has its fake and contract tests (`scripts/ports-gate.mjs declared`, `contracts`) | Q, B, F |
| Externals behind ports | No HTTP client, provider SDK or database client outside a port's adapter (the Data layer and the DataGuard for PostgreSQL), no network call from a Micro-UI (`behind`) | Q |
| Secrets encrypted | Only sops-encrypted `secrets/<env>.enc.yaml` and no `.env` file are committed (`secrets`) | Q |
| Rules covered | Every rule of `rules.json` has a test of its family naming it, its criterion copied as it is; no test names a rule `rules.json` has not (`scripts/rules-gate.mjs coverage`) | P |
| Rules hold | Every rule's test whose units are built runs on the product the runtime serves, and passes; each rule's state (pending, green, red) is kept as `rules-status.json`; a rule marked `conform` may be red in a task, never where a milestone lands (`scripts/rules-gate.mjs status`) | P |
| Rule tests untouched | A change that is not the rules' task leaves `rules.json` and `tests/rules/` alone (`scripts/rules-gate.mjs untouched`), and carries no `CONTEST.md` — which opens the `Rules contested` issue the rules' task settles (`contest`) | P |

Beside the gates, the project's own CI (`.github/workflows/ci.yml`) judges
the code and its tests, not the contracts: format and lint; the tests a
change can break (`scripts/test-scope.sh`); the dead code the compiler lets
through — a dependency no crate uses (`cargo machete`), a `pub` item no
crate of the workspace calls (`scripts/dead-code.sh`); and the tests
themselves, by mutation — every function the change touches is rewritten
wrong by `cargo mutants`, and a rewrite no test catches fails the build
(`.cargo/mutants.toml` says what is never mutated). A session runs the
same checks through `scripts/quiet.sh`: one line when a check passes, every
line of its errors when it fails, the full log kept — the CI keeps its whole
logs. The code no change
touches is judged in rotation: each run mutates one bucket of files, the
bucket a hash of the path, so runs in parallel judge disjoint files
(`scripts/mutants-rotation.sh`); what it misses never fails the build, it
becomes a `harness:test-debt` issue per file (`scripts/test-debt.sh`).
Dead code is removed, not allowed; a missed mutant is answered with a test.

## Organisation of the agents

Many agents in parallel on the read side, few agents and an agent review on the
truth side, one change at a time. During development no review waits on a
human: each is told what landed, and acts after the fact.

| Agent | Number | Produces | Review |
|---|---|---|---|
| Read agents | Many, one per system or per Capability | Capabilities, persisted queries, Micro-UIs, pure-addition DataCapabilities | CI gates only |
| Semantic agent | One | Concepts, conformance fixtures | Agent review and CI gates; the human is told |
| DataGuard agent | One | Mutating DataCapabilities, invariants, relations, schema, migrations | Agent review and CI gates, one at a time; the human is told and may revert |
| Composer agent | At run time or at design time | Screen compositions | Validation before freezing |
| Reconciliation AI | At run time | Reconciliation proposals | Human approver |

Example of a conflict-free split: Customer Risk, Orders, Revenue, Customer
Timeline and Dashboard developed by five read agents in parallel, without
touching a shared component outside the Concepts and the DataGuard.

In the harness that drives this repository, a task on the read side carries
`harness:read-side` and runs in parallel with its peers; a task on the write
side carries `harness:write-side`, runs alone, and its pull request is merged
by the harness once the agent review is clean and every gate green.

## Technology choices

Kept as simple as the rules allow:

- **Rust** for everything but the Micro-UIs: the Capabilities, the Data layer,
  the DataQueue, the DataGuard, the Resolver, the DataCapabilities. One
  workspace, one crate per Capability, one crate for the write side.
- **TypeScript** for the Micro-UIs and the composer: thin, declarative, calling
  Capabilities and DataCapabilities by identifier only.
- **PostgreSQL alone** as the store: the DataQueue is a table (one row per
  command, `SELECT … FOR UPDATE SKIP LOCKED` per partition), `LISTEN/NOTIFY`
  carries the invalidations, the read models are views, the persisted queries
  are files in the repository registered by hash. No Kafka, no NATS.
- Development data: one isolated database per agent (a branch, a schema, a
  container); in production, one `main` fed by the DataQueue.

## Known limits and open questions

### Limits

- **Invariants not expressible in SQL.** A rule over several rows or over a
  derived value must be executed by the DataGuard; no database constraint
  covers it.
- **Write latency.** The asynchronous write, the holds and the human review
  suit a back office, not real-time transactional work.
- **Point of concentration.** The DataGuard, the schema and the Concepts
  concentrate the risk and the review load. It is intended, but their
  throughput bounds that of the agents as soon as an evolution touches the
  write side.

### Decided

- Partitions: **one per aggregate**.
- Default durations: holds **1 minute**, renewable; parked commands **24
  hours**.
- Relative commands: **exempt from confirmation**; invariants checked at
  application.
- Contracts: **JSON Schema**, in `contracts/`.
- DataQueue and read layer: **PostgreSQL only**, as above.

<!-- /harness:architecture -->
