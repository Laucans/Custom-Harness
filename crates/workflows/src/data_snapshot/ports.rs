//! The ports a data snapshot needs: GitHub for the heads and the archive's
//! index, `git` for the snapshot clone, the disk for its files, a process
//! for the target's dump script, and the clock.

use std::rc::Rc;

use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repos;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::shell::process::Process;

/// The ports of a data snapshot.
pub struct Ports {
    /// The branches' heads, and the archive's index without a clone.
    pub gh: Rc<dyn GitHub>,
    /// `git`, on the snapshot clone.
    pub repos: Rc<dyn Repos>,
    /// The clone's files: the script, the model, the archive.
    pub disk: Rc<dyn Disk>,
    /// Runs the dump script.
    pub process: Rc<dyn Process>,
    /// The injected clock: ISO-8601 UTC, to the second.
    pub now: fn() -> String,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports for a test that sets up its own GitHub, disk and process.

    use std::rc::Rc;

    use super::Ports;
    use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_github::FakeGitHub;
    use crate::common::fake_process::FakeProcess;

    /// A fixed clock.
    pub fn now() -> String {
        "2026-10-09T12:00:00Z".to_string()
    }

    /// Test ports over these fakes.
    pub fn with(
        gh: Rc<FakeGitHub>,
        repos: FakeRepos,
        disk: Rc<FakeDisk>,
        process: Rc<FakeProcess>,
    ) -> Ports {
        Ports {
            gh,
            repos: Rc::new(repos),
            disk,
            process,
            now,
        }
    }
}
