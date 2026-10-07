//! Opens the task slices that deliver one milestone, one run.
//!
//! Triggered by `harness:ready` on a `harness:milestone` issue. On success,
//! removes `harness:ready` and poses `harness:triggered` — in that order,
//! last, so a re-poll never re-splits a milestone it already saw.
//!
//! Same skeleton as the other workflows (`ARCHITECTURE.md`):
//!
//! - [`data`] — the state, and parsing the slice plan out of a reply's text;
//! - [`action`] — what it writes: the tasks, their links, the two labels;
//! - [`checks`] — what judges without ever writing;
//! - [`orchestration`] — what sequences: the table, the round, the run.
//!
//! [`ports`], [`config`] and [`run`] stay at the root: none of the three is
//! a step.
//!
//! Unlike `planner`, this workflow does **not** wire the shared repository
//! map (`common::explore`): splitting a milestone into tasks doesn't need
//! it the way verifying a roadmap item's dependencies does.

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
