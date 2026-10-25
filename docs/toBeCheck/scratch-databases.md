# Scratch databases: the guard, the right to force, the watch's sweep (H6)

## Context
ForgeDesMondes, 2026-10-24: the factory's PostgreSQL held 7 939 databases
(68 GB, about +250 an hour). 73 product test files create a database with
`CREATE DATABASE` and drop it at their end, which a panicking test, a timed
out one or a mutant `cargo mutants` kills never reaches. A stopgap,
`tools/db-sweeper.sh` in the run folder (superuser, `pg_stat_file`), keeps
the server alive meanwhile (DECISIONS.md: "until the harness owns the
sweep").

## Shipped
In the commit that adds this file:
- `harness-core::ports::shell::database` (`Databases`), `PgDatabases` in
  `harness-adapters` (tokio-postgres, the factory's `DATABASE_URL` role,
  plain `DROP DATABASE`, `object_in_use` = busy, kept).
- `harness-workflows::scratch_sweep`: only names carrying their creation
  time (`scratch_<epoch>_<pid>_<seq>_<label>`, `rules_runtime_<epoch>_<pid>`,
  `schema_dump_<epoch>_<pid>`), idle, not a template, not named by
  `DATABASE_URL` / `DATABASE_URL_READONLY`, older than 30 minutes. At most
  every 10 minutes, 200 drops at most per sweep, on one connection, within
  60 s (a hung server holds the loop that long, no more). Event
  `ScratchDatabasesSwept { dropped, busy, left }`, one warning per outage.
- `harness watch`: the sweep runs from the loop after every tick (also on a
  quiet board and during a quota rest), never under `--dry-run`.
- Asset `crates/platform/scratch-db` (`platform-scratch-db`, `ScratchDb`,
  drop `WITH (FORCE)` on its own thread and runtime, 30 s timeout), rules
  block v6 (rule 19), gate `scratch-databases-shared` (test files a task's
  PR *adds*, from the merge base; a milestone's PR and pushes are not
  judged; the 73 existing files may still be edited). The guard is in every
  narrowed checkout (`closure::ALWAYS`) and out of the product's mutants
  (`exclude_globs`).
- `scripts/rules-runtime.sh` creates its `rules_runtime_*` database after
  the build of `crates/server`, right before the server connects to it.
- Harness CI job `postgres` (a `postgres:16` service, a `NOSUPERUSER`
  role): runs the ignored `owned_and_dropped_against_a_real_server` and
  `tests/scratch_db.rs`, so the guard's panic test is proven on every PR.
- `docker/postgres/provision.sh`: the application role (`APP_DATABASE_URL`'s)
  holds `pg_signal_backend` (idempotent, checked with `USAGE`, still
  `NOSUPERUSER`).

Verified before shipping, on a disposable `postgres:16` container (never
the factory's): provision run twice (one membership, `rolsuper = f`);
`FORCE` on another role's session refused without the grant ("permission
denied to terminate process"), accepted with it; `PgDatabases` against a
`NOSUPERUSER` role (`owned_and_dropped_against_a_real_server`, ignored by
default); the installed guard compiled against the engine by path and its
panic test leaving no database (`tests/scratch_db.rs`, ignored by default).

## To check
- [x] The factory's role is **`forgedesmondes`** (its `DATABASE_URL`, owner
      of the test databases), not the role `provision.sh` provisions: the
      container's env names `dnd_app` (`APP_DATABASE_URL` of the shared
      checkout's `.env.local`), so `scripts/postgres.sh provision` never
      grants `forgedesmondes` anything. It already holds the right (granted
      by `postgres`, read 2026-10-24): `select pg_has_role('forgedesmondes',
      'pg_signal_backend', 'USAGE')` is `t` and its `rolsuper` is `f`. Check
      it again before relying on `WITH (FORCE)`; a new factory's role is
      granted by hand (`GRANT pg_signal_backend TO <role>` as `postgres`). (seen 2026-10-24 10:20 Paris: DECISIONS.md — `GRANT pg_signal_backend TO forgedesmondes` on harness-postgres; not re-queried since)
- [x] Restart the watch (`tools/restart_watch.sh`): the start's upgrade
      commits `crates/platform/scratch-db/`, the v6 `CLAUDE.md` block and
      `gates.yml` on `main_agent`, with no "diverged … kept" line for
      `gates.yml` or `CLAUDE.md` in the journal. If `gates.yml` was kept,
      port the gate by hand. (seen 2026-10-24 13:38 Paris: `upgrade: installed 9 file(s) on main_agent: .github/workflows/gates.yml, …, crates/platform/scratch-db/…, scripts/rules-runtime.sh, CLAUDE.md`; neither gates.yml nor CLAUDE.md in the diverged list)
- [x] The application role sees the connection counts of its own databases
      and drops them: no "could not be listed" / "could not be dropped"
      warning in `watch.log` over the first hour. (seen 2026-10-24 13:38–22:52 Paris: no `could not be listed` / `could not be dropped` warning; no ScratchDatabasesSwept line either (nothing dated to drop))
- [ ] The sweep drops only dated names: compare `select datname from
      pg_database` before and after the first `ScratchDatabasesSwept` line —
      `forgedesmondes`, `dnd_helper`, `postgres`, the templates and every
      `<name>_<pid>` of the 73 old tests are still there. Expected numbers:
      the first sweep (at the start) drops every `rules_runtime_*` /
      `schema_dump_*` older than 30 minutes (`select count(*) from
      pg_database where datname ~ '^(rules_runtime|schema_dump)_[0-9]{10}_'`
      taken just before); `scratch_*` drops appear only once product tests
      use the guard, and stay a handful an hour (killed processes only).
- [x] The upgrade carried the new `scripts/rules-runtime.sh` (database
      created after the build): no "diverged … kept" line for it. (seen 2026-10-24 13:38 Paris: in the installed list, not diverged)
- [x] A new product test written by a session uses `ScratchDb::create`; a PR
      that adds one with `CREATE DATABASE` turns `scratch-databases-shared`
      red, and editing an old file stays green. (seen 2026-10-24 22:06 Paris: #24's new tests `analyze-brief/tests/tables.rs` and `Session/tests/entries.rs` use ScratchDb, no CREATE DATABASE; the gate's red case not seen)
- [ ] The 73 old files still create `<name>_<pid>` databases the sweep
      never touches: the +250/h goes on until they move. Keep
      `tools/db-sweeper.sh` running, and open the product task that moves
      them to the guard **with** this commit's rollout, not after. That task
      also lists `crates/platform/scratch-db` in the root `members` by name
      (as `crates/platform/conformance`) and ports the
      `"crates/platform/scratch-db/**"` line into the product's own
      `.cargo/mutants.toml`, which has diverged from the asset and will not
      take it by upgrade.
- [ ] A narrowed session of a DataCapability finds
      `crates/platform/scratch-db/` in its checkout (`closure::ALWAYS`) and
      its path dev-dependency resolves.
- [ ] After that migration, `assert!(false)` after `ScratchDb::create` in a
      worktree leaves no `scratch_*` row; a `cargo mutants` run on one crate
      leaves none older than its run but those of mutants killed by a
      timeout, which the next sweep clears.
- [ ] Over 24 h the database count stays roughly flat (was +250/h) and
      `docker exec harness-postgres du -sh /var/lib/postgresql/data` stops
      growing. Then stop `tools/db-sweeper.sh` by its pid (no broad pkill)
      and record it in DECISIONS.md.
