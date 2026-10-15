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
[`messages::Notifier`], [`store::Clock`], [`guard::Invariants`],
[`reconcile::Proposer`] — because the harness's ports are about driving
sessions, not about a product's data.

## The modules, each a contract

| module | contract | what it holds |
| --- | --- | --- |
| `manifest` | F | a `DataCapability` manifest: effect, target, `touches`, mode, permissions, `callableBy`, idempotency |
| `aggregate` | I | an `aggregate.json`: fields and their criticality, invariants and what a violation does, relations and `onDelete`, holds |
| `registry` | C | the persisted queries served, by Capability and hash; the readers of a field |
| `command` | G, H | an entry of the queue, its projection, its states; the command result (five fields) |
| `queue` | G | **pure**: one FIFO per partition, fixed positions, projected view, `ValueDeclaredAhead`, confirmations, parked (24 h), holds (1 min), expiry |
| `guard` | F, H, I | **pure**: authorisation, static then dynamic classification of the effect, the invariants on a projected and on the real state — a rule written `<field> <op> <value>` in `aggregate.json` is judged here (as the mock judges it), any other is the product's code behind `Invariants`, and the two add up |
| `resolver` | J | **pure**: the impact plan — readers, holds, cascade by relation policy, the decision `auto / confirm / human_review / blocked / rejected` |
| `reconcile` | K | **pure**: a review, the AI's proposal behind a trait, a human's decision by role |
| `messages` | L | the twelve message kinds and the notifier port |
| `store` | — | the store port: pending entries, holds, rows, collisions, incoming relations, `apply` in one transaction with the journal |
| `memory` | — | the store, notifier and clock in memory: what every test runs on |
| `postgres` (feature) | — | the store on PostgreSQL via `tokio-postgres`: its own tables (`migrations/0001_dataguard.sql`), `NOTIFY dataguard` on every application |
| `api` | H | the wire shapes the mock server already speaks: `Submission`, `Submitted`, `ReadRequest`, `HoldRequest`, `ReviewDecision` |
| `engine` | all | the four together over the ports: `submit`, `confirm`, `cancel`, `requeue`, `hold`/`renew`/`release`, `decide`, `status`, `run_once` |

The pure modules decide; the engine sequences them and talks to the ports.
A new policy is a change in a pure module and its tests, never in the
engine; a new store is an implementation of `Store`, never a branch in it.

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

## Testing

Every rule has a unit test in its module; `queue` has a property test (fixed,
strictly increasing positions whatever happens). The engine's tests run the
contract's scenario end to end on the in-memory store. The PostgreSQL store's
test is `#[ignore]`d and needs `DATAGUARD_PG_URL`; `scripts/postgres.sh`
serves one.
