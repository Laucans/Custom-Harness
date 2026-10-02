//! The advisory review of a PR: a second opinion in fresh context, then
//! notes for the human who must decide if they trust the batch.
//!
//! Triggered by a hook on `gh pr create`, not by a loop round — see
//! `docs/CUTOVER.md` for what remains human in this trigger.
//!
//! Same skeleton as other workflows (`ARCHITECTURE.md`):
//!
//! - [`data`] — what the review reads, pure business logic;
//! - [`action`] — what it writes;
//! - [`checks`] — what judges without ever writing;
//! - [`orchestration`] — what sequences: table, round, and entire review.
//!
//! [`ports`], [`config`] and [`run`] stay at the root: none of the three is
//! a step. **The design surface, and the only one**: `orchestration::stages::table`.

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
