//! The one port `milestone_merge` needs: GitHub. No checkout, no session —
//! every read and write here is an API call.

use std::rc::Rc;

use harness_core::ports::shell::github::GitHub;

/// The ports of a merge attempt.
pub struct Ports {
    /// What reads the milestone's tasks and PR, and merges it.
    pub gh: Rc<dyn GitHub>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for a test that provides its own `FakeGitHub`.

    use std::rc::Rc;

    use super::Ports;
    use crate::common::fake_github::FakeGitHub;

    /// Test ports, empty GitHub.
    pub fn ports() -> Ports {
        with(Rc::new(FakeGitHub::default()))
    }

    /// Test ports against this GitHub.
    pub fn with(gh: Rc<FakeGitHub>) -> Ports {
        Ports { gh }
    }
}
