//! What a pull request already cost — the port.
//!
//! The comment a review posts says what it cost. The stage that posts thus
//! has to *read* spending, which
//! [`Spending`](crate::ports::store::spending::Spending) — a write port —
//! does not answer; hence this one, small on purpose.
//!
//! The implementation, a read of the one cost ledger on the pull request's
//! rows, lives in `harness_adapters::store::review_ledger`.

use crate::domain::Outcome;

/// What a pull request has already cost.
pub trait ReviewCosts {
    /// The total spent on this PR — its review passes and any repair —
    /// formatted for a comment, or empty when nothing was recorded.
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
