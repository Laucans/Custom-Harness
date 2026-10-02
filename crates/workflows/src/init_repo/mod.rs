//! `harness init-repo <url>`: the `harness:*` labels, the integration
//! branch, a read-only audit, and the link written to `.env.local`.
//!
//! A deterministic command, not a workflow: no `Context`, no stage table, no
//! session, no cost. Nothing here is written to `costs.tsv`.
//!
//! **Departs from the skeleton in `../ARCHITECTURE.md` §1**: no
//! `orchestration/`, no `checks/`. A stage table would name one entry for a
//! sequence that does not exist, and a `Verification` would judge against a
//! `Context<S>` nobody carries here — `init_repo` sequences nothing. What
//! `checks/` would hold (the criteria behind the read-only audit) lives in
//! [`data::audit`] as pure functions over text already read.
//!
//! - [`data`] — what is read and decided: the plan, the audit, the report,
//!   the env-file rewrite. Pure business logic.
//! - [`action`] — the writes: labels, the branch, `.env.local`.
//! - [`ports`], [`config`], [`run`] stay at the root, as in every workflow.

pub mod action;
pub mod config;
pub mod data;
pub mod ports;
pub mod run;
