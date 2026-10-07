//! A lock that always grants, shared by workflows that wire one in their tests.
//!
//! A fake adapter, not a mock, and the reason it exists rather than
//! `DirLocks`: setting up a table must not create a `.lock-*` directory
//! anywhere. A test that wants the *refused* case declares its own port
//! rather than racing a real filesystem for it.

use std::path::Path;

use harness_core::domain::Outcome;
use harness_core::ports::store::lock::Locks;

/// A lock nobody else holds.
pub struct Grants;

impl Locks for Grants {
    fn acquire(&self, _dir: &Path, _name: &str) -> Outcome<bool> {
        Ok(true)
    }
    fn release(&self, _dir: &Path, _name: &str) {}
}
