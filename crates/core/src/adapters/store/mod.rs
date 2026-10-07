//! What the harness writes and re-reads on disk: its accounting, its locks,
//! and its resume point.
//!
//! The implementations behind [`ports::store`](crate::ports::store), plus the
//! two ledgers the launcher writes directly — `ledger` for rounds and
//! `error_ledger` for a run that broke, neither of which the hexagon reads.

pub mod checkpoint;
pub mod error_ledger;
pub mod ledger;
pub mod lock;
pub mod review_ledger;
