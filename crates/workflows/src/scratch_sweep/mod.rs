//! `harness watch` sweeping the scratch databases a product's tests left
//! behind on its PostgreSQL server.
//!
//! A test that creates a database and panics, times out or is killed by
//! `cargo mutants` never drops it (`ForgeDesMondes`, 2026-10-24: +250
//! databases an hour, 7 939 in all, 68 GB). The fix at the source is the
//! product's `platform-scratch-db` guard, installed by `init-repo`, which
//! drops its database even when the test panics; this sweep is what is left
//! for a process killed outright.
//!
//! **Only what carries its creation time is dropped**: a name of the
//! grammar `data::names` reads (`scratch_<epoch>_…`, the guard's;
//! `rules_runtime_<epoch>_<pid>` and `schema_dump_<epoch>_<pid>`, the
//! installed scripts'), owned by the factory's role, not a template, not a
//! database `DATABASE_URL` or `DATABASE_URL_READONLY` names, with nobody
//! connected, older than `config::OLDER_THAN`. A database a human made has
//! no such name, and is never touched.
//!
//! A **deterministic command, not a workflow** — no `Context`, no stage
//! table, no session, no cost — and a smaller one than its siblings: no
//! subcommand and no trigger of its own. The watch's loop calls it after
//! every tick, whatever the tick did (a quiet board, a quota rest), and it
//! runs at most every `config::EVERY` seconds. It never fails the watch: a
//! server it cannot reach is one warning per outage, and the next sweep
//! tries again.

pub mod config;
pub mod data;
pub mod ports;
pub mod run;
