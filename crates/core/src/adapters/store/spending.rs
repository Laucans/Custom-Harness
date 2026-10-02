//! Where spending is recorded — the port, not the file.
//!
//! [`Ledger`](super::ledger::Ledger) knows how to write a row, but not what
//! time it is, what this run is called, or what machine it runs on. Those
//! three are facts of the launcher, not the execution: an action that just
//! paid a session knows what it spent and for which task, and nothing more.
//!
//! Hence this port. The implementation lives in `harness-launcher`, which
//! alone holds a clock and machine name — and this is also what keeps
//! `harness-core` free of time dependency.

use crate::domain::{Outcome, Spend};

/// What a finished stage cost, as execution knows it.
///
/// Borrowed rather than owned: the row is written in the call, nothing is
/// kept after.
pub struct Entry<'a> {
    /// The round number.
    pub round: u32,
    /// The billed task. Empty on a rollover round, which has none.
    pub task: &'a str,
    /// The stage.
    pub stage: &'a str,
    /// What the session carrier observed — including unobserved fields.
    pub spend: &'a Spend,
    /// `ok`, or the reason for missing response.
    pub outcome: &'a str,
}

/// Where spending is recorded.
pub trait Spending {
    /// Record what a stage cost.
    ///
    /// # Errors
    ///
    /// Write failure, and it **stops the round**: a run's budget is read from
    /// the ledger, and a lost row makes it lie about already-incurred spending.
    fn record(&self, entry: &Entry<'_>) -> Outcome<()>;
}
