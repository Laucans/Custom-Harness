//! The ports `init-repo` needs: GitHub, the disk for `.env.local` and the
//! install clone, `git` for that clone, and a process for the script the
//! install runs in it (`scripts/register.sh`).
//!
//! Separated from [`crate::init_repo::config::Config`] on purpose: here,
//! what is **injected**; there, what this run **is worth**.

use std::rc::Rc;

use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repos;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::shell::process::Process;

/// The ports of `init-repo`.
pub struct Ports {
    /// The target repository.
    pub gh: Rc<dyn GitHub>,
    /// Where `.env.local` is read and written, and the install clone's files.
    pub disk: Rc<dyn Disk>,
    /// `git`, on the install clone.
    pub repos: Rc<dyn Repos>,
    /// The install clone's own scripts.
    pub process: Rc<dyn Process>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for a test that sets up exactly what it reads.

    use std::rc::Rc;

    use super::Ports;
    pub use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_github::FakeGitHub;
    use crate::common::fake_process::FakeProcess;

    /// Test ports: empty GitHub, empty disk.
    pub fn ports() -> Ports {
        with(Rc::new(FakeGitHub::default()))
    }

    /// Test ports against this GitHub, an empty disk.
    pub fn with(gh: Rc<FakeGitHub>) -> Ports {
        with_disk(gh, Rc::new(FakeDisk::default()))
    }

    /// Test ports against this GitHub and this disk, a `git` and scripts
    /// that answer yes to everything.
    pub fn with_disk(gh: Rc<FakeGitHub>, disk: Rc<FakeDisk>) -> Ports {
        Ports {
            gh,
            disk,
            repos: Rc::new(FakeRepos::default()),
            process: Rc::new(FakeProcess::default()),
        }
    }
}
