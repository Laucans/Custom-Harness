//! The ports that the loop's actions need: GitHub, sessions, spending.
//!
//! Separated from [`crate::dev_loop::config::Config`] on purpose: this is only
//! `Rc<dyn Trait>`, never a value. Mixing them in a single type was tried and
//! abandoned — an object that carries both ports and config answers two
//! different questions ("how it's made" and "what does this run change") and
//! the name that would fit one would betray the other.
//!
//! No `Rc` is cloned at each stage: the table takes one per action that needs
//! it, once, at assembly.

use std::rc::Rc;

use harness_core::ports::agent::SessionFactory;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

/// The ports that a loop round needs.
pub struct Ports {
    /// The issue board, and the labels placed on it.
    pub gh: Rc<dyn GitHub>,
    /// What opens a paid session — or runs it dry.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where a stage's spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// What the preflight gates check for — skill files, installed
    /// dependencies — through a port rather than a path alone.
    pub disk: Rc<dyn Disk>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that lead nowhere, for setting up a table without network.
    //!
    //! The two fixed ports **refuse** rather than do nothing: setting up a
    //! table must not open any session and record nothing, and a silent port
    //! would let the opposite happen without a test seeing it.

    use std::path::Path;
    use std::rc::Rc;

    use async_trait::async_trait;
    use harness_core::domain::{Halt, Outcome};
    use harness_core::ports::agent::{Session, SessionFactory, SessionSpec};
    use harness_core::ports::shell::disk::Disk;
    use harness_core::ports::store::spending::{Entry, Spending};

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

    /// A disk that answers every read with absence — the normal case: most
    /// tests don't exercise a preflight disk read at all.
    pub struct NoDiskReads;

    impl Disk for NoDiskReads {
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
            sessions: Rc::new(NoSessions),
            spending: Rc::new(Nowhere),
            disk: Rc::new(NoDiskReads),
        }
    }
}
