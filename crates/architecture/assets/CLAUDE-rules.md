<!-- harness:architecture v1 -->
## Agent-native architecture — the rules every change is checked against

This repository follows the agent-native application architecture described in
`docs/ARCHITECTURE.md`. Read it once; these are the rules a diff is reviewed
against, and the CI gates in `.github/workflows/gates.yml` enforce the ones
that can be checked mechanically.

### Stack

- **Rust** for everything but the UI: Capabilities, the Data layer, the
  DataQueue, the DataGuard, the Resolver, the DataCapabilities. Edition 2024,
  one Cargo workspace, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo fmt --all -- --check` clean before every commit.
- **TypeScript** for the Micro-UIs and the screen composer, strict mode,
  `tsc --noEmit` and the project's lint clean before every commit. A Micro-UI
  calls Capabilities and DataCapabilities **by identifier only**: it never
  imports their code.
- **PostgreSQL** alone as the store: the DataQueue is a table, invalidations
  travel over `LISTEN/NOTIFY`, read models are views. No message broker.
  Locally it runs **in Docker**, provided by the harness and reached through
  `DATABASE_URL` / `DATABASE_URL_READONLY`: never install, start or
  configure a database yourself, and never create by hand what a migration
  would not recreate.
- **These rules bind every new unit, even where the existing code predates
  them.** A repository that began as one TypeScript program (a plugin, a
  static app) is the Micro-UI shell; every Capability, DataCapability or
  infrastructure unit it gains is a Rust crate at its place in the layout.
  When no server runs it, the crate is compiled to WebAssembly
  (`wasm-bindgen`, `wasm-pack`) and the TypeScript side loads it. Writing the
  unit in TypeScript because that is where the code is today is a violation,
  not a pragmatic choice.

### Layout

```
apps/<system>/<micro-ui>/          one Micro-UI (TypeScript) + micro-ui.json (contract D)
apps/screens/<screen>.json         one frozen screen composition (contract E)
crates/<system>/capabilities/<name>/   one Capability crate + capability.json (contract B)
crates/<system>/queries/<name>.graphql persisted queries, registered in queries/registry.json (contract C)
crates/<system>/<name>/            one infrastructure crate of the system (an adapter, a source, the Data layer's plumbing)
crates/dataguard/                  the write side: DataQueue, DataGuard, Resolver
crates/dataguard/data-capabilities/<name>/  one DataCapability + data-capability.json (contract F)
crates/dataguard/aggregates/<Aggregate>/aggregate.json   criticality, invariants, relations (contract I)
concepts/<Concept>/v<n>/concept.json + fixtures.json      a versioned Concept (contract A)
data/model.json                    the data model: what every table and field is for (contract M)
mock/pages.json                    the mock-up's pages, one per screen, and its micro-frontends (contract N)
mock/pages/<screen>.html           one mocked page: static HTML, the rest injected as micro-frontends
mock/components/<MicroUi>.html     one micro-frontend: the real Micro-UI bundled on the mock server, or a hand-written fragment before it is built
mock/drafts/<page>.json            on a mockup/* branch: the operations, forks and requests behind the page as the human rewrote it
mock/design/tokens.json + tokens.css   written on the design-system branch, carried onto main_agent on publish, read-only elsewhere: the tokens (W3C), one CSS variable each (contract O)
mock/kit/kit.json + kit/<Name>.html    written on the design-system branch, carried like the tokens: the kit a mock-up injects with data-kit, its look in kit/kit.css for the real frontend (contract O)
mock/src/server.js, screen.js      installed: the mock server (the shell's wire protocol from mock/fixtures/) and the screen host ($ctx.<name>)
mock/src/reads.js, commands.js     what the product teaches the server: a read per Capability, a judgement per DataCapability
mock/src/<MicroUi>.js, mock/build.sh   an entry per built Micro-UI (`source` in pages.json), bundled into mock/components/<MicroUi>.js, committed with its builtFrom
mock/fixtures/*.json               what the Capabilities would answer
mock/mock.js                       the loader: fills every data-component element from components/
scripts/dump-schema.sh             prints the schema the migrations build (pg_dump --schema-only)
contracts/*.schema.json            the seventeen contracts, JSON Schema 2020-12
rules.json                         the rules the product keeps, with their criteria (contract P)
tests/rules/business/  tests/rules/journey/   the rules' tests, written before the code (contract P)
scripts/rules-runtime.sh           installed, changed by nobody: starts the product for the rules' tests (serve, stop)
ports.json                         every dependency outside the product, behind a port (contract Q)
crates/ports/<port>/               a port: its trait, its contract tests, its `fake`
crates/ports/<port>/<provider>/    the adapter that wraps one provider
.sops.yaml  secrets/<env>.enc.yaml the secrets, encrypted with sops; names only in ports.json
crates/server/                     the one binary serving the shell's wire protocol and GET /health; reads PORT, DATABASE_URL, FIXTURES_DIR
apps/shell/                        the frontend's host: npm scripts build and serve (PORT, SERVER_URL), a frozen screen at /screens/<screen>
```

### Rules

1. **One unit per task.** A task builds exactly one unit of the architecture —
   the `## Architecture` section of its issue says which — and nothing beside
   it. Need a second unit? That is a second task.
2. **A Capability reads, and only reads.** It uses read-only credentials, reads
   through its persisted queries only, and never imports, invokes or
   references another Capability, a DataCapability, the DataQueue, the
   DataGuard or the Resolver. On the real dependency graph
   (`scripts/isolation.sh`, the gate): it reaches no other Capability or
   DataCapability, directly or transitively, and only crates of its own
   system; a crate it depends on is plumbing — it reaches no slice and no
   other system — never a bridge; no `#[path]` or `include!` leaves its
   folder. A DataCapability follows the same rule inside the DataGuard's
   system — and is a **folder**: `crates/<system>/data-capabilities/<name>/`,
   its library named after it, listed by glob in the workspace and
   discovered by the server's build script; adding one edits no shared file
   but the server's `[dependencies]` line (the gate checks it), so two land
   in parallel without meeting.
3. **A Capability's logic is local.** Shared *meaning* is a Concept with
   conformance fixtures, never a shared library of business code. Duplicate the
   computation; declare the Concept it implements; pass its fixtures.
4. **Every read is a persisted query**, declared in its Capability's manifest
   and registered by hash. No free-form query anywhere.
5. **Only DataCapabilities write**, always behind the DataGuard, always through
   the DataQueue. A DataCapability declares its `effect` and the existing
   fields it `touches`. `insert` with empty `touches` is the fast path and may
   be proposed by a read agent; everything else is the DataGuard agent's, and
   its pull request merges only once the agent review is clean and every gate
   green — the human is told, and may revert it.
6. **Invariants live in the DataGuard**, declared on the aggregate, each with
   tests. Never in a Capability, never in a Micro-UI.
7. **Every relation declares `onDelete` and `onUpdate`.** Deletion is a soft
   delete with a tombstone unless a human approves otherwise.
8. **A command is never rejected for concurrency.** It queues per aggregate,
   keeps its position, and its author confirms an overwrite. Relative commands
   need no confirmation. Parked commands expire after 24 hours, holds after 1
   minute unless renewed.
9. **Every value classifies itself**: derived (recomputed, carries its Concept
   version) or authoritative (canonical, mutated only through the DataGuard).
   Prefer derived when a stored value could be deduced from other rows.
10. **A Micro-UI declares `needs` and `actions`** and reads the Data through
    nothing else. Screens are composed from the registry and frozen in a
    versioned manifest; a frozen screen is replayed, never regenerated.
11. **The rules are tests, written first.** `rules.json` (contract P) and
    `tests/rules/` are the rules' task's (`harness:rules`), never a
    development's: a change makes the rules' tests it unblocks pass, without
    touching them. A rule's test you cannot meet — it contradicts another
    rule or the architecture — is **contested**: stop, and write
    `CONTEST.md` at the repository's root (the rule's id, the test, why it
    cannot hold) instead of changing it: the CI opens the issue the rules'
    task settles, and your change waits for its decision. The product is
    started for them by `scripts/rules-runtime.sh`, which nobody changes: the
    server is `crates/server` (the wire protocol, `GET /health`, the mock-up's
    fixtures loaded from `FIXTURES_DIR` at start), the frontend `apps/shell`
    (`npm run build`, `npm run serve`), both where the layout puts them.
12. **Contracts first.** A unit lands with its manifest or contract file and
    passes the gate that validates it. A change to a contract schema is its
    own task and lists the units it touches.
13. **Read side merges on green gates; write side after its review.** In the
    harness, `harness:read-side` tasks run in parallel and merge on their own;
    `harness:write-side` tasks run one at a time, open their pull request with
    `harness:to-review`, and the harness merges it once the agent review is
    clean and every gate green. Nothing waits on a human during development:
    the human is told what merged and may revert it.
14. **The data model is described in the task that changes it.** A migration
    that adds, renames or changes a table or a field updates `data/model.json`
    (contract M) in the same pull request: the field's `type` and `nullable`
    as the migration makes them, its `purpose` in a sentence, its business
    rules, the field it `references`. The structure is not copied there by
    hand for its own sake: after every merge the harness dumps the real schema
    (`scripts/dump-schema.sh`), archives it with the model on the branch
    `harness/data-model`, and compares the two — a divergence sends an agent
    to find out which side is wrong. Keep `scripts/dump-schema.sh` printing
    the schema the migrations build when the stack changes.
15. **Every screen is mocked before it is built, and stays mocked.** The
    mock-up under `mock/` is the product's frontend without a backend: one
    static HTML page per screen under `mock/pages/`, any file server runs it,
    no backend ever. `mock/pages.json` (contract N) lists every page with its
    objective, and every **micro-frontend**: a Micro-UI as the mock shows it,
    with `needs`, `actions` and `props` as contract D has them. A built
    Micro-UI is shown **as its real code**: an entry `mock/src/<MicroUi>.js`
    mounts it on the shell client over the mock server (`mock/src/server.js`,
    the shell's wire protocol answered from `mock/fixtures/`), bundled by
    `mock/build.sh` into `mock/components/<MicroUi>.js` and committed; a
    Micro-UI not built yet is a hand-written fragment
    `mock/components/<MicroUi>.html`. A page is static HTML plus the
    micro-frontends it injects: `<div data-component="<MicroUi>"
    data-props='{…}'></div>`, filled by `mock/mock.js`. What a Micro-UI
    renders names its parts (contract D): `data-writes="<DataCapability>"`
    on every actuator, `data-reads="<Capability>"` on every part that shows
    a read — one at least for each entry of `actions` and `needs`. A task that builds a
    Micro-UI replaces its fragment with the real code's entry and rebuilds
    the bundle; a task that adds a Capability or a DataCapability teaches the
    mock server its answer; a task that builds a screen composition ships the
    screen's page and lists it with `screen: <screen>`. A screen under
    `apps/screens/` with no page in `mock/pages.json`, a Micro-UI under
    `apps/` with no micro-frontend there or one whose `needs`/`actions`
    differ from its manifest, a listed micro-frontend with no fragment, one
    whose fragment or bundle names no `data-writes`/`data-reads` for an
    entry of its `actions`/`needs`, or a bundle not rebuilt by
    `mock/build.sh`, fails the gates. The human
    annotates the mock-up in the factory view and may push a `mockup/*`
    branch with the pages as they want them: read it before building the
    screen it names. Such a branch carries `mock/drafts/<page>.json` beside
    the page it rewrote — the operations the human made on the committed
    page (`move`, `resize`, `remove`, `clone`, `text`, `static`, `add`,
    `fork`), the micro-frontends it forked (`variantOf` in `pages.json`,
    the copy's fragment under `mock/components/`), and the requests it
    carries, each an intention written for the next roadmap: build the
    layout the page shows with clean code, not the operations one by one,
    and keep a forked micro-frontend a variant of its origin.

### What a session does before writing code

- Read `docs/ARCHITECTURE.md` and the `## Architecture` section of the issue.
- Find the unit's place in the layout above; create its manifest from the
  matching `contracts/*.schema.json` first, the code second.
- For a Micro-UI or a screen: open the mock-up page under `mock/pages/`
  first — it is the specification of what the screen looks like — and
  keep it true to what gets built.
- Run the gates locally where they exist (`npx ajv-cli validate` on the
  manifest, the isolation check), then the project's own tests.

16. **Everything outside the product goes through a port.** A third-party
    API, a SaaS, payment, mail, object storage, an LLM, identity, the
    product's own Postgres: each is a **port** in `ports.json` (contract Q),
    named by its business function (`payments`, never `stripe`), with the
    adapter that wraps the provider (`crates/ports/<port>/<provider>/`), the
    fake every test, the rules' runtime and the mock-up run on (the `fake`
    module of `crates/ports/<port>/`, beside the port's trait and its
    contract tests — the same tests for every adapter), the secrets it needs
    by name, its cost, and the units that use it (`usedBy`, matching each
    manifest's `ports`). No HTTP client, provider SDK or database client
    outside a port's adapter (Postgres: the Data layer, declared as the
    `database` port) — a gate refuses it. A provider that **keeps state** (a
    customer, a file, a user) is `stateful`: our Data is the truth, only a
    DataCapability writes through it and a Capability reads our mirror, never
    the provider; a Capability may call a stateless port (an LLM, a
    geocoder) directly. A port your task needs and `ports.json` has not is
    **not yours to add**: stop with `AGENT_LOOP_STOP: port <id> — <provider>,
    <cost>, <why>` and a human approves it. Secrets are never written in
    clear: `secrets/<env>.enc.yaml`, encrypted with sops (`.sops.yaml`), the
    values a human's to provide; tests never need them. Switching provider is
    a new adapter behind the same port, its contract tests and the rules'
    tests green.

<!-- /harness:architecture -->
