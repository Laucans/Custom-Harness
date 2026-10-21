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
product's crates/dataguard/server (bin, axum)  ──git──▶  harness-dataguard (lib)
          ▲                                                       │
          │ installed by `harness init-repo`                      │ depends on no crate of this workspace
  crates/<system>/data-capabilities/*  (manifests, dependency lines the gate requires)
```

It sits beside `harness-view-render`, outside the harness's own graph: none of
the eight other crates names it, and it names none of them. The harness
installs its **server skeleton** (`crates/architecture/assets/dataguard-server/`)
into every product; the product's `Cargo.toml` takes the engine by `git`. An
agent's closure never contains the engine: a DataCapability sees its
manifest, its aggregate's `aggregate.json`, and the server's dependency line.

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
| `command` | G, H | an entry of the queue, its projection, its states; the command result (five fields) |
| `queue` | G | **pure**: one FIFO per partition, fixed positions, projected view and `projected_row` (the row as the pending commands leave it), `ValueDeclaredAhead` (never for a merging field), confirmations, parked (24 h), holds (1 min), expiry |
| `guard` | F, H, I | **pure**: authorisation, static then dynamic classification of the effect, the invariants on a projected and on the real state — a rule written `<field> <op> <value>` in `aggregate.json` is judged here (as the mock judges it), any other is the product's code behind `Invariants` (the engine's is its aggregates' `Judge`s), and the two add up |
| `resolver` | J | **pure**: the impact plan — readers, holds, cascade by relation policy, the decision `auto / confirm / human_review / blocked / rejected` |
| `reconcile` | K | **pure**: a review, the AI's proposal behind a trait, a human's decision by role |
| `messages` | L | the twelve message kinds and the notifier port |
| `store` | — | the store port: pending entries, holds, rows, collisions, incoming relations, `apply` in one transaction with the journal; the `Clock` and the `Ids` a fresh row id comes from |
| `memory` | — | the store, notifier, clock and ids (`Counter`) in memory: what every test runs on |
| `plugins` | F, I, C | what a unit plugs in: `Prepare` (a DataCapability), `Judge` and `Project` (an aggregate), `Answer` (a Capability), `Shared` (what units read and do not own), `Plugin` and `Plugins` |
| `reads` | C | the Capabilities' `Answer`s, run: the query's rows over a read-only `Rows` connection — `[]` in memory — handed to the answer |
| `postgres` (feature) | — | the store on PostgreSQL via `tokio-postgres`: its own tables (`migrations/0001_dataguard.sql`), `NOTIFY dataguard` on every application |
| `api` | H | the wire shapes the mock server already speaks: `Submission`, `Submitted`, `ReadRequest`, `HoldRequest`, `ReviewDecision` |
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
  sees a `Context` — the manifest, the caller, the target row as the
  pending commands leave it (`Queue::projected_row`, empty when no row is
  named), now, the `Ids`, `Shared` — and returns the payload and target
  the command writes, with warnings for the author, or a `Refusal`
  (`NotFound`, `Unprocessable`), which is `Error::Refused` and queues
  nothing. The rules that follow (derived fields, undeclared keys,
  invariants) judge what the `Prepare` returned.
- **`Judge`**, keyed by aggregate: the engine's `Invariants` is the
  composite of the judges, and a judge is only asked about its own
  aggregate.
- **`Project`**, keyed by aggregate: `observe` sees each write the store
  applied, once, after `apply` succeeded; a rejected command is never seen.
- **`Answer`**, keyed by Capability: its SQL, the variables it binds in
  order, and the function from rows to answer. `Reads::run` hands it `[]`
  when there is no connection (A4 brings PostgreSQL).
- **`Shared`**, keyed by type: what the server (`with_shared`) and the
  plugins (`.share`) give. A plugin declares what it reads (`.needs::<T>()`).

`load()` refuses (`Error::Misconfigured`) a `Prepare` for a DataCapability
the engine does not register, two for the same one, a `Judge` or `Project`
for an aggregate it does not register, and a need nothing shares. The
check sits in `load()`, not in the builders: a need can be filled by the
server or by any plugin, which is known only once everything is given, and
`load()` already refuses units that do not fit before the server serves.

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
4. An application is one store transaction: the row, the next `dataVersion`,
   the journal; then the author, the infrastructure and whoever saw the
   command ahead are told.

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
(`assets/dataguard-server/Cargo.toml`), never at `main`: a branch would move
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

## Testing

Every rule has a unit test in its module; `queue` has a property test (fixed,
strictly increasing positions whatever happens). The engine's tests run the
contract's scenario end to end on the in-memory store. The PostgreSQL store's
test is `#[ignore]`d and needs `DATAGUARD_PG_URL`; `scripts/postgres.sh`
serves one.
