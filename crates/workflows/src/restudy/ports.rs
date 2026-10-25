//! The ports a re-study needs: GitHub, its checkout, sessions, spending,
//! locks.

use std::rc::Rc;

use harness_core::ports::agent::SessionFactory;
use harness_core::ports::shell::git::Repo;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::lock::Locks;
use harness_core::ports::store::spending::Spending;

/// The ports of a re-study.
pub struct Ports {
    /// What reads the task, its milestone and its siblings, and opens the
    /// slices.
    pub gh: Rc<dyn GitHub>,
    /// The re-study's checkout: what tells that the session committed
    /// nothing.
    pub git: Rc<dyn Repo>,
    /// What opens a paid session — or runs it dry.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where the re-study's spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// What holds the lock — one re-study per task at a time.
    pub locks: Rc<dyn Locks>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for setting up a round without network.

    use std::path::Path;
    use std::rc::Rc;

    use harness_adapters::agent::rehearsal::Rehearsal;
    use harness_core::domain::{Halt, Outcome};
    use harness_core::ports::shell::git::Repos;
    use harness_core::ports::store::spending::{Entry, Spending};
    use harness_core::traces::Logbook;

    use super::Ports;
    use crate::common::fake_git::FakeRepos;
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
        with(Rc::new(FakeGitHub::default()), &FakeRepos::default())
    }

    /// Test ports against this GitHub and this `git`.
    pub fn with(gh: Rc<FakeGitHub>, git: &FakeRepos) -> Ports {
        Ports {
            gh,
            git: git.at(Path::new("/w/restudy")),
            sessions: Rc::new(Rehearsal::new(Logbook::null())),
            spending: Rc::new(Nowhere),
            locks: Rc::new(Grants),
        }
    }
}
