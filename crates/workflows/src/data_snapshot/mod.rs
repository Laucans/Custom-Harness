//! `harness watch` keeping the product's data model, version by version.
//!
//! After a merge moves `main`, `main_agent` or a milestone branch, the head is
//! checked out, the target's `scripts/dump-schema.sh` prints the schema its
//! migrations build (`pg_dump --schema-only`), and that dump, with the
//! agents' `data/model.json` of the same commit, is archived on the target's
//! branch `harness/data-model` — where the factory view reads every version.
//!
//! A **deterministic command, not a workflow** — same departure as
//! `main_agent_merge`: no `Context`, no stage table, no session, no cost.
//!
//! The archive, on its own orphan branch:
//!
//! ```text
//! index.json                                   heads read, snapshots kept
//! <branch>/<YYYYMMDDTHHMMSSZ>-<sha7>/schema.sql
//! <branch>/<YYYYMMDDTHHMMSSZ>-<sha7>/model.json
//! ```
//!
//! A head whose dump and model match the branch's last snapshot moves the
//! head in `index.json` and adds no folder: the versions a human browses are
//! the ones that changed something.

pub mod action;
pub mod config;
pub mod data;
pub mod ports;
pub mod run;
