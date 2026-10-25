//! The volume a path lives on: how much room is left on it.
//!
//! One fact, asked by the watch before it starts a lane: four cold product
//! builds at once took a 460 GB disk to 1 GB free (2026-10-24 22:28), and a
//! session that cannot write its build dies in ways the doctor cannot treat.
//! A port, so the floor's rule is tested without filling a disk.

use std::path::Path;

use crate::domain::Outcome;

/// What the watch asks of the volume its lanes build on.
pub trait Volume {
    /// The bytes an unprivileged process may still write on the volume
    /// holding `path`.
    ///
    /// # Errors
    /// If the volume could not be asked — `path` missing, a system error.
    fn free_bytes(&self, path: &Path) -> Outcome<u64>;
}
