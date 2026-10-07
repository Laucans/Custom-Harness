//! The ports a planner run needs: GitHub, sessions, spending, locks, disk.
//!
//! Separated from [`crate::planner::config::Config`] on purpose: here, what
//! is **injected**; there, what this run **is worth**.
//!
//! The repository map ports are **not** here: they're shared and live in
//! [`crate::common::explore::Ports`].

use std::rc::Rc;

use harness_core::ports::agent::SessionFactory;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;
use harness_core::ports::store::spending::Spending;

/// The ports of a planner run.
pub struct Ports {
    /// What reads the roadmap issue and writes the milestones it opens.
    pub gh: Rc<dyn GitHub>,
    /// What opens a paid session — or runs it dry.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where a step's spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// What holds the lock — one planning run per roadmap item at a time.
    pub locks: Rc<dyn Locks>,
    /// Where the grilling digests are read from, if either exists.
    pub disk: Rc<dyn Disk>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for setting up a round without network.

    use std::path::Path;
    use std::rc::Rc;

    use harness_core::adapters::agent::rehearsal::Rehearsal;
    use harness_core::domain::{Halt, Outcome};
    use harness_core::ports::shell::disk::Disk;
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

    /// A disk that answers every read with absence — the normal case: most
    /// tests exercise no grilling digest at all.
    pub struct NoDigests;

    impl Disk for NoDigests {
        fn read_to_string(&self, _path: &Path) -> Option<String> {
            None
        }
        fn exists(&self, _path: &Path) -> bool {
            unreachable!()
        }
        fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
            unreachable!()
        }
        fn remove_dir_all(&self, _path: &Path) -> Outcome<()> {
            unreachable!()
        }
        fn dir_names(&self, _path: &Path) -> Vec<String> {
            unreachable!()
        }
        fn write_to_string(&self, _path: &Path, _content: &str) -> Outcome<()> {
            unreachable!()
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
            disk: Rc::new(NoDigests),
        }
    }
}
