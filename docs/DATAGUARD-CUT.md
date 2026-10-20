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
   Both are regenerated. The harness's catch-up step (`branching::catch_up`)
   and the `pr_fix` conflict path rerun `scripts/register.sh`, then
   `cargo metadata`, on a conflict limited to these two files, and commit the
   result. A conflict anywhere else stays a real conflict.
2. **The proof is parallel.** A5's verification creates two branches, each
   adding one DataCapability, and merges them into one base. The merge must
   need nothing but the regeneration of point 1.
3. **No silent `Shared` hole.** A `Plugin` declares what it reads from
   `Shared` (`.needs::<T>()`). `plant()` refuses to start when a declared
   need is missing (`Error::Misconfigured`). The server's `inventory`
   subcommand lists the needs.
4. **`load()` checks every plugin.** It refuses a `Prepare` for an unknown
   DataCapability, and a `Judge` or `Project` for an unknown aggregate.
5. **Release step.** A `dataguard-vN` tag is cut on harness `main` only after
   its CI is green, and pushed only on the owner's word. A tag is never moved.
6. **A5 is three PRs** (A5a, A5b, A5c below), each mergeable alone.

## Phases

| Phase | What | Depends on | Tag | Size |
|---|---|---|---|---|
| A1 | Pin the engine by tag — **done** 08991cc, tag pushed 2026-10-20 | — | dataguard-v1 | small |
| A2 | The engine honours its contracts | A1 | dataguard-v2 | medium |
| A3 | Plugins in the engine, in memory | A2 | dataguard-v3 | large |
| A4 | Plugins over PostgreSQL | A3 | dataguard-v4 | medium |
| A5a | Registry crate, `register.sh`, server skeleton as a lib, `crates/server` | A4 | — | large |
| A5b | Gates, `tree.rs`, `closure.rs`, isolation | A5a | — | medium |
| A5c | Rule cards, docs, CLAUDE.md, catch-up regeneration | A5b | — | medium |
| M0–M5 | ForgeDesMondes migration | A5c | — | ~15 tasks |

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
  - Bump the engine to `dataguard-v4`.
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
