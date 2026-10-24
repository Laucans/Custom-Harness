//! A database server held in memory, for the sweep's tests.
//!
//! A fake adapter, not a mock: it lists what it was given, drops what is
//! not busy, refuses what it was told to, stops where it was told its
//! deadline falls, and says what it dropped.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::database::{Databases, Dropped, Dropping, OwnedDatabase};

/// The role's databases, in memory.
#[derive(Default)]
pub struct FakeDatabases {
    /// What the role owns.
    pub owned: RefCell<Vec<OwnedDatabase>>,
    /// The databases someone connects to before the drop.
    pub busy: Vec<String>,
    /// The databases whose drop the server refuses.
    pub refused: Vec<String>,
    /// Whether the server is unreachable.
    pub down: Cell<bool>,
    /// Whether the server, having listed, is lost before the drops.
    pub lost: bool,
    /// How many names a pass reaches before its deadline; all when `None`.
    pub reaches: Option<usize>,
    dropped: RefCell<Vec<String>>,
    listings: Cell<u32>,
    passes: RefCell<Vec<(usize, Duration)>>,
}

impl FakeDatabases {
    /// A server whose role owns `names`, nobody connected.
    pub fn idle(names: &[&str]) -> Self {
        Self {
            owned: RefCell::new(
                names
                    .iter()
                    .map(|name| OwnedDatabase {
                        name: (*name).to_string(),
                        connections: 0,
                        template: false,
                    })
                    .collect(),
            ),
            ..Self::default()
        }
    }

    /// The databases dropped, in order.
    pub fn dropped(&self) -> Vec<String> {
        self.dropped.borrow().clone()
    }

    /// How many times the role's databases were listed.
    pub fn listings(&self) -> u32 {
        self.listings.get()
    }

    /// Each pass of drops asked for: how many names, within how long.
    pub fn passes(&self) -> Vec<(usize, Duration)> {
        self.passes.borrow().clone()
    }
}

#[async_trait(?Send)]
impl Databases for FakeDatabases {
    async fn owned(&self) -> Outcome<Vec<OwnedDatabase>> {
        self.listings.set(self.listings.get() + 1);
        if self.down.get() {
            return Err(Halt::Failed("connection refused".to_string()));
        }
        Ok(self.owned.borrow().clone())
    }

    async fn drop_idle(&self, names: &[String], within: Duration) -> Outcome<Vec<Dropping>> {
        self.passes.borrow_mut().push((names.len(), within));
        if self.lost {
            return Err(Halt::Failed("connection refused".to_string()));
        }
        let reached = names.iter().take(self.reaches.unwrap_or(names.len()));
        Ok(reached
            .map(|name| {
                let dropped = if self.refused.contains(name) {
                    Dropped::Refused("permission denied".to_string())
                } else if self.busy.contains(name) {
                    Dropped::Busy
                } else {
                    self.owned.borrow_mut().retain(|db| &db.name != name);
                    self.dropped.borrow_mut().push(name.clone());
                    Dropped::Gone
                };
                Dropping {
                    name: name.clone(),
                    dropped,
                }
            })
            .collect())
    }
}
