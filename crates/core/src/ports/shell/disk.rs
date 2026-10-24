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

    /// Remove this file.
    ///
    /// Refused by default: a disk that does not say how it removes a file
    /// must not pretend it did — the caller would leave it behind believing
    /// it gone.
    ///
    /// # Errors
    /// If removal failed. The file being absent is not an error: the desired
    /// end state is reached.
    fn remove_file(&self, path: &Path) -> Outcome<()> {
        Err(Halt::Failed(format!(
            "this disk cannot remove {}",
            path.display()
        )))
    }

    /// Names of subdirectories, sorted. Empty if the path isn't a directory —
    /// this is used to list workspaces kept in a message.
    fn dir_names(&self, path: &Path) -> Vec<String>;

    /// Names of the files directly under `path`, sorted. Empty if the path
    /// isn't a directory. The architecture's rules read the frozen screens
    /// and the mocked pages with it; a disk with no files to list says so.
    fn file_names(&self, _path: &Path) -> Vec<String> {
        Vec::new()
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A disk that says nothing of how it removes a file.
    struct Silent;

    impl Disk for Silent {
        fn exists(&self, _path: &Path) -> bool {
            false
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
        fn read_to_string(&self, _path: &Path) -> Option<String> {
            None
        }
        fn write_to_string(&self, _path: &Path, _content: &str) -> Outcome<()> {
            Ok(())
        }
    }

    #[test]
    fn a_disk_that_does_not_say_how_it_removes_a_file_refuses_to() {
        let refused = Silent.remove_file(Path::new("/ws/a.txt"));
        assert!(
            matches!(&refused, Err(Halt::Failed(why)) if why.contains("/ws/a.txt")),
            "{refused:?}"
        );
        assert_eq!(Silent.file_names(Path::new("/ws")).len(), 0);
    }
}
