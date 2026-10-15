//! The ports onto what outlives a run: spending, locks, the resume point,
//! the human's prompt edits, the register of factories.
//!
//! All of them survive the code they describe — a disposable workspace
//! disappears, the accounting stays. That is what makes a clone disposable.
//!
//! [`spending`] is the one port `harness-core` declares and does **not**
//! implement: writing a row needs a clock, a run id and a machine name, all
//! facts of the launcher. The others are implemented in
//! `harness_adapters::store`.

pub mod checkpoint;
pub mod costs;
pub mod events;
pub mod factories;
pub mod lock;
pub mod prompts;
pub mod review;
pub mod spending;
