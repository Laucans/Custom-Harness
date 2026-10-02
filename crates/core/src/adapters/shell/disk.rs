//! The disk, wrapped — so that deletion is testable.
//!
//! A port for five filesystem operations might seem like too much:
//! `std::fs` is already a library. The reason is elsewhere. The only module
//! that uses it is [`crate::execution::provisioning`], i.e. the only one
//! in the crate that **deletes hundreds of megabytes**, and its three rules
//! — nothing is overwritten silently, nothing is deleted silently, a
//! dry-run doesn't clone — are precisely those that must be testable
//! without putting a real folder at risk.
//!
//! A fake disk makes "this test proves we don't delete" verifiable.
//! Without it, proving it would require creating a clone and hoping.

use std::path::Path;

use crate::domain::{Halt, Outcome};

/// What workspace setup asks from the disk.
pub trait Disk {
    /// Does this path exist?
    fn exists(&self, path: &Path) -> bool;

    /// Create this folder and its parents. Already existing is not an error.
    ///
    /// # Errors
    /// If the folder couldn't be created.
    fn create_dir_all(&self, path: &Path) -> Outcome<()>;

    /// Remove this folder and everything in it.
    ///
    /// # Errors
    /// If removal failed. The folder being absent is not an error: the desired
    /// end state is reached.
    fn remove_dir_all(&self, path: &Path) -> Outcome<()>;

    /// Names of subdirectories, sorted. Empty if the path isn't a directory —
    /// this is used to list workspaces kept in a message.
    fn dir_names(&self, path: &Path) -> Vec<String>;

    /// The content of a text file, or `None` for any reason.
    ///
    /// `None`, never an error: refinement reads `CLAUDE.md` and repository
    /// docs to build its map, and a missing or unreadable file is data —
    /// "nothing to read there" — never a failure that stops the round.
    fn read_to_string(&self, path: &Path) -> Option<String>;

    /// Write this text to a file, replacing its content wholesale.
    ///
    /// # Errors
    /// If the write failed.
    fn write_to_string(&self, path: &Path, content: &str) -> Outcome<()>;
}

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
