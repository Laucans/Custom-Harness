//! The ports `init-repo` needs: GitHub, and the disk for `.env.local`.
//!
//! Separated from [`crate::init_repo::config::Config`] on purpose: here,
//! what is **injected**; there, what this run **is worth**.

use std::rc::Rc;

use harness_core::adapters::shell::disk::Disk;
use harness_core::adapters::shell::github::GitHub;

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

    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use harness_core::adapters::shell::disk::Disk;
    use harness_core::domain::Outcome;

    use super::Ports;
    use crate::common::fake_github::FakeGitHub;

    /// A disk in memory: what it holds can be read, what it wrote can be
    /// read back.
    #[derive(Default)]
    pub struct FakeDisk {
        /// The content a path reads as, before any write.
        pub existing: HashMap<PathBuf, String>,
        /// Every write, in order.
        pub written: RefCell<Vec<(PathBuf, String)>>,
    }

    impl FakeDisk {
        /// The last thing written to `path`, if anything was.
        pub fn written_to(&self, path: &Path) -> Option<String> {
            self.written
                .borrow()
                .iter()
                .rev()
                .find(|(p, _)| p == path)
                .map(|(_, content)| content.clone())
        }
    }

    impl Disk for FakeDisk {
        fn exists(&self, path: &Path) -> bool {
            self.existing.contains_key(path)
        }
        fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
            Ok(())
        }
        fn remove_dir_all(&self, _path: &Path) -> Outcome<()> {
            Ok(())
        }
        fn dir_names(&self, _path: &Path) -> Vec<String> {
            Vec::new()
        }
        fn read_to_string(&self, path: &Path) -> Option<String> {
            self.existing.get(path).cloned()
        }
        fn write_to_string(&self, path: &Path, content: &str) -> Outcome<()> {
            self.written
                .borrow_mut()
                .push((path.to_path_buf(), content.to_string()));
            Ok(())
        }
    }

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
