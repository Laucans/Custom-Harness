//! The ports an arbitration needs: GitHub, sessions, spending, locks, the
//! checkout's disk.
//!
//! Separated from [`crate::arbiter::config::Config`] on purpose: here,
//! what is **injected**; there, what this run **is worth**.

use std::rc::Rc;

use harness_core::ports::agent::SessionFactory;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;
use harness_core::ports::store::spending::Spending;

/// The ports of an arbitration.
pub struct Ports {
    /// What reads the task and its milestone, rewrites bodies, links and
    /// labels.
    pub gh: Rc<dyn GitHub>,
    /// What opens the paid session — or runs it dry.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where the session's spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// What holds the lock — one arbitration per task at a time.
    pub locks: Rc<dyn Locks>,
    /// What reads the checkout: which ids the repository already holds.
    pub disk: Rc<dyn Disk>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for setting up a table without network.

    use std::rc::Rc;

    use harness_adapters::agent::rehearsal::Rehearsal;
    use harness_core::domain::{Halt, Outcome};
    use harness_core::ports::store::spending::{Entry, Spending};
    use harness_core::traces::Logbook;

    use super::Ports;
    use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_github::FakeGitHub;
    use crate::common::fake_locks::Grants;

    /// A registry that refuses to write.
    pub struct Nowhere;

    impl Spending for Nowhere {
        fn record(&self, _entry: &Entry<'_>) -> Outcome<()> {
            Err(Halt::Failed(
                "no spending should be recorded in this test".to_string(),
            ))
        }
    }

    /// Test ports, empty GitHub.
    pub fn ports() -> Ports {
        with(Rc::new(FakeGitHub::default()))
    }

    /// Test ports against this GitHub.
    pub fn with(gh: Rc<FakeGitHub>) -> Ports {
        Ports {
            gh,
            sessions: Rc::new(Rehearsal::new(Logbook::null())),
            spending: Rc::new(Nowhere),
            locks: Rc::new(Grants),
            disk: Rc::new(FakeDisk::default()),
        }
    }
}
