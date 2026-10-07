//! Opens the milestones that deliver one roadmap item, one run.
//!
//! Triggered on an open `harness:roadmap` issue with no milestone open under
//! it yet — the router decides this (`crate::common::routing`), not a label
//! on the issue itself: a roadmap item carries no state of its own to flip.
//!
//! Same skeleton as the other workflows (`ARCHITECTURE.md`):
//!
//! - [`data`] — the state, and parsing the plan out of a reply's text;
//! - [`action`] — what it writes: the milestones, their links, their label;
//! - [`checks`] — what judges without ever writing;
//! - [`orchestration`] — what sequences: the table, the round, the run.
//!
//! [`ports`], [`config`] and [`run`] stay at the root: none of the three is
//! a step.

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
