//! The ports a refinement round needs: GitHub, sessions, spending, locks.
//!
//! Separated from [`crate::refinement::config::Config`] on purpose: here,
//! what is **injected**; there, what this run **is worth**.
//!
//! The repository map ports are **not** here: they're shared and live in
//! [`crate::common::explore::Ports`].

use std::rc::Rc;

use harness_core::ports::agent::SessionFactory;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;
use harness_core::ports::store::spending::Spending;

/// The ports of a refinement round.
pub struct Ports {
    /// What reads the issue, rewrites its body, and places labels.
    pub gh: Rc<dyn GitHub>,
    /// What opens a paid session — or runs it dry.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where a step's spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// What holds the lock — one refinement per issue at a time.
    pub locks: Rc<dyn Locks>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for setting up a table without network.
    //!
    //! The session factory **rehearses** here rather than refuses, unlike
    //! other workflows: the preflight tests run the entire sequence in dry-run,
    //! and a refusing port would fail on the first step instead of testing what
    //! they test.

    use std::rc::Rc;

    use harness_core::adapters::agent::rehearsal::Rehearsal;
    use harness_core::domain::{Halt, Outcome};
    use harness_core::ports::store::spending::{Entry, Spending};
    use harness_core::traces::Logbook;

    use super::Ports;
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
        }
    }
}
