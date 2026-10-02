//! The ports a review needs: GitHub, sessions, spending, locks, clock.
//!
//! Separated from [`crate::pr_review::config::Config`] on purpose: here,
//! what is **injected**; there, what this run **is worth**. One type for both
//! would answer two questions under one name.
//!
//! [`Ports::now`] is a function pointer, not an `Rc<dyn Trait>`, but it's
//! still a port and it's here for that reason: neither `harness-core` nor
//! workflows carry a time dependency — it's the launcher that knows the time.
//! The port's form doesn't change its nature.

use std::rc::Rc;

use harness_core::adapters::agent::SessionFactory;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::store::lock::Locks;
use harness_core::adapters::store::spending::Spending;

/// The ports of a review.
pub struct Ports {
    /// The PR board, and where the comment is posted.
    pub gh: Rc<dyn GitHub>,
    /// What opens a paid session — or runs it dry.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where a pass's spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// What holds the lock for a review — one per PR at a time.
    pub locks: Rc<dyn Locks>,
    /// The instant to display in the header of the posted comment.
    pub now: fn() -> String,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for setting up a table without network.
    //!
    //! The two paid ports **refuse** rather than do nothing: setting up a
    //! table must not open any session and record nothing.

    use std::rc::Rc;

    use async_trait::async_trait;
    use harness_core::adapters::agent::{Session, SessionFactory, SessionSpec};
    use harness_core::adapters::store::lock::DirLocks;
    use harness_core::adapters::store::spending::{Entry, Spending};
    use harness_core::domain::{Halt, Outcome};

    use super::Ports;
    use crate::common::fake_github::FakeGitHub;

    /// A factory that refuses to open.
    pub struct NoSessions;

    #[async_trait(?Send)]
    impl SessionFactory for NoSessions {
        async fn open(&self, _spec: &SessionSpec) -> Outcome<Box<dyn Session>> {
            Err(Halt::Failed(
                "no session should open in this test".to_string(),
            ))
        }
    }

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
            sessions: Rc::new(NoSessions),
            spending: Rc::new(Nowhere),
            locks: Rc::new(DirLocks),
            now: || "2026-10-02 16:00".to_string(),
        }
    }
}
