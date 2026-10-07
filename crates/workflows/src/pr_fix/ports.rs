//! The ports a repair needs: GitHub, sessions, spending, locks.
//!
//! Separated from [`crate::pr_fix::config::Config`] on purpose: here, what
//! is **injected**; there, what this run **is worth**.
//!
//! No `git` port: the session commits and pushes with its own tools, the
//! same way `/code` does today. Taking that over deterministically is its
//! own piece of work (`todo.md`), not something to half-do here.

use std::rc::Rc;

use harness_core::ports::agent::SessionFactory;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;
use harness_core::ports::store::spending::Spending;

/// The ports of a repair attempt.
pub struct Ports {
    /// What reads the PR, its checks and its comments, and removes the label.
    pub gh: Rc<dyn GitHub>,
    /// What opens a paid session — or runs it dry.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where the attempt's spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// What holds the lock — one repair per PR at a time.
    pub locks: Rc<dyn Locks>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for setting up a round without network.

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
