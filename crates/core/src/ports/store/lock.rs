//! A lock: at most one carrier of a given name at a time — the port.
//!
//! Prevents `pr_review` and `refinement` from double-running — two hooks on
//! the same PR, or two refinement rounds on the same issue, would each post
//! twice. The implementation and why it is a `mkdir` live in
//! [`adapters::store::lock`](crate::adapters::store::lock).

use std::path::Path;

use crate::domain::Outcome;

/// What a lock demands from the disk.
pub trait Locks {
    /// Attempt to acquire the lock `name` under `dir`. True if this call
    /// acquired it, false if someone else already holds it.
    ///
    /// # Errors
    /// If `dir` could not be created.
    fn acquire(&self, dir: &Path, name: &str) -> Outcome<bool>;

    /// Release the lock. Silent if it no longer exists — releasing it twice
    /// must not be an error.
    fn release(&self, dir: &Path, name: &str);
}
