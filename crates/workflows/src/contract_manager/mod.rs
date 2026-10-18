//! The contract manager: a contract a consumer needs changed, evolved for
//! every consumer at once.
//!
//! Several milestones build side by side against the interfaces the plan's
//! graph names (contract R), mocked until built. A consumer never edits the
//! contract of an id another milestone produces (gate `contracts-untouched`):
//! it stops with `AGENT_LOOP_STOP: contract <id> — <what it needs>`, the dev
//! loop opens a `harness:contract-change` issue that blocks the task, and
//! this workflow answers it.
//!
//! **One session, one evolution.** It reads the request, the graph (who
//! produces the id, who reads it, its version) and the repository, and
//! answers the new version of the manifest — and the fixture that stands
//! for it until built. The workflow writes both on the integration branch,
//! bumps the id in the graph (`evolving`), opens the task that builds the
//! new version on the producer's milestone, tells every consumer, and closes
//! the request: the task that asked runs again, against the new contract.
//!
//! An id the graph does not know is nobody else's: the requester is told to
//! change it itself, and nothing is paid.
//!
//! Same skeleton as the other workflows (`ARCHITECTURE.md`).

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
