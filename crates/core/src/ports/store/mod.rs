//! The ports onto what outlives a run: spending, locks, the resume point.
//!
//! All three survive the code they describe — a disposable workspace
//! disappears, the accounting stays. That is what makes a clone disposable.
//!
//! [`spending`] is the one port `harness-core` declares and does **not**
//! implement: writing a row needs a clock, a run id and a machine name, all
//! facts of the launcher. The others are implemented in
//! [`adapters::store`](crate::adapters::store).

pub mod checkpoint;
pub mod lock;
pub mod review;
pub mod spending;
