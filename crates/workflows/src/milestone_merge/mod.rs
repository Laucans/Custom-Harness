//! `harness watch` merging a finished milestone: `harness milestone-merge`,
//! in effect, though nothing exposes it as its own subcommand — the router
//! is its only caller.
//!
//! A **deterministic command, not a workflow** — same departure as
//! `init_repo`, and for the same reason: no `Context`, no stage table, no
//! session, no cost. What `checks/` would hold (the one predicate this
//! needs) lives in `data::audit` as a pure function over issues already
//! read.
//!
//! Two-tick flow, by design: tasks closed but no PR yet → opens the PR and
//! stops; a PR exists but CI hasn't finished → waits; CI green → merges and
//! poses `harness:waiting-merge` **on the milestone**, not a task — the
//! porter the grilling session chose, since the milestone, not any one
//! task, is what's actually waiting on a human merge to `main`.

pub mod action;
pub mod config;
pub mod data;
pub mod ports;
pub mod run;
