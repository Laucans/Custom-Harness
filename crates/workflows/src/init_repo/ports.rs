//! The ports `init-repo` needs: GitHub, the disk for `.env.local` and the
//! install clone, and `git` for that clone.
//!
//! Separated from [`crate::init_repo::config::Config`] on purpose: here,
//! what is **injected**; there, what this run **is worth**.

use std::rc::Rc;

use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repos;
use harness_core::ports::shell::github::GitHub;

/// The ports of `init-repo`.
pub struct Ports {
    /// The target repository.
    pub gh: Rc<dyn GitHub>,
    /// Where `.env.local` is read and written, and the install clone's files.
    pub disk: Rc<dyn Disk>,
    /// `git`, on the install clone.
    pub repos: Rc<dyn Repos>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for a test that sets up exactly what it reads.

    use std::rc::Rc;

    use super::Ports;
    pub use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_github::FakeGitHub;

    /// Test ports: empty GitHub, empty disk.
    pub fn ports() -> Ports {
        with(Rc::new(FakeGitHub::default()))
    }

    /// Test ports against this GitHub, an empty disk.
    pub fn with(gh: Rc<FakeGitHub>) -> Ports {
        with_disk(gh, Rc::new(FakeDisk::default()))
    }

    /// Test ports against this GitHub and this disk, a `git` that answers
    /// yes to everything.
    pub fn with_disk(gh: Rc<FakeGitHub>, disk: Rc<FakeDisk>) -> Ports {
        Ports {
            gh,
            disk,
            repos: Rc::new(FakeRepos::default()),
        }
    }
}
