# The DataGuard engine on main

## Context

`harness-dataguard` was written on the branch
`claude/vertical-architecture-analysis-bxr497` and ported to `main` alone,
without the rest of that branch. It is the products' write side: the
DataQueue, the DataGuard, the Resolver, the messages. A product's
`crates/dataguard/server` (installed by `init-repo`) takes it by `git`, from
`main`. On the branch, the engine passed its tests and its mutants, and a
run against a local PostgreSQL passed (`DATAGUARD_PG_URL`). Nothing has run
it from `main` inside a real product yet.

## What shipped

- The crate `crates/dataguard/` (engine, in-memory store, PostgreSQL store
  behind the `postgres` feature, migration `0001_dataguard.sql`).
- `init-repo` installs `crates/dataguard/server/` (`Cargo.toml`,
  `build.rs`, `src/main.rs`): 56 files in all.
- The gates `queries-valid` and `schema-impact` are switched on (a Postgres
  service in the CI). `semantic-conformance` stays off.
- `mock/src/server.js` behaves as the engine: one queue per partition,
  contract G's states, a 30 s grace at the head before a command is parked,
  the aggregate's invariants, a DataCapability nobody handles rejected.

## To check

- [ ] On a fresh product, `cargo check -p dataguard-server` resolves
      `harness-dataguard` by `git` from `main` and builds.
- [ ] The server starts against the product's PostgreSQL
      (`docker/postgres/compose.yml`), migrates, and lists the registry its
      build script generated (every `aggregate.json` and
      `data-capability.json`).
- [ ] A command through the shell client: `queued` → applied; a stale one
      → `awaiting_confirmation`; unconfirmed for 30 s at the head → `parked`,
      its author told once; requeued → its grace starts again.
- [ ] `queries-valid` on a real PR: a registered query on a column that does
      not exist fails, named; a valid one passes.
- [ ] `schema-impact` on a real PR: a migration dropping a column a
      Capability reads, without its query, fails naming the Capability.
- [ ] The mock-up in the browser: a write on a critical field waits for its
      approver; an invariant breach is rejected with its id.
- [ ] The ignored test that builds the skeleton passes from `main`:
      `cargo test -p harness-architecture --test dataguard_server -- --ignored`.
