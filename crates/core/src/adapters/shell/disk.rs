//! The real disk, behind the [`Disk`] port.
//!
//! The port and the reason it exists live in
//! [`ports::shell::disk`](crate::ports::shell::disk); this file is only the
//! implementation that calls `std::fs`, plus the proof that "already gone is
//! not a failure" holds.

use std::path::Path;

use crate::domain::{Halt, Outcome};
use crate::ports::shell::disk::Disk;

/** The real disk. */
pub struct RealDisk;

impl Disk for RealDisk {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir_all(&self, path: &Path) -> Outcome<()> {
        std::fs::create_dir_all(path)
            .map_err(|e| Halt::Failed(format!("couldn't create {}: {e}", path.display())))
    }

    fn remove_dir_all(&self, path: &Path) -> Outcome<()> {
        match std::fs::remove_dir_all(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Halt::Failed(format!(
                "couldn't delete {}: {e}",
                path.display()
            ))),
        }
    }

    fn dir_names(&self, path: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(path) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn read_to_string(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn write_to_string(&self, path: &Path, content: &str) -> Outcome<()> {
        std::fs::write(path, content)
            .map_err(|e| Halt::Failed(format!("couldn't write {}: {e}", path.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_something_that_is_already_gone_is_not_a_failure() {
        // The desired end state is reached, and teardown must not requalify
        // a successful run as a failure for this.
        assert!(
            RealDisk
                .remove_dir_all(Path::new("/tmp/nonexistent-directory"))
                .is_ok()
        );
    }

    #[test]
    fn a_missing_file_reads_as_none_not_as_a_failure() {
        // Refinement reads CLAUDE.md and docs for its map: a missing file
        // is data — "nothing to read there" — never a failure.
        assert!(
            RealDisk
                .read_to_string(Path::new("/tmp/nonexistent-file.md"))
                .is_none()
        );
    }

    #[test]
    fn listing_something_that_is_not_a_directory_gives_no_names() {
        assert!(
            RealDisk
                .dir_names(Path::new("/tmp/pas-un-dossier-du-tout"))
                .is_empty()
        );
    }
}
