<!-- harness:architecture v2 -->
## Agent-native architecture — the rules every change is checked against

This repository follows the agent-native application architecture described in
`docs/ARCHITECTURE.md` (the layout and the contracts; the engine's manual,
`docs/DATA-INTEGRITY.md`, is the write side's alone). Read it once; these are
the rules a diff is reviewed against, and the CI gates in
`.github/workflows/gates.yml` enforce every one of them.

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
  (`wasm-bindgen`, `wasm-pack`) and the TypeScript side loads it; the task
  says `host: hostless` and the manifest `"host": "hostless"`, and no
  server dependency line is asked of it. Writing the
  unit in TypeScript because that is where the code is today is a violation,
  not a pragmatic choice.

### Layout

```
apps/<system>/<micro-ui>/          one Micro-UI (TypeScript) + micro-ui.json (contract D)
apps/screens/<screen>.json         one frozen screen composition (contract E)
crates/<system>/capabilities/<name>/   one Capability crate + capability.json (contract B)
crates/<system>/queries/<name>.graphql persisted queries, registered in queries/registry.json (contract C)
crates/<system>/<name>/            one infrastructure crate of the system (an adapter, a source, the Data layer's plumbing)
crates/dataguard/                  the write side: DataQueue, DataGuard, Resolver — the DataGuard's own system
crates/<system>/data-capabilities/<name>/  one DataCapability + data-capability.json (contract F); its system is the DataGuard's (`dataguard`)
crates/<system>/aggregates/<Aggregate>/aggregate.json   criticality, invariants, relations (contract I), under the same system
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
scripts/queries.sh                 writes queries/registry.json from the Capabilities' reads (--check in CI)
crates/platform/conformance/       the conformance runner, a dev-dependency of every Capability that implements a Concept
contracts/*.schema.json            the fifteen contracts, JSON Schema 2020-12
```

### Rules

1. **One unit per task.** A task builds exactly one unit of the architecture —
   the `## Architecture` section of its issue says which — and nothing beside
   it. Need a second unit? That is a second task.
2. **A Capability reads, and only reads.** (And every slice is a workspace
   member **by glob**, never by name — `crates/*/capabilities/*` — so a
   checkout narrowed to one slice still resolves, and adding one edits no
   shared file; the gate `workspace-members-by-glob` refuses a slice listed
   by name.) It uses read-only credentials, reads
   through its persisted queries only, and never imports, invokes or
   references another Capability, a DataCapability, the DataQueue, the
   DataGuard or the Resolver. On the real dependency graph
   (`scripts/isolation.sh`, the gate): it reaches no other Capability or
   DataCapability, directly or transitively, and only crates of its own
   system; a crate it depends on is plumbing — it reaches no slice and no
   other system — never a bridge; no `#[path]` or `include!` leaves its
   folder; it holds no database driver of its own (`sqlx`, `tokio-postgres`,
   `diesel`, …): it reads through its system's read crate, `<system>-read`,
   whose connection has no write API — read-only by the types, the read-only
   role as defence in depth. A dev-dependency counts for none of this.
   A DataCapability follows the same rule inside the DataGuard's
   system — and is a **folder**: `crates/<system>/data-capabilities/<name>/`,
   its library named after it, listed by glob in the workspace and
   discovered by the server's build script; adding one edits no shared file
   but the server's `[dependencies]` line (the gate checks it), so two land
   in parallel without meeting.
3. **A Capability's logic is local.** Shared *meaning* is a Concept with
   conformance fixtures, never a shared library of business code. Duplicate the
   computation; declare the Concept it implements; pass its fixtures — from a
   test of the Capability calling `platform_conformance::check("<Concept>@<n>",
   |input| …)` (`crates/platform/conformance`, a dev-dependency), over
   `concepts/<Concept>/v<n>/fixtures.json`: `{"concept", "version", "cases":
   [{"name", "input", "output"}]}`. The gate `semantic-conformance` runs it.
4. **Every read is a persisted query**, declared in its Capability's manifest
   and registered by hash: the query file opens with `# fields: <entity>.<field>,
   …`, `bash scripts/queries.sh` writes `queries/registry.json`, committed with
   the query; the gate `queries-registered` refuses a registry that is not what
   the script writes. No free-form query anywhere.
5. **Only DataCapabilities write**, always behind the DataGuard, always through
   the DataQueue. A DataCapability declares its `effect` and the existing
   fields it `touches`. `insert` with empty `touches` is the fast path and may
   be proposed by a read agent; everything else is the DataGuard agent's and a
   human reviews it.
6. **Invariants live in the DataGuard**, declared on the aggregate, each with
   a test naming its id in a crate of the aggregate's own system. Never in a
   Capability, never in a Micro-UI.
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
11. **Contracts first.** A unit lands with its manifest or contract file and
    passes the gate that validates it. A change to a contract schema is its
    own task and lists the units it touches.
12. **Read side merges on green gates; write side waits for a human.** In the
    harness, `harness:read-side` tasks run in parallel and merge on their own;
    `harness:write-side` tasks run one at a time and their pull request is
    merged only once a human poses `harness:approved` on the task, after the
    agent review and a green CI.
13. **The data model is described in the task that changes it.** A migration
    that adds, renames or changes a table or a field updates `data/model.json`
    (contract M) in the same pull request: the field's `type` and `nullable`
    as the migration makes them, its `purpose` in a sentence, its business
    rules, the field it `references`. The structure is not copied there by
    hand for its own sake: after every merge the harness dumps the real schema
    (`scripts/dump-schema.sh`), archives it with the model on the branch
    `harness/data-model`, and compares the two — a divergence sends an agent
    to find out which side is wrong. Keep `scripts/dump-schema.sh` printing
    the schema the migrations build when the stack changes.
14. **Every screen is mocked before it is built, and stays mocked.** The
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

- Read the `## Architecture` section of the issue, and the rule card of its
  unit: the harness injects it into the prompt (`--- ARCHITECTURE RULES ---`)
  — where it lives, its manifest, what it may know, the gate that judges it.
  `docs/ARCHITECTURE.md` is the reference behind the card: open it only where
  the card sends you there, or when no card was injected.
- The working tree a harness session runs in may be **narrowed** to the
  unit's closure (`git sparse-checkout`): its folder, the plumbing it
  reaches, the manifests it needs, its mock page — the prompt's
  `--- WORKING TREE ---` block lists what is there. A file the plan names
  and the tree does not hold is not yours to go looking for: stop with
  `AGENT_LOOP_STOP: missing <path>`, and the harness widens the closure.
  Nothing else changes: the index and the history are whole, branch, commit
  and push work as usual.
- Find the unit's place in the layout above; create its manifest from the
  matching `contracts/*.schema.json` first, the code second.
- For a Micro-UI or a screen: open the mock-up page under `mock/pages/`
  first — it is the specification of what the screen looks like — and
  keep it true to what gets built.
- Run the gates locally where they exist (`npx ajv-cli validate` on the
  manifest, the isolation check), then the project's own tests.
<!-- /harness:architecture -->
