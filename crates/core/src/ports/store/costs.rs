//! What tasks of a kind have cost — the port, not the file.
//!
//! The budget gate and the split read it: the first to refuse a stage that
//! would take a task past what its kind costs, the second to size the
//! slices it cuts. The arithmetic is [`crate::domain::budget`]; the rows
//! come from wherever spending is kept.

use crate::domain::Outcome;
use crate::domain::budget::Typical;

/// What tasks of one kind have cost.
pub trait CostHistory {
    /// The distribution of the totals of every finished task tagged with
    /// this `unit` and `profile` — either empty to mean "any". `None` when
    /// none was measured.
    ///
    /// # Errors
    ///
    /// A read failure: an unreadable history is not an empty one.
    fn typical(&self, unit: &str, profile: &str) -> Outcome<Option<Typical>>;
}
