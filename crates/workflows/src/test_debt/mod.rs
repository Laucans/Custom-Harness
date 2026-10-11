//! One attempt at the tests a file owes: the mutants `cargo mutants` found
//! no test catching.
//!
//! Triggered by `harness:test-debt` on an open issue, which the CI's mutants
//! rotation opens — one per file, `Test debt: <file>` — and labels again
//! whenever a later run still finds mutants there (`scripts/test-debt.sh`,
//! installed by `init-repo`).
//!
//! **One label, one attempt.** The request is consumed in the free stage,
//! before anything is paid for, so a file whose mutants no session can catch
//! does not buy a session per poll. The next rotation that judges the file
//! asks again if it still owes tests — the same shape as `pr_fix`.
//!
//! Same skeleton as the other workflows (`ARCHITECTURE.md`):
//!
//! - [`data`] — the state, and what a test-debt issue says;
//! - [`action`] — what it reads for free, the label it consumes, and what it
//!   asks of its session;
//! - [`checks`] — what judges without ever writing;
//! - [`orchestration`] — what sequences: the table, the round, the run.
//!
//! [`ports`], [`config`] and [`run`] stay at the root: none of the three is
//! a step.
//!
//! **What this workflow does not do**: it never changes the code under test,
//! and never decides the mutants are caught. That is read by the next
//! rotation that judges the file — an attempt that judged its own result
//! would be the one judge nobody can appeal to.

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
