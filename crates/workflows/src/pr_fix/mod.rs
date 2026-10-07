//! One repair attempt on a pull request whose CI has gone red.
//!
//! Triggered by `harness:pr-fix` on a PR **and** at least one check that has
//! concluded in failure. Both are required: the label alone would send a
//! session at a PR that is merely still building, and a red check alone
//! would repair without anyone having asked.
//!
//! **One label, one attempt.** The request is consumed in the free stage,
//! before anything is paid for, so a PR nobody can fix does not buy a
//! session per poll. Re-posing the label by hand is how a second attempt is
//! asked for — the same shape as every other tap in this harness.
//!
//! Same skeleton as the other workflows (`ARCHITECTURE.md`):
//!
//! - [`data`] — the state the stages pass each other;
//! - [`action`] — what it reads for free, what it asks of its session, and
//!   the one label it writes;
//! - [`checks`] — what judges without ever writing;
//! - [`orchestration`] — what sequences: the table, the round, the run.
//!
//! [`ports`], [`config`] and [`run`] stay at the root: none of the three is
//! a step.
//!
//! **What this workflow does not do**: it never merges, never closes the
//! PR, and never decides the repair worked. Whether CI went green is read
//! on a later poll, by whatever is waiting on that PR — a repair that
//! judged its own result would be the one judge nobody can appeal to.

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
