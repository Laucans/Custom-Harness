//! The ports `init-repo` needs: GitHub, and the disk for `.env.local`.
//!
//! Separated from [`crate::init_repo::config::Config`] on purpose: here,
//! what is **injected**; there, what this run **is worth**.

use std::rc::Rc;

use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;

/// The ports of `init-repo`.
pub struct Ports {
    /// The target repository.
    pub gh: Rc<dyn GitHub>,
    /// Where `.env.local` is read and written.
    pub disk: Rc<dyn Disk>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for a test that sets up exactly what it reads.

    use std::rc::Rc;

    use super::Ports;
    pub use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_github::FakeGitHub;

    /// Test ports: empty GitHub, empty disk.
    pub fn ports() -> Ports {
        with(Rc::new(FakeGitHub::default()))
    }

    /// Test ports against this GitHub, an empty disk.
    pub fn with(gh: Rc<FakeGitHub>) -> Ports {
        Ports {
            gh,
            disk: Rc::new(FakeDisk::default()),
        }
    }

    /// Test ports against this GitHub and this disk.
    pub fn with_disk(gh: Rc<FakeGitHub>, disk: Rc<FakeDisk>) -> Ports {
        Ports { gh, disk }
    }
}
