# DataGuard vertical cut, then the ForgeDesMondes migration

The execution plan for the design reviewed on 2026-10-20: the products' write
side cut by unit. Phases A1–A5 change the harness. Phases M0–M5 move
ForgeDesMondes onto the result. The cut applied to the harness itself
(workflows, launcher, notify, view) is parked, not part of this plan.

## Goal

A session that adds one unit (an aggregate, a DataCapability, a Capability)
writes only in that unit's folder and reruns at most one script. A forgotten
registration is a compile error or a gate failure, never a silent hole. Two
parallel sessions never resolve a hub file by hand.

## Preconditions (2026-10-20 evening)

- The ForgeDesMondes watch is stopped, by the owner's order. It stays stopped
  until M5. No product session runs meanwhile, except the migration's own.
- The open product PRs are merged first: #321 (milestone #23 → `main_agent`),
  `milestone/25` caught up with `main_agent`, then #312 (task #186).
- Harness `main` is green on CI (clippy 1.99, Linux runner).
- Lint with the rustup toolchain, never Homebrew's:
  `PATH=$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH`.

Because the factory is stopped, no layout version is needed. ForgeDesMondes
never sees A5's gates and rule cards before it is migrated. The factory
restarts on the new harness only at M5, once the product matches the new
layout.

## Design adjustments to the reviewed plan

1. **Generated files are never merged by hand.** Every new unit still
   changes two shared files: the registry `Cargo.toml` and `Cargo.lock`.
   Both are regenerated. The harness's catch-up (`common::catch_up`, its
   local merge `branching::catch_up_locally`) and the `pr_fix` conflict path
   rerun `scripts/register.sh`, then `cargo metadata`, on a conflict limited
   to generated files (since H1: also `queries/registry.json` and the
   mock-up's bundles, one table in `harness_architecture::generated`), and
   commit the result. A conflict anywhere else stays a real conflict.
2. **The proof is parallel.** A5's verification creates two branches, each
   adding one DataCapability, and merges them into one base. The merge must
   need nothing but the regeneration of point 1.
3. **No silent `Shared` hole.** A `Plugin` declares what it reads from
   `Shared` (`.needs::<T>()`). `plant()` refuses to start when a declared
   need is missing (`Error::Misconfigured`). The server's `inventory`
   subcommand lists the needs.
4. **`load()` checks every plugin.** It refuses a `Prepare` for an unknown
   DataCapability, and a `Judge` or `Project` for an unknown aggregate.
5. **Release step.** A tag is cut only when a product must move to a newer
   engine, never per phase (the owner's call, 2026-10-20). It is cut on a
   commit whose CI is green and pushed on the owner's word, and the skeleton
   names it in the same change. A tag is never moved.
6. **A5 is three PRs** (A5a, A5b, A5c below), each mergeable alone.

## Phases

| Phase | What | Depends on | Tag | Size |
|---|---|---|---|---|
| A1 | Pin the engine by tag — **done** 08991cc, tag pushed 2026-10-20 | — | dataguard-v1 | small |
| A2 | The engine honours its contracts — **done** fc2db57 | A1 | — | medium |
| A3 | Plugins in the engine, in memory — **done** 06511d6 | A2 | — | large |
| A4 | Plugins over PostgreSQL — **done** cb32b38 | A3 | — | medium |
| A5a | Registry crate, `register.sh`, server skeleton as a lib, `crates/server` — **done** 1be415c | A4 | dataguard-v2 | large |
| A5b | Gates, `tree.rs`, `closure.rs`, isolation — **done** 6b0213c | A5a | — | medium |
| A5c | Rule cards, docs, CLAUDE.md, catch-up regeneration — **done** f13f38b | A5b | — | medium |
| A6 | What the first real product needs (B1, B2, D1–D3, D5, D6) — **done** 02dc509 | A5c | dataguard-v3 | medium |
| A7 | Three gaps the M1 migration found — **done** 6c2f72e | A6 | dataguard-v4 | small |
| B3 | A dotted payload path (`entity.id`) declares its top key; found by M2-entity | A7 | dataguard-v5 | small |
| M0–M5 | ForgeDesMondes migration (M0 map done 2026-10-21: 11 PRs) | A6, A7 | — | ~11 PRs |

A2, A3 and A4 all edit `engine.rs`. They run one after the other, each with
its own mutants budget.

### A1 — Pin

- Tag harness `main` `dataguard-v1`. Document the release step (adjustment 5)
  in `crates/dataguard/ARCHITECTURE.md` and `CLAUDE.md`.
- `assets/dataguard-server/Cargo.toml`: `tag = "dataguard-v1"` instead of
  `branch = "main"`. `tests/dataguard_server.rs` matches the `git =` line
  whatever follows it.

### A2 — The engine honours its contracts

- `aggregate.rs`: drop `on_update`. `Field.merge` becomes
  `enum Merge { Set, LastWriter }`.
- `guard.rs`: `projected` unions arrays for `set`. Add
  `derived_touched(aggregate, entry)`.
- `queue.rs`: `submit` takes the never-stale field set. Add the pure
  `projected_row(partition, row)`.
- `engine.rs`: `Forbidden` on a derived field, a warning for an unknown
  payload key. `load()` validates `manifest.invariants` and returns
  `Error::Misconfigured` (new variant).
- Contract `i-aggregate.schema.json`: `onUpdate` marked deprecated.
  `assets/DATA-INTEGRITY.md` updated.
- Tests: one per rule, and a proptest for `projected_row`.

### A3 — Plugins in the engine, in memory

- New `plugins.rs`: `Shared`, `Context`, `Prepared`, `Refusal`, the traits
  `Prepare`, `Judge` and `Project`, `Answer`, `Plugin` (with `.needs::<T>()`,
  adjustment 3) and `Plugins`.
- `store.rs` gets `Ids`, and `memory.rs` gets `Counter`.
- `engine.rs`: `with_plugins`, `with_ids`, the `Judges` composite keyed by
  aggregate, `Prepare` in `submit` (after authorisation and idempotency
  replay), `observe` in `applied`, and `Error::Refused`. `with_invariants` and
  `NoInvariants` go. `load()` checks every plugin (adjustment 4).
- New `reads.rs`: in memory, an answer is `[]`. In `api.rs`,
  `ReadRequest.sha256` becomes optional.
- Tests:
  - a `Prepare` that rewrites the payload and target, and one that refuses;
  - two judges, each blind to the other's aggregate;
  - `observe` called once per applied write;
  - an `Answer` over memory;
  - `load()` refusing each unknown kind;
  - a missing `Shared` need refused.

### A4 — Plugins over PostgreSQL

- `postgres.rs`:
  - `PgStore::with_projections`: the matching projections run inside the
    apply transaction, and `Refused` takes the engine's `rejected()` path;
  - `ReadOnly` runs a Capability's SQL in a `READ ONLY` transaction, rows
    returned as JSON;
  - `RandomIds` behind a new optional feature `uuid`.
- `engine.rs`: `applied` turns a store `Refused` into a rejection.
- `#[ignore]` tests under `DATAGUARD_PG_URL`, against the Docker Postgres
  (`scripts/postgres.sh up`).

### A5a — Registry, script, skeleton

- `assets/dataguard-server/`:
  - `Cargo.toml`: the engine by tag, with features `postgres` and `uuid`, plus
    `dataguard-registry` by path. No product line.
  - `src/lib.rs`: the generic handlers — `plant(url, Shared)`, the router,
    the applier and `migrate`. Its `submit` maps `Refused` to 404 or 422.
  - `src/main.rs` and `build.rs` are deleted.
- New `assets/dataguard-registry/build.rs` and `src/lib.rs`:
  - `build.rs` walks the unit folders and writes the glue into `OUT_DIR`;
  - it panics, telling you to run `scripts/register.sh`, when a folder and
    the registry's `Cargo.toml` disagree;
  - `lib.rs` exposes `plugins()`, `AGGREGATES` and `DATA_CAPABILITIES`;
  - hostless units are skipped from the host.
- New `assets/register.sh`, installed as `scripts/register.sh`, with
  `--check`. It writes the registry's `Cargo.toml`, which is not an asset;
  `init-repo` runs the script after installing the assets.
- New `assets/server/{Cargo.toml, src/main.rs}`, installed once at
  `crates/server/` and expected to diverge:
  - an empty `Shared` and `plant`;
  - `/health`;
  - the `migrate` and `serve` subcommands;
  - an `inventory` subcommand that lists the units and their needs.
- `assets.rs` registers the new assets and drops the removed ones.
  `CLAUDE_VERSION` is bumped.

### A5b — Gates and the tree

- `gates.yml`:
  - `data-capability-registered` becomes `units-registered`
    (`register.sh --check`, plus "lib name = folder" for DataCapabilities and
    aggregates);
  - `workspace-members-by-glob` adds `crates/*/aggregates/*`;
  - new gate `aggregate-placed`;
  - `ownership` adds "no touched field is derived".
- `isolation.sh`:
  - a new kind, `aggregate`;
  - a DataCapability may depend on its target aggregate's crate;
  - only `crates/dataguard/server` and `crates/server` may depend on the
    registry.
- `tree.rs`:
  - aggregates are discovered under `crates/*/aggregates/*`;
  - a violation names the folder the aggregate belongs in;
  - it mirrors the derived-field check, the aggregate's lib name, and
    "every unit is named in the registry".
- `closure.rs`: a DataCapability, Capability or Aggregate task gets
  `crates/dataguard/registry` ("regenerated, never edited"). An aggregate task
  gets its own crate folder.

### A5c — Rule cards, docs, catch-up regeneration

- `rules.rs`:
  - DATA_CAPABILITY card: "adding one edits no shared file: run
    `scripts/register.sh`", `plugin()`, `Prepare`;
  - AGGREGATE card: a crate holding `Judge`, `Project` and `plugin()`;
  - CAPABILITY card: the four items.
  - The parity tests force the renamed and new gates into the cards.
- Catch-up regeneration (adjustment 1) in `common/branching.rs` and the
  `pr_fix` conflict path, with a fake-git test: a conflict limited to
  `Cargo.lock` and the registry's `Cargo.toml` is regenerated, and any other
  conflict is not.
- Docs:
  - `assets/ARCHITECTURE.md`, `assets/DATA-INTEGRITY.md`,
    `assets/CLAUDE-rules.md`;
  - the crates' `ARCHITECTURE.md`, and `ARCHITECTURE_OVERVIEW.md`;
  - `docs/toBeCheck/dataguard-plugins.md`.

### A6 — What the first real product needs (from M0)

M0, the ForgeDesMondes migration map, found two bugs and seven gaps between
the plugin model and a real product. These are the decisions taken on
2026-10-21, under the owner's delegation (log: harness-dnd2-run/DECISIONS.md).
The harness takes a gap when every product will meet it. A gap that is one
product's own stays in that product. Tag `dataguard-v3` once CI is green.

| # | Gap | Decision |
|---|---|---|
| B1 | The undeclared-key warning is computed on the prepared payload. | Compute it on the submitted payload, before `Prepare`. |
| B2 | The skeleton's `submit` drops the wire form `"target": {"id": …}`. | Accept both `target_id` and `target.id`, as the mock and the rules suite send them. |
| D1 | A `Prepare` reads only its target row. | `Context` gains `row(partition)`, the engine's projected row of any partition, through an async lookup the engine provides. It also gains `rows: Option<&dyn reads::Rows>`, read-only SQL, `None` over memory. |
| D2 | One submission makes one command. | `Prepared.then: Vec<Submission>`: follow-up submissions the engine submits in order once the first is queued, each with the same caller. This is non-atomic, as products do today. Each follow-up's result is returned in `Submitted.then`. |
| D3 | An answer is computed with the command but read only once it is applied. | `Prepared.attachment: Option<Value>`. It is kept with the entry, in memory and in the Postgres journal, and returned in `CommandResult.attachment` once the command is applied. |
| D4 | A value is settled against the real row at apply time. | Product only: the aggregate's `ProjectPg` settles it inside the transaction, and `Project::observe` does it over memory. No harness change. |
| D5 | Rows the tables hold but the engine never wrote, such as fixtures loaded by SQL. | `ProjectPg::hydrate(partition, client) -> Result<Option<row>>`, defaulting to `None`. `PgStore::row` consults it when the engine has no row for the partition. |
| D6 | Products stamp milliseconds. | `Clock::now_millis()`, defaulting to `now() * 1000`; `FixedClock` and the system clock implement it. `Context.now_millis`. |
| D7 | A Capability with no SQL, reading a port. | Product only: a route of `crates/server`. A Capability never depends on the engine, so `Answer` stays a `fn` over rows. |
| D8 | Ids renamed on reads (fixture slug and uuid). | Product only: middleware in `crates/server`. |
| D9 | The system of an aggregate no DataCapability targets. | The product's plan graph decides. The M0 map lists them. |

### A7 — Three gaps the M1 migration found

M1 moved ForgeDesMondes onto `dataguard-v3` and met three gaps every product
meets. No API breaks. Tag `dataguard-v4` once CI is green.

| # | Gap | Decision |
|---|---|---|
| 1 | An aggregate's `ProjectPg` needs `tokio-postgres`, which `externals-behind-ports` refuses outside `crates/data` and `crates/dataguard`. | The engine re-exports it (`harness_dataguard::tokio_postgres`, feature `postgres`): one version, one `Transaction` type, no driver in the unit. The gate reads `Cargo.toml` dependency names, so the re-exported path is not flagged; it is unchanged. |
| 2 | `cargo machete` flags every unit dependency of the registry: the glue that uses them is in `OUT_DIR`, which it does not read. | `register.sh` writes `[package.metadata.cargo-machete] ignored`, every unit package, sorted; `--check` covers it, and `tree.rs` mirrors it. |
| 3 | A hostless Capability has no `host` in contract B, so its manifest fails `manifests-conform-to-contracts`. | Contract B gains the optional `"host": "server" \| "hostless"`, as contract F spells it and the walkers read it. |

### A5 verification (on the throwaway product, `--ignored`)

1. Add one aggregate crate, one DataCapability crate and one Capability crate.
   Run `register.sh`, then `cargo check --all-targets`.
2. The glue names all three. `server inventory` lists them over
   `DATABASE_URL=memory`.
3. Two branches each add one DataCapability. Their merge needs only the
   regeneration (adjustment 2).
4. A second install reports every server asset kept as installed, and
   `crates/server/src/main.rs` as diverged once it is edited.

## ForgeDesMondes migration (after A5c)

The size as of 2026-10-20: 8 DataCapabilities, 12 Capabilities, 12
aggregates under `crates/dataguard/aggregates/`, and a server whose largest
files are `projection.rs` (1525 lines) and `lib.rs` (1408 lines). The
product has 117 test files. Each step is a PR into `main_agent`, driven by
sessions outside the watch, and is green on the product's CI before the next
one starts.

- **M0, freeze and inventory.** Map every `submit()` branch, judge,
  projection and read table to its target unit, in a checked-in file
  `docs/migration-dataguard.md`.
- **M1, strangler root.**
  - Bump the engine to `dataguard-v2`, the tag A5a cuts.
  - Install the A5 assets.
  - `crates/server` composes the registry's plugins with the legacy server,
    which handles any unit not migrated yet.
  - Wrap `Llm` so it can be used as `dyn` in `Shared`.
- **M2, one PR per aggregate (12, in parallel by system).** Each moves the
  aggregate into `crates/<system>/aggregates/<A>/` with its judge and
  projection, and the DataCapabilities that target it move to `Prepare`. Each
  deletes its own branch from the legacy server and reruns `register.sh`.
- **M3, Capabilities (12).** They move to the four items, and the
  registry's `reads` replaces the legacy `reads.rs`.
- **M4, delete the legacy server.** `crates/dataguard/server` becomes
  identical to the asset, and `.harness/installed.json` records its
  fingerprint.
- **M5, restart the factory on harness `main`.**
  - Every gate is green on `main_agent`.
  - The first new write-side task proves the goal: its PR touches only its
    unit's folder, `Cargo.lock` and the registry's `Cargo.toml`.

### The building milestones are frozen, then rebuilt by the factory

The owner's decision, 2026-10-21: the migration covers `main_agent` only.
Five milestones were building on legacy-style branches: #20, #21, #25, #26
and #28. Together they hold 32 DataCapabilities and about 29k lines of
legacy server code. They are frozen, and the factory rebuilds them on the
new model. That is the factory's real test of the cut: units by folder,
parallel lanes, no hub conflict, and generated files regenerated.

- **Done (2026-10-21).** Each branch's head is kept as
  `archive/milestone/<n>-…`, with its sha listed in the run folder.
- **At M5, before the watch starts**, for each frozen milestone:
  1. Reset `milestone/<n>-…` to the migrated `main_agent`.
  2. Reopen every task: the delivered ones and the open ones.
     - Clear the delivery labels, and keep `harness:agent` and
       `harness:ready`.
     - Add a `## Reference` section to the body: the archive branch and
       sha, and the task's merged PR with its files. The session ports
       that code into its unit's folder instead of inventing it again.
  3. Delete the tasks' old `feat/…` branches, whose code is in the archive.
  4. No re-split. The split is already one task per unit, chained by what
     each produces and needs, which is the new model's grain. Each reopened
     spec is rewritten instead:
     - drop the `## Decision` sections that steer repairs in the legacy
       server;
     - turn "Registered with the DataGuard and crates/server" into "run
       `bash scripts/register.sh`";
     - do not reopen a task whose unit already exists on `main_agent`, such
       as #21's Sound and SoundPlayback aggregates, moved by M2-0.
- **Then** the watch rebuilds the five milestones, and goes on with #22,
  #24, #27, #29 and #30 as split on the new model.

## Risks

- **Projection drift during M2.** It is the most delicate code. The Postgres
  tests on the real tables check that behaviour is unchanged.
- **Feature unification.** An aggregate crate implementing `ProjectPg` turns
  on the engine's `postgres` feature itself.
- **A moved tag breaks every product.** Tags are never moved.
- **Mutants budget on `engine.rs`.** A2, A3 and A4 stay separate PRs.
- **Narrowing.** It is off today. If it comes back, the registry's path
  dependencies need the whole `crates/` tree, as the server does today. This
  is recorded in toBeCheck.

## Verification, every phase

    PATH=$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH
    cargo check --workspace --all-targets
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo fmt --all -- --check
    cargo test --workspace --all-features
    cargo machete && bash scripts/dead-code.sh
    git diff origin/main > change.diff && cargo mutants --workspace --in-diff change.diff

A4 also runs
`DATAGUARD_PG_URL=… cargo test -p harness-dataguard --features postgres -- --ignored`.
A5a and A5c also run
`cargo test -p harness-architecture --test dataguard_server -- --ignored`.
