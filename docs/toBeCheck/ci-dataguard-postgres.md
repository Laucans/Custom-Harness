# CI: DataGuard's PgStore tests in the `postgres` job

## Context
The 7 `#[ignore]`d PgStore tests in `crates/dataguard/src/postgres.rs`
never ran in CI. The `postgres` job (a `postgres:16` service, a
`NOSUPERUSER` role `app` owning database `app`) now runs them. Main is not
pushed yet, so the step has never run on GitHub.

## Shipped
In the commit that adds this file:
- `.github/workflows/ci.yml`, job `postgres`: step
  `cargo test -p harness-dataguard --all-features --lib -- --ignored postgres::tests`
  with `DATAGUARD_PG_URL=postgres://app:app@localhost:5432/app`.
- `Scratch::new` panics without `DATAGUARD_PG_URL` (no silent pass on
  zero assertions).

Run locally once, 2026-10-25, against `scripts/postgres.sh`'s server, role
`dnd_app` (`NOSUPERUSER`, `CREATEDB`) in a throwaway database it owned:
7 passed, in parallel, 0.44 s.

## To verify
- [ ] On the first push, the `postgres` job runs the dataguard step.
- [ ] Its output says `7 passed` — not 0, not all filtered out.
- [ ] Role `app` can `CREATE SCHEMA` in database `app` (no permission error).
- [ ] The tests run in parallel without schema collisions
      (`dataguard_test_<name>_<pid>`).
