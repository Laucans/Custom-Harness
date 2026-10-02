//! Refinement of an issue: rewrites its body in five canonical sections,
//! one round per run.
//!
//! Triggered by the `harness:refinement` label, placed by hand or by
//! `/planner` — see `docs/CUTOVER.md`.
//!
//! Same skeleton as other workflows (`ARCHITECTURE.md`):
//!
//! - [`data`] — what refinement reads, pure business logic: its state, round
//!   counter, five body sections;
//! - [`action`] — what it writes;
//! - [`checks`] — what judges without ever writing;
//! - [`orchestration`] — what sequences: texts, table, round, entire run.
//!
//! [`ports`], [`config`] and [`run`] stay at the root: none of the three is
//! a step. **The design surface, and the only one**:
//! `orchestration::stages::table`.

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
