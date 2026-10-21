<!-- harness:architecture v5 -->
## Agent-native architecture — the rules every change is checked against

This repository follows the agent-native architecture of
`docs/ARCHITECTURE.md` (layout and contracts; `docs/DATA-INTEGRITY.md` is
the write side's manual). Each rule below names the gate of
`.github/workflows/gates.yml` that enforces it. The detail of one unit —
its folder, its manifest, what it may know, its gates — is its **rule
card**, injected into the prompt of the task that builds it
(`--- ARCHITECTURE RULES ---`).

### Stack

- **Rust** for everything but the UI, one Cargo workspace, edition 2024;
  `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --all -- --check` clean before every commit.
- **TypeScript** (strict) for the Micro-UIs and the screen composer;
  `tsc --noEmit` and the lint clean. A Micro-UI calls Capabilities and
  DataCapabilities **by identifier only**, never by importing their code.
- **PostgreSQL** alone: the DataQueue is a table, invalidations travel over
  `LISTEN/NOTIFY`, read models are views, no broker. It runs **in Docker**,
  provided by the harness (`DATABASE_URL` / `DATABASE_URL_READONLY`): never
  install, start or configure a database yourself.
- **These rules bind every new unit**, even beside older code: a unit is a
  Rust crate at its place in the layout; with no server to run it, it is
  compiled to WebAssembly (`host: hostless`). Writing it in TypeScript
  because the code around it is TypeScript is a violation.

### Layout

```
apps/<system>/<micro-ui>/               a Micro-UI + micro-ui.json (D)
apps/screens/<screen>.json              a frozen screen composition (E)
apps/shell/                             the frontend's host (npm build, serve)
crates/<system>/capabilities/<name>/    a Capability + capability.json (B)
crates/<system>/queries/<name>.graphql  persisted queries, queries/registry.json (C)
crates/<system>/<name>/                 a system's plumbing (adapter, source, read crate)
crates/<system>/data-capabilities/<name>/  a DataCapability + data-capability.json (F), under its id's system
crates/<system>/aggregates/<Aggregate>/  an aggregate's crate + aggregate.json (I): Judge, projection, plugin()
crates/data/                            the Data layer
crates/dataguard/server/  registry/     the engine planted with every unit; the registry register.sh writes
crates/server/                          the one binary: the shell's wire protocol, GET /health
crates/ports/<port>/[<provider>/]       a port (trait, fake, contract tests) and its adapters
crates/platform/conformance/            the Concepts' conformance runner
concepts/<Concept>/v<n>/                concept.json + fixtures.json (A)
data/model.json                         the data model (M)
mock/                                   the mock-up: pages.json (N), pages/, components/, src/, design/ and kit/ (O)
rules.json  tests/rules/                the product's rules and their tests (P)
ports.json  .sops.yaml  secrets/        the ports (Q) and their encrypted secrets
contracts/*.schema.json                 the seventeen contracts
scripts/                                register.sh, queries.sh, isolation.sh, dump-schema.sh, rules-runtime.sh
```

### Rules

1. **One unit per task**, the one the issue's `## Architecture` names.
   A second unit is a second task.
2. **A Capability reads, and only reads**: read-only credentials, its
   persisted queries, its system's read crate (itself on `crates/data`, the
   one read-only connection — no database driver outside the Data layer and
   the DataGuard); it reaches no other
   Capability, DataCapability or system, and no slice is listed by name in
   the workspace. Gates: `capability-isolation`, `read-only`,
   `workspace-members-by-glob`, `units-registered`.
3. **A Capability's logic is local**: shared meaning is a Concept with
   fixtures, never a shared library. Gates: `concepts-documented`,
   `semantic-conformance`.
4. **Every read is a persisted query**, registered by hash
   (`bash scripts/queries.sh`). Gate: `queries-registered`.
5. **Only DataCapabilities write**, behind the DataGuard, through the
   DataQueue, declaring `effect` and `touches`. Gates:
   `effect-classification`, `ownership`.
6. **Invariants live in the DataGuard**, declared on the aggregate, each
   with a test naming its id. Gate: `invariants-tested`.
7. **Every relation declares `onDelete` and `onUpdate`**; deletion is a
   soft delete with a tombstone unless a human approves.
8. **A command is never rejected for concurrency**: it queues per
   aggregate, keeps its position, its author confirms an overwrite;
   relative commands need no confirmation; parked commands expire after
   24 hours, holds after 1 minute unless renewed.
9. **Every value classifies itself**, derived or authoritative; prefer
   derived when other rows could recompute it.
10. **A Micro-UI declares `needs` and `actions`** and reads through nothing
    else; a frozen screen is replayed, never regenerated. Gates:
    `system-frontier`, `write-actions`, `compositions-resolve`.
11. **The rules are tests, written first**: `rules.json` and `tests/rules/`
    are the rules' task's (`harness:rules`). A rule's test you cannot meet
    is contested in `CONTEST.md`, never edited. Gates: `rules-hold`,
    `rule-tests-untouched`.
12. **Contracts first**: a unit lands with its manifest, valid against
    `contracts/`. A schema change is its own task. Gate:
    `manifests-conform-to-contracts`.
13. **Read side merges on green gates; write side after its agent review**
    (`harness:to-review`). Nothing waits on a human; they may revert.
14. **The data model is described in the task that changes it**: a
    migration updates `data/model.json` in the same pull request; the
    harness compares it with the real schema after every merge.
15. **Every screen is mocked before it is built, and stays mocked**
    (`mock/pages.json`); a built Micro-UI is shown as its real code. Read a
    `mockup/*` branch before building the screen it names. Gate:
    `pages-mocked`.
16. **Everything outside the product goes through a port** of `ports.json`,
    named by its business function, with its fake and contract tests;
    secrets are encrypted with sops. A port your task needs and `ports.json`
    lacks is not yours to add: stop with
    `AGENT_LOOP_STOP: port <id> — <provider>, <cost>, <why>`. Gates:
    `ports-declared`, `externals-behind-ports`, `secrets-encrypted`.
17. **A milestone changes only the contracts it produces** (`plan/graph.json`,
    contract R): a `capability.json` or `data-capability.json` another
    milestone produces is read, mocked by its fixture, never edited. Need it
    changed: stop with `AGENT_LOOP_STOP: contract <id> — <what you need>`;
    the contract manager evolves it for every consumer. Gate:
    `contracts-untouched`.
18. **A unit is its folder.** Adding an aggregate, a DataCapability or a
    Capability edits no shared file: write its folder, then run
    `bash scripts/register.sh`. `crates/dataguard/registry/Cargo.toml` and
    `Cargo.lock` are generated, never edited or merge-resolved by hand: on
    a conflict there, take the base's `Cargo.lock`
    (`git checkout --theirs Cargo.lock` while merging the base in), rerun
    the script and `cargo metadata --format-version 1 >/dev/null`, stage
    both. A unit that needs the engine takes it from the workspace:
    `harness-dataguard = { workspace = true }`. Gates: `units-registered`,
    `aggregate-placed`, `data-capability-placed`.

### What a session does before writing code

- Read the issue's `## Architecture` and the unit's rule card in the prompt;
  open `docs/ARCHITECTURE.md` only where the card sends you, or when no
  card was injected.
- The working tree may be **narrowed** to the unit's closure (the prompt's
  `--- WORKING TREE ---`). A file the plan names and the tree lacks: stop
  with `AGENT_LOOP_STOP: missing <path>`; the harness widens it.
- Manifest first, from its `contracts/*.schema.json`, then the code.
- For a Micro-UI or a screen, open its page under `mock/pages/` first.
- Run the gates locally where they exist (the manifest's `ajv` validation,
  `scripts/isolation.sh`), then the project's tests.
<!-- /harness:architecture -->
