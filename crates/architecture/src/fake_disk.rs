//! A read-only `Disk` in memory, for this crate's tests.
//!
//! A fake adapter, not a mock: what it holds reads back. Nothing here writes,
//! so a write is refused rather than silently dropped — a test that reaches
//! one is wrong about this crate.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::disk::Disk;

/// A disk in memory: a path reads as what `existing` holds for it.
#[derive(Default)]
pub struct FakeDisk {
    /// The content each path reads as.
    pub existing: HashMap<PathBuf, String>,
}

impl Disk for FakeDisk {
    fn exists(&self, path: &Path) -> bool {
        // A path it holds, or a folder on the way to one.
        self.existing
            .keys()
            .any(|known| known == path || known.starts_with(path))
    }
    fn create_dir_all(&self, path: &Path) -> Outcome<()> {
        refused(path)
    }
    fn remove_dir_all(&self, path: &Path) -> Outcome<()> {
        refused(path)
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
    fn file_names(&self, path: &Path) -> Vec<String> {
        // The files directly under `path`: a held path exactly one level down.
        let mut names: Vec<String> = self
            .existing
            .keys()
            .filter_map(|known| known.strip_prefix(path).ok())
            .filter(|rest| rest.components().count() == 1)
            .map(|rest| rest.to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
    fn read_to_string(&self, path: &Path) -> Option<String> {
        self.existing.get(path).cloned()
    }
    fn write_to_string(&self, path: &Path, _content: &str) -> Outcome<()> {
        refused(path)
    }
}

fn refused(path: &Path) -> Outcome<()> {
    Err(Halt::Halted(format!(
        "the architecture reads, it never writes: {}",
        path.display()
    )))
}
