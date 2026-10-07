//! Where spending is recorded — the port, not the file.
//!
//! [`Ledger`](crate::adapters::store::ledger::Ledger) knows how to write a row, but not what
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
    /// The billed task. Empty on a round that works on no task.
    pub task: &'a str,
    /// The stage.
    pub stage: &'a str,
    /// What the session carrier observed — including unobserved fields.
    pub spend: &'a Spend,
    /// `ok`, or the reason for missing response.
    pub outcome: &'a str,
    /// The fingerprint of the prompt that was sent
    /// ([`breaker::fingerprint`](crate::domain::breaker::fingerprint)).
    ///
    /// Recorded so that a later run can recognise *this* session rather than
    /// merely "a session on the same stage": it is what lets the breaker
    /// forget a failure once the prompt changes.
    pub fingerprint: &'a str,
}

/// Where spending is recorded, and what it remembers of past failures.
pub trait Spending {
    /// Record what a stage cost.
    ///
    /// # Errors
    ///
    /// Write failure, and it **stops the round**: a run's budget is read from
    /// the ledger, and a lost row makes it lie about already-incurred spending.
    fn record(&self, entry: &Entry<'_>) -> Outcome<()>;

    /// How many times this exact session — same task, same stage, same
    /// prompt — already ended in error, counting back from the most recent.
    ///
    /// The default is `0`, which is the truthful answer for a ledger that
    /// keeps nothing: an implementation that remembers no row knows of no
    /// failure. One that does keep rows must answer from them, or the
    /// breaker it feeds never trips.
    ///
    /// # Errors
    ///
    /// A read failure, and it **stops the round** before paying: a history
    /// that cannot be read is not an empty history, and reading it as one is
    /// exactly how a loop pays five times for the same refusal.
    fn failures(&self, task: &str, stage: &str, fingerprint: &str) -> Outcome<u32> {
        let _ = (task, stage, fingerprint);
        Ok(0)
    }

    /// Keeps the carrier's last view of its rate-limit windows, for the *next*
    /// run's preflight to read.
    ///
    /// Not a row of the ledger: the ledger is a history, and only the latest
    /// reading means anything. Here all the same, because this is the port that
    /// already answers "what should a later run know about an earlier one" — and
    /// because the alternative was the agent adapter writing a file itself, which
    /// put persistence in the one layer that must stay swappable.
    ///
    /// **Does nothing by default**, and that is the honest default: an
    /// implementation that keeps no history keeps no reading either, and a
    /// preflight that finds none starts. Nothing is lost but an optimisation.
    ///
    /// # Errors
    ///
    /// Never, for the default. An implementation that writes may report a
    /// failure, and callers are expected to carry on regardless: losing a
    /// convenience file must not stop a round.
    fn remember_quota(&self, reading: &crate::domain::quota::Reading) -> Outcome<()> {
        let _ = reading;
        Ok(())
    }
}
