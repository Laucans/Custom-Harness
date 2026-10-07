//! The advisory review of a PR: a second opinion in fresh context, then
//! notes for the human who must decide if they trust the batch.
//!
//! Triggered by `harness:to-review` on a pull request, not by a loop round.
//! The original design said "a hook on `gh pr create`"; that needs a public
//! URL permanently reachable, which this harness does not have, so the
//! trigger is a label like every other stage of the flow — posed by a human
//! or by whatever opened the PR, and read by the router.
//!
//! **The label is a request, not state this workflow clears.** It is left in
//! place: a PR that already carries a review stops being offered because the
//! review's own rules say so (`data::skip_rules`), which the router applies
//! before mounting anything. Nothing has to remember to clean up.
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
