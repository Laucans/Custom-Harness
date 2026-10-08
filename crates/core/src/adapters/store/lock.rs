//! A file lock, behind the [`Locks`] port.
//!
//! A `mkdir`, not a witness file: directory creation is atomic on every
//! filesystem we care about, so two processes starting at once cannot both
//! get it.
//!
//! **And an owner inside it, held with an OS file lock.** A directory alone
//! outlives the process that made it: a run killed mid-flight left its lock
//! behind, and the issue stayed taken forever. The holder now keeps an
//! exclusive `flock` on `<lock>/owner` for as long as it holds the lock; the
//! OS drops that lock the moment the process dies, however it dies. A lock
//! directory whose owner can be locked again is a dead holder's, and is taken
//! over.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::domain::{Halt, Outcome};
use crate::ports::store::lock::Locks;

/// A lock that is actually a directory on disk.
pub struct DirLocks;

/// The owner files this process holds locked, by lock directory: dropping
/// one releases its OS lock.
fn held() -> &'static Mutex<HashMap<PathBuf, File>> {
    static HELD: OnceLock<Mutex<HashMap<PathBuf, File>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(HashMap::new()))
}

impl DirLocks {
    fn path(dir: &Path, name: &str) -> PathBuf {
        dir.join(format!(".lock-{name}"))
    }

    /// Takes the OS lock on `lock`'s owner file. `Ok(false)`: a live process
    /// holds it.
    fn own(lock: &Path) -> std::io::Result<bool> {
        let owner = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock.join("owner"))?;
        match owner.try_lock() {
            Ok(()) => {
                held()
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(lock.to_path_buf(), owner);
                Ok(true)
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(false),
            Err(std::fs::TryLockError::Error(e)) => Err(e),
        }
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
        let lock = Self::path(dir, name);
        let failed = |e: std::io::Error| {
            Halt::Failed(format!(
                "cannot place lock {name} in {}: {e}",
                dir.display()
            ))
        };
        match std::fs::create_dir(&lock) {
            Ok(()) => Self::own(&lock).map_err(failed),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Held by a live process, or left behind by a dead one. A
                // directory with no owner file yet is a holder between its
                // `mkdir` and its `own` — taken, not stale.
                if lock.join("owner").exists() {
                    Self::own(&lock).map_err(failed)
                } else {
                    Ok(false)
                }
            }
            Err(e) => Err(failed(e)),
        }
    }

    fn release(&self, dir: &Path, name: &str) {
        let lock = Self::path(dir, name);
        let owned = held()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&lock);
        let _ = std::fs::remove_file(lock.join("owner"));
        let _ = std::fs::remove_dir(&lock);
        drop(owned);
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

    #[test]
    fn a_lock_its_holder_died_with_is_taken_over() {
        let dir = Dir::new("dead-holder");
        // What a killed run leaves: the directory and its owner file, with
        // no process holding the OS lock any more.
        let lock = dir.0.join(".lock-32");
        std::fs::create_dir_all(&lock).expect("lock dir");
        std::fs::write(lock.join("owner"), "").expect("owner");
        assert!(DirLocks.acquire(&dir.0, "32").expect("taken over"));
        assert!(!DirLocks.acquire(&dir.0, "32").expect("now held"));
        DirLocks.release(&dir.0, "32");
        assert!(!lock.exists());
    }
}
