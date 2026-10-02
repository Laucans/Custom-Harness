//! A file lock: at most one carrier of a given name at a time.
//!
//! Prevents `pr_review` and `refinement` from double-running — two hooks on
//! the same PR, or two refinement rounds on the same issue, would each post
//! twice. A `mkdir`, not a witness file: directory creation is atomic on
//! every filesystem we care about, so two processes starting at once cannot
//! both get it.

use std::path::Path;

use crate::domain::{Halt, Outcome};

/// What a lock demands from the disk.
pub trait Locks {
    /// Attempt to acquire the lock `name` under `dir`. True if this call
    /// acquired it, false if someone else already holds it.
    ///
    /// # Errors
    /// If `dir` could not be created.
    fn acquire(&self, dir: &Path, name: &str) -> Outcome<bool>;

    /// Release the lock. Silent if it no longer exists — releasing it twice
    /// must not be an error.
    fn release(&self, dir: &Path, name: &str);
}

/// A lock that is actually a directory on disk.
pub struct DirLocks;

impl DirLocks {
    fn path(dir: &Path, name: &str) -> std::path::PathBuf {
        dir.join(format!(".lock-{name}"))
    }
}

impl Locks for DirLocks {
    fn acquire(&self, dir: &Path, name: &str) -> Outcome<bool> {
        std::fs::create_dir_all(dir).map_err(|e| {
            Halt::Failed(format!(
                "cannot create {} to place a lock: {e}",
                dir.display()
            ))
        })?;
        match std::fs::create_dir(Self::path(dir, name)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(Halt::Failed(format!(
                "cannot place lock {name} in {}: {e}",
                dir.display()
            ))),
        }
    }

    fn release(&self, dir: &Path, name: &str) {
        let _ = std::fs::remove_dir(Self::path(dir, name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("harness-lock-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_fresh_name_is_acquired() {
        let dir = Dir::new("fresh");
        assert!(DirLocks.acquire(&dir.0, "32").expect("acquired"));
    }

    #[test]
    fn a_name_already_held_is_refused_to_a_second_claimant() {
        // Two hooks on the same PR: the second must not also get the lock.
        let dir = Dir::new("held");
        assert!(DirLocks.acquire(&dir.0, "32").expect("first gets it"));
        assert!(
            !DirLocks
                .acquire(&dir.0, "32")
                .expect("second does not get it")
        );
    }

    #[test]
    fn releasing_frees_the_name_for_the_next_claimant() {
        let dir = Dir::new("released");
        assert!(DirLocks.acquire(&dir.0, "32").expect("acquired"));
        DirLocks.release(&dir.0, "32");
        assert!(DirLocks.acquire(&dir.0, "32").expect("re-acquired"));
    }

    #[test]
    fn releasing_twice_is_not_an_error() {
        let dir = Dir::new("double-release");
        assert!(DirLocks.acquire(&dir.0, "32").expect("acquired"));
        DirLocks.release(&dir.0, "32");
        DirLocks.release(&dir.0, "32");
    }

    #[test]
    fn two_different_names_do_not_contend() {
        let dir = Dir::new("two-names");
        assert!(DirLocks.acquire(&dir.0, "32").expect("first"));
        assert!(DirLocks.acquire(&dir.0, "99").expect("second, unrelated"));
    }
}
