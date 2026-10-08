//! An in-memory `Disk`, shared by workflows that need one for their tests.
//!
//! A fake adapter, not a mock: what it holds reads back, what was written to
//! it can be re-read. This is what `CLAUDE.md` asks for — "inject a fake
//! adapter; nothing mocks at the call site" — and it is what lets a test
//! prove a file was written without putting a real folder at risk.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use harness_core::domain::Outcome;
use harness_core::ports::shell::disk::Disk;

/// A disk in memory: what it holds can be read, what it wrote can be read back.
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
        // A path it holds, or a folder on the way to one.
        self.existing
            .keys()
            .any(|known| known == path || known.starts_with(path))
    }
    fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
        Ok(())
    }
    fn remove_dir_all(&self, _path: &Path) -> Outcome<()> {
        Ok(())
    }
    fn dir_names(&self, path: &Path) -> Vec<String> {
        // The folders directly under `path`: the next component of every
        // held path that goes deeper than one more level.
        let mut names: Vec<String> = self
            .existing
            .keys()
            .filter_map(|known| known.strip_prefix(path).ok())
            .filter(|rest| rest.components().count() > 1)
            .filter_map(|rest| rest.components().next())
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names.dedup();
        names
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
