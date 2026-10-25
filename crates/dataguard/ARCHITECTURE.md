# `harness-dataguard` — architecture

The write side of the agent-native architecture, once. Every product the
harness builds mutates its data through one frontier — a typed command
(`DataCapability`, contract F) enters the DataQueue (G), the DataGuard judges
it (F, H, I), the Resolver works out what a mutation of existing data reaches
(J), one applier applies the head of each partition and takes the next
`dataVersion`, the authors hear what happened (L), and what nobody can settle
goes to a human with the AI's proposal beside it (K). This crate is that
engine, written once here and never by an agent of a product.

```
product's crates/server (bin) ──▶ crates/dataguard/server (lib, axum) ──git──▶ harness-dataguard (lib)
                                          │                                          │
                                          ▼                                          │ depends on no crate
                                 crates/dataguard/registry (lib, build.rs)            │ of this workspace
                                          │  one `path` line per unit, written by scripts/register.sh
                                          ▼
  crates/<system>/{aggregates,data-capabilities,capabilities}/*  (one crate per unit)
```

It sits beside `harness-view-render`, outside the harness's own graph: none of
the eight other crates names it, and it names none of them. The harness
installs the product's **write side** into every product (see *The product's
server, as installed* below); the engine is taken by `git`, at a tag written
once, in `crates/dataguard/server/Cargo.toml`. An agent's closure never
contains the engine: a unit sees its own folder, its manifest, and the
registry it is registered in by a script.

## Why `Send`, and why no port of core

The harness drives one session at a time and chose `#[async_trait(?Send)]`
throughout (`docs/MIGRATION.md`, decision 3). This crate runs in a product's
multi-threaded server, so its traits are `Send + Sync` and its async methods
hold no lock across an `.await` (`engine.rs` says so, and clippy's
`await_holding_lock` checks it). It declares its own ports — [`store::Store`],
[`messages::Notifier`], [`store::Clock`], [`store::Ids`],
[`reconcile::Proposer`], [`reads::Rows`], and the units' traits in
[`plugins`] — because the harness's ports are about driving sessions, not
about a product's data.

## The modules, each a contract

| module | contract | what it holds |
| --- | --- | --- |
| `manifest` | F | a `DataCapability` manifest: effect, target, `touches`, mode, permissions, `callableBy`, idempotency |
| `aggregate` | I | an `aggregate.json`: fields, their kind (a derived one is never written) and criticality, how they merge (`set`, `last-writer`), invariants and what a violation does, relations and `onDelete` (`onUpdate` is deprecated, ignored), holds |
| `registry` | C | the persisted queries served, by Capability and hash; the readers of a field |
| `command` | G, H | an entry of the queue, its projection, its states, the attachment its `Prepare` computed; the command result (five fields, and the attachment once applied) |
| `queue` | G | **pure**: one FIFO per partition, fixed positions, projected view and `projected_row` (the row as the pending commands leave it), `ValueDeclaredAhead` (never for a merging field), confirmations, parked (24 h), holds (1 min), expiry |
| `guard` | F, H, I | **pure**: authorisation, static then dynamic classification of the effect, the invariants on a projected and on the real state — a rule written `<field> <op> <value>` in `aggregate.json` is judged here (as the mock judges it), any other is the product's code behind `Invariants` (the engine's is its aggregates' `Judge`s), and the two add up |
| `resolver` | J | **pure**: the impact plan — readers, holds, cascade by relation policy, the decision `auto / confirm / human_review / blocked / rejected` |
| `reconcile` | K | **pure**: a review, the AI's proposal behind a trait, a human's decision by role |
| `messages` | L | the twelve message kinds and the notifier port |
| `store` | — | the store port: pending entries, holds, rows, collisions, incoming relations, `apply` in one transaction with the journal; the `Clock` (seconds, and `now_millis`, by default `now() * 1000`) and the `Ids` a fresh row id comes from (`RandomIds`, uuid v4, behind the `uuid` feature) |
| `memory` | — | the store, notifier, clock (`FixedClock`, set to the second or the millisecond) and ids (`Counter`) in memory: what every test runs on |
| `plugins` | F, I, C | what a unit plugs in: `Prepare` (a DataCapability) and the `Context` it reads (`Partitions`, the rows the engine lends it), `Judge`, `Project` and, over PostgreSQL, `ProjectPg` (an aggregate, `hydrate` included), `Answer` (a Capability), `Shared` (what units read and do not own), `Plugin` and `Plugins` (whose `needs()` names what they read from `Shared`, for a server's inventory) |
| `reads` | C | the Capabilities' `Answer`s, run: the query's rows over a read-only `Rows` connection — `[]` in memory — handed to the answer |
| `postgres` (feature) | — | the store on PostgreSQL via `tokio-postgres`, which the crate root re-exports (`harness_dataguard::tokio_postgres`: a unit's `ProjectPg` names its `Transaction` and `Client` there, never a driver of its own): its own tables (`migrations/0001_dataguard.sql`), the aggregates' `ProjectPg`s inside the apply transaction, `NOTIFY dataguard` on every application; `ReadOnly`, the `Rows` of the reads, in `READ ONLY` transactions; `values`, **pure**: a JSON variable typed for its parameter, a row as a JSON object |
| `api` | H | the wire shapes the mock server already speaks: `Submission` (its row named by `targetId` or `"target": {"id": …}`), `Submitted` (with `then`, a `FollowUp` per follow-up), `ReadRequest`, `HoldRequest`, `ReviewDecision` |
| `engine` | all | the four together over the ports and the plugins: `submit`, `confirm`, `cancel`, `requeue`, `hold`/`renew`/`release`, `decide`, `status`, `run_once` |

The pure modules decide; the engine sequences them and talks to the ports.
A new policy is a change in a pure module and its tests, never in the
engine; a new store is an implementation of `Store`, never a branch in it.

## Plugins: a unit's code, in the engine

Each unit of a product brings a `Plugin`; the server gathers them into
`Plugins` (`FromIterator`) and hands them to `Engine::with_plugins`, and
builds `Reads::from(&plugins)` for the reads.

- **`Prepare`**, keyed by `DataCapability`: runs in `submit`, after the
  authorisation and the idempotency replay, before the entry is built. It
  sees a `Context`:
  - the manifest, the caller, now (`now`, seconds; `now_millis`), the
    `Ids`, `Shared`;
  - `base`, the target row as the pending commands leave it
    (`Queue::projected_row`, empty when no row is named);
  - `row(partition)`, any partition's row the same way, through
    `Partitions`, which the engine implements over its store and queue.
    The engine borrows itself into it (`Lent`): no queue lock is held
    across an await (the lock is taken after the store answers and
    released in the same expression), so `Context` stays `Sync` and the
    `Prepare`'s future `Send`. A store failure under it is kept, and the
    engine answers `Error::Store` whatever the `Prepare` made of it;
  - `rows`, read-only SQL (`reads::Rows`), `None` unless the server gave
    one (`Engine::with_rows`; the skeleton does over PostgreSQL).

  It returns `Prepared`: the payload and target the command writes,
  warnings for the author, follow-ups (`then`) and an `attachment`; or a
  `Refusal` (`NotFound`, `Unprocessable`), which is `Error::Refused` and
  queues nothing. The undeclared-key warning judges the payload the author
  submitted, before the `Prepare` runs (its keys are the wire's, and a
  `Prepare` may rename them into fields); derived fields and invariants
  judge what the `Prepare` returned.
- **Follow-ups (`Prepared::then`)**: submissions the engine makes once the
  first command is queued, in order, each with the first's caller (the
  caller a follow-up names is replaced) and through the whole `submit`
  path — authorised, replayed on its own idempotency key, prepared. They
  are not atomic with the first: the first refused one stops the rest and
  undoes nothing. Each outcome is in `Submitted::then`, `{"submitted": …}`
  or `{"refused": {"dataCapability", "error"}}`. One level only: a
  follow-up whose `Prepare` asks for follow-ups is refused
  (`Error::Misconfigured`), so a chain always ends and the answer stays
  flat; a replay of the first submits none.
- **Attachment (`Prepared::attachment`)**: kept with the entry (`Entry`,
  serialised with it, so in the PostgreSQL queue's `entry` JSON) and copied
  into the result when the head is applied; the journal keeps that result,
  so `status` answers it once applied, never before, and never for a
  rejected command.
- **`Judge`**, keyed by aggregate: the engine's `Invariants` is the
  composite of the judges, and a judge is only asked about its own
  aggregate.
- **`Project`**, keyed by aggregate: `observe` sees each write the store
  applied, once, after `apply` succeeded; a rejected command is never seen.
- **`ProjectPg`** (feature `postgres`), keyed by aggregate: the projection
  into the product's own tables, run by `PgStore` (see below), not by the
  engine. It is its own trait, not a `Project` with more: what it writes
  commits with the write, so it has nothing left to observe after. Its
  `hydrate(partition, client)` (`Ok(None)` by default) answers a row its
  tables hold but the engine never wrote. The unit takes the PostgreSQL
  types from the engine, `harness_dataguard::tokio_postgres` (the crate
  root re-exports the client the store runs on): one version, one
  `Transaction` type, and no `tokio-postgres` in the unit's `Cargo.toml`
  for the gate `externals-behind-ports`, which allows a database client in
  `crates/data` and `crates/dataguard` only, to refuse.
- **`Answer`**, keyed by Capability: its SQL, the variables it binds in
  order, and the function from rows to answer. `Reads::run` hands it `[]`
  when there is no connection, and over PostgreSQL the rows `ReadOnly`
  returns (see below).
- **`Shared`**, keyed by type: what the server (`with_shared`) and the
  plugins (`.share`) give. A plugin declares what it reads (`.needs::<T>()`).

`load()` refuses (`Error::Misconfigured`) a `Prepare` for a DataCapability
the engine does not register, two for the same one, a `Judge`, `Project` or
`ProjectPg` for an aggregate it does not register, and a need nothing shares. The
check sits in `load()`, not in the builders: a need can be filled by the
server or by any plugin, which is known only once everything is given, and
`load()` already refuses units that do not fit before the server serves.

## Over PostgreSQL: projections in the transaction, reads that only read

The server gives the same `Plugins` to the store first, then to the engine:

```rust
let store = PgStore::open(client).await?.with_projections(&plugins).await?;
let read_only: Arc<dyn Rows> = Arc::new(ReadOnly::new(read_client));
let reads = Reads::from(&plugins).with_connection(read_only.clone());
let engine = Engine::new(Arc::new(store), notifier, clock)
    .with_rows(read_only)
    .with_plugins(plugins);
```

- **`PgStore::with_projections(&Plugins)`**: the store takes the
  `ProjectPg`s from the plugins rather than from a list of its own, so a
  unit registers once, in its `plugin()`, and the engine's `load()` checks
  the same aggregates the store runs. Each one is `warm`ed there, once, when
  the store starts (its tables, a cache); one that does not warm stops the
  start (`Error::Store`).
- **`apply`** runs, in one transaction: the next `dataVersion`, the engine's
  row, then every `ProjectPg` of the write's aggregate (`Write::aggregate`),
  then the journal and the `NOTIFY`, then the commit. A projection's
  `ProjectionError::Refused` rolls the transaction back and the store
  answers `Error::Refused` (an `Unprocessable` refusal, the projection's
  message): the product's code refused the command, as a `Prepare` refuses
  one, and the server already maps that variant. No store-level variant was
  added, because the engine is the only one to see it: `applied` takes the
  `rejected()` path on it, so the entry is Rejected (no violation listed,
  none was), its author hears `CommandRejected` with the projection's reason,
  and the applier's step goes on. `ProjectionError::Failed` is `Error::Store`:
  the step fails, as any store failure does.
- **`row`** answers the engine's row of the partition. When the engine has
  none (it never wrote the partition), it asks the `ProjectPg`s of the
  partition's aggregate to `hydrate` it, the first `Some` wins: fixtures
  loaded by SQL reach the judges and a `Prepare`'s `base` and `row`. An
  archived partition is never hydrated back. A failed hydration is
  `Error::Store`. An insert's collision still reads the engine's rows only.
- **`ReadOnly`** is the reads' `Rows`, on a connection of its own (a slow
  read never waits on the applier): each query runs in a `READ ONLY`
  transaction, so a Capability's SQL that writes is refused by PostgreSQL.
  The variables are bound in the order `Answer.variables` lists them, each
  typed by the parameter it fills (integers, floats, bool, text, json, uuid;
  another type is cast in the SQL from text, `$1::text::numeric`). A row comes
  back as an object, column name to value: integers, floats (a non-finite one
  as `"NaN"`, `"Infinity"`), bool, text, `json`/`jsonb`, `uuid` hyphenated,
  `timestamptz` in RFC 3339 UTC, `numeric` as a decimal string (no lossless
  JSON number), `NULL` as `null`. Any other type is `Error::Store` naming the
  column.

## The product's server, as installed

`harness init-repo` installs the write side from
`crates/architecture/assets/` (`assets.rs` lists the paths), and runs
`scripts/register.sh` once, through the `Process` port, when the product has
no registry yet:

| installed at | from | what it is |
| --- | --- | --- |
| `crates/dataguard/server/` | `dataguard-server/` | a library, generic: `plant(url, Shared) -> Planted { engine, reads }` (over PostgreSQL one `ReadOnly` serves the reads and the `Prepare`s' `rows`), `router`, `woken_applier(engine, wake)` with `waking(app, wake)` (the applier runs every second and at once after each write the wire answers — a `POST` anywhere but `/capabilities/` notifies it), `applier(engine)` kept for a binary that wakes nothing (the clock alone), `migrate`; it re-exports the engine (`dataguard_server::engine`) and the registry. Its `Cargo.toml` holds the engine's one `tag =` line; its `submit` takes `targetId` or `target.id` and maps a refusal to 404 or 422, its read checks a hash when the request names one, then runs `Reads::run` |
| `crates/dataguard/registry/` | `dataguard-registry/` | `build.rs` and `src/lib.rs`; its `Cargo.toml` is **not** an asset, `scripts/register.sh` writes it |
| `scripts/register.sh` | `register.sh` | writes the registry's `Cargo.toml`: the header, the engine line copied from the server's, one sorted `path` line per unit crate (named after its package); copies the same engine line into the root `Cargo.toml`'s `[workspace.dependencies]`, whence a unit takes `harness-dataguard = { workspace = true }` (one engine, one `Plugin` type); lists the same unit packages, sorted, as `[package.metadata.cargo-machete] ignored` (the registry uses them only in its `OUT_DIR` glue, which `cargo machete` does not read); `--check` exits 1 when either file differs or a unit takes the engine from a source of its own |
| `crates/server/` | `server/` | the product's binary, installed once and expected to diverge: an empty `Shared`, `plant`, `/health`, the woken applier (`waking` over the whole app), the subcommands `migrate`, `serve` and `inventory` (each unit with what it needs, read through `Plugins::needs`) |

The registry's build script walks
`crates/*/{aggregates,data-capabilities,capabilities}/*` from the root, skips
a unit whose manifest says `"host": "hostless"`, and writes
`OUT_DIR/glue.rs`: the consts `AGGREGATES`, `DATA_CAPABILITIES` and
`CAPABILITIES` (each a list of `Unit { folder, manifest, plugin }`) and
`QUERIES` (contract C). The glue calls one conventional item per unit:

- an aggregate's crate (library `aggregate_<snake>`, `AiSettings` as
  `aggregate_ai_settings`) and a DataCapability's (library named after its
  folder, `-` as `_`) expose `pub fn plugin() -> Plugin`;
- a Capability's crate (library named after its folder) exposes
  `CAPABILITY`, `SQL`, `VARIABLES` and `fn answer`, which the glue wraps
  into an `Answer`: a Capability never depends on the engine.

A folder holding only its manifest is registered by its manifest alone (an
aggregate not yet moved into a crate). The build fails, naming the units
and telling to run `scripts/register.sh`, when a unit crate is missing from
the registry's `Cargo.toml` or a line there names no unit crate; it also
fails when two unit crates would be the same library. It watches the whole
`crates/` tree, so a new folder reruns it.

`crates/architecture/tests/dataguard_server.rs` (`--ignored`) builds a
throwaway product from these assets, the engine by `path`: one unit of each
kind, the script, `cargo check` and `clippy`, the glue, `server inventory`
over `DATABASE_URL=memory`, `plant` over memory, then two DataCapabilities
added on two branches, each by its folder and the script alone, and merged
into one base: the merge conflicts on `Cargo.lock` and the registry's
`Cargo.toml` only, and the base's lock, the script and `cargo metadata`
resolve it (the harness's `branching::catch_up_locally` does the same when
it catches a milestone up).

## The applier's step (`Engine::run_once`)

1. Time passes: parked commands past their `ttl` expire, holds past theirs
   are released, their owners warned beforehand.
2. For each partition with something pending, the head is looked at: blocked
   by a hold, waiting for a review, or ready. An unconfirmed head holds its
   partition for a grace of 30 s (`CONFIRM_GRACE_SECONDS`), then is parked
   and set aside; the next one is the head.
3. A ready head is classified **at execution** (`guard::classify` on what the
   store reports: a unique key, a trigger, a dangling reference): a pure
   addition applies when the invariants hold on the real state; a mutation
   goes through `resolver::plan`, whose decision applies it, rejects it,
   blocks it in its place, or opens a review.
4. An application is one store transaction: the row, the aggregate's
   projections over PostgreSQL, the next `dataVersion`, the journal; then
   the author, the infrastructure and whoever saw the command ahead are
   told. A store that refuses the write (`Error::Refused`) has rolled it
   back: the head is rejected, as on a violated invariant.

Only the invariants on the **real** state decide (`ARCHITECTURE.md`,
principle 10): what the enqueue computed on the projected state is a warning
to the author, never a verdict.

## The mock is the same protocol

`crates/architecture/assets/mock/src/server.js` answers the same routes with
the same states and messages, in memory, in the page: a Micro-UI built on the
mock meets no surprise on the real server. `crates/architecture/tests/mock_server.rs`
runs the contract's scenario against both.

## Releases: a tag when a product must move

A product takes the engine at a release tag, `dataguard-vN`
(`assets/dataguard-server/Cargo.toml`, the one line `register.sh` copies
into the registry and the root's `[workspace.dependencies]`),
never at `main`: a branch would move
under every product at its next `cargo update`. A tag is not cut per
change. It is cut when a product has to move to a newer engine, such as a
skeleton that needs a new API, or a fix a product waits for:

1. The engine's changes land on harness `main`, and `main`'s CI is green.
2. The tag `dataguard-vN` is cut on that green commit and pushed on the
   owner's word, and the skeleton's `tag =` line names it in the same
   change. The skeleton never names a tag that does not exist yet.
3. A tag is never moved or deleted: a product's `Cargo.lock` holds the
   commit it resolved, and a moved tag breaks every product that fetches it
   again.
4. A product moves to the new tag in a change of its own (`cargo update` is
   its own PR), never bundled with feature work.

A changed engine API is a contract change for every product: the release
note says what a product must change to move.

**`dataguard-v3`** (A6, `docs/DATAGUARD-CUT.md`), what a product changes:

- `Prepared` gained `then` and `attachment`, `Context` gained
  `now_millis`, `partitions` and `rows`, `Entry` and `CommandResult` gained
  `attachment`, `Submitted` gained `then`: a struct literal of one of them
  takes the new fields, or `..Prepared::default()` for `Prepared`.
- The undeclared-key warning now reads the submitted payload: a test that
  asserted a warning on a key only a `Prepare` wrote loses it.
- `Error` is `Clone`; `Clock` has `now_millis` (a clock of the product's
  own may keep the default); `ProjectPg` has `hydrate` (default `None`).

**`dataguard-v4`** (A7, `docs/DATAGUARD-CUT.md`), what a product changes
to move from `dataguard-v3` (no API break):

- Bump the `tag =` line of `crates/dataguard/server/Cargo.toml` to
  `dataguard-v4`.
- An aggregate crate that declares `tokio-postgres` for its `ProjectPg`
  drops that line and imports `harness_dataguard::tokio_postgres::…`
  instead (the engine line of `[workspace.dependencies]` carries the
  `postgres` feature).
- Rerun `bash scripts/register.sh`: it moves the root's engine line to the
  tag and writes the registry's `cargo machete` ignore list. Commit both
  with `Cargo.lock`.


### `dataguard-v5`

A manifest's dotted payload path (`"entity.id"`) now declares its top key
(`entity`), so a submission sending `{"entity": {…}}` no longer gets a
spurious undeclared-key warning. No API change: a product moves by bumping
the tag and rerunning `scripts/register.sh`.

## Testing

Every rule has a unit test in its module; `queue` has a property test (fixed,
strictly increasing positions whatever happens). The engine's tests run the
contract's scenario end to end on the in-memory store (`MemoryStore::refusing`
stands for a projection that refuses). The PostgreSQL mappings are pure
(`postgres/values.rs`) and tested without a database. The PostgreSQL store's
tests are `#[ignore]`d and need `DATAGUARD_PG_URL` (the CI's `postgres` job
runs them), each in a scratch schema
it drops: the contract's scenario, a projection writing a product table in
the apply transaction, a refused one rolling both back, `ReadOnly` answering
and refusing a write, `RandomIds` accepted as distinct uuids, a row loaded
by plain SQL hydrated into `row()` and a `Prepare`'s `base`, an attachment
kept in the queue across a restart and in the journal.

    bash scripts/postgres.sh up
    DATAGUARD_PG_URL=postgres://postgres:postgres@localhost:5432/postgres \
      cargo test -p harness-dataguard --features postgres,uuid -- --ignored
