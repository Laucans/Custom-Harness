//! An in-memory register of relays, for the dev loop's tests.

use std::cell::RefCell;
use std::collections::HashMap;

use harness_core::domain::Outcome;
use harness_core::ports::store::handoffs::{Handoff, Handoffs};

/// Relays kept in memory, taken once as the real register takes them.
#[derive(Default)]
pub struct FakeHandoffs {
    /// The relays left, by key.
    pub left: RefCell<HashMap<String, Handoff>>,
}

impl Handoffs for FakeHandoffs {
    fn leave(&self, key: &str, handoff: &Handoff) -> Outcome<()> {
        self.left
            .borrow_mut()
            .insert(key.to_string(), handoff.clone());
        Ok(())
    }

    fn take(&self, key: &str) -> Outcome<Option<Handoff>> {
        Ok(self.left.borrow_mut().remove(key))
    }
}
