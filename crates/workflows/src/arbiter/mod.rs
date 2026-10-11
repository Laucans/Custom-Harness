//! The arbiter: settles a task's graph question an agent may settle, and
//! hands the rest to a human — see `OBJECTIVE.md`.
//!
//! The `pr_fix` shape: its trigger label is consumed before anything is
//! paid for, so a question nobody can settle does not buy one session per
//! poll. The question itself stays in the body (`## Decision`), where the
//! human reads it with the verdict.

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
