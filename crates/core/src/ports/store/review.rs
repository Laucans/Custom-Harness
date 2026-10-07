//! What a review already cost — the port.
//!
//! A review charges per PR and per pass, never by round or task, and the
//! comment it posts says the total. The stage that posts thus has to *read*
//! spending, which [`Spending`](crate::ports::store::spending::Spending) —
//! a write port — does not answer; hence this one, small on purpose.
//!
//! The implementation, a ledger with its own frozen columns, lives in
//! [`adapters::store::review_ledger`](crate::adapters::store::review_ledger).

use crate::domain::Outcome;

/// What a review's passes have already cost.
pub trait ReviewCosts {
    /// The total spent on this PR, formatted for a comment, or empty when
    /// nothing was recorded.
    ///
    /// Empty rather than `0`: a carrier that reports no usage bought a review
    /// all the same, and printing `$0` would claim it was free.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if rows exist but
    /// cannot be read — the comment then says so instead of claiming `$0`.
    fn cost_of(&self, pr: &str) -> Outcome<String>;
}
