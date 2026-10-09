//! Where events are kept — the port, not the database.
//!
//! Several processes write at once (the watch and each of its lanes) and
//! several read (the view, its notifications), so the store is shared and
//! durable: an event written while nobody looks is still there to read. A
//! reader goes by `id`, which only grows: "everything after the last one I
//! saw" is the whole of a cursor.
//!
//! The clock is the caller's: `at` is passed in, so `harness-core` keeps no
//! time of its own.

use serde::Serialize;

use crate::domain::Outcome;
use crate::traces::Event;

/// An event as kept: when, by whom, and in what order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stored {
    /// Its place in the store; later events have larger ids.
    pub id: u64,
    /// When it happened, as the traces write a clock (`2026-10-09T01:02:03Z`).
    pub at: String,
    /// Who told it: `watch`, or a run (`agent-loop/20261009-004034-15`).
    pub source: String,
    /// What happened.
    pub event: Event,
}

/// The plant's events, shared by every process of the checkout.
pub trait EventLog {
    /// Keeps `event`, told by `source` at `at`.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn append(&self, at: &str, source: &str, event: &Event) -> Outcome<()>;

    /// The events after `id`, oldest first, at most `limit`.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn after(&self, id: u64, limit: usize) -> Outcome<Vec<Stored>>;

    /// The events stamped between `from` and `to` (either open, both
    /// inclusive), oldest first, at most `limit` — the newest ones when there
    /// are more.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn between(&self, from: Option<&str>, to: Option<&str>, limit: usize) -> Outcome<Vec<Stored>>;
}
