//! The real disk, behind the [`Disk`] port.
//!
//! The port and the reason it exists live in
//! [`ports::shell::disk`](harness_core::ports::shell::disk); this file is only the
//! implementation that calls `std::fs`, plus the proof that "already gone is
//! not a failure" holds.

use std::path::Path;

use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::disk::Disk;

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

    fn remove_file(&self, path: &Path) -> Outcome<()> {
        match std::fs::remove_file(path) {
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

    fn file_names(&self, path: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(path) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.path().is_file())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn size_bytes(&self, path: &Path) -> u64 {
        size_of(path)
    }

    fn modified_secs(&self, path: &Path) -> Option<u64> {
        let modified = std::fs::symlink_metadata(path).ok()?.modified().ok()?;
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|since| since.as_secs())
    }

    fn read_to_string(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn write_to_string(&self, path: &Path, content: &str) -> Outcome<()> {
        std::fs::write(path, content)
            .map_err(|e| Halt::Failed(format!("couldn't write {}: {e}", path.display())))
    }
}

/// The blocks `path` takes, its whole tree for a folder, links never
/// followed — what `du` counts. What cannot be read counts nothing.
fn size_of(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    // `st_blocks` counts 512-byte units whatever the volume's block size.
    let own = meta.blocks().saturating_mul(512);
    if !meta.is_dir() {
        return own;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return own;
    };
    entries.flatten().fold(own, |total, entry| {
        total.saturating_add(size_of(&entry.path()))
    })
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
    fn a_file_is_removed_and_one_already_gone_is_not_a_failure() {
        let path = std::env::temp_dir().join(format!(
            "harness-remove-file-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::write(&path, "x").expect("written");
        assert!(RealDisk.remove_file(&path).is_ok());
        assert!(!path.exists(), "removed");
        assert!(RealDisk.remove_file(&path).is_ok(), "already gone");
        let folder = std::env::temp_dir();
        assert!(
            RealDisk.remove_file(&folder).is_err(),
            "a folder is no file"
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
        assert_eq!(
            RealDisk.dir_names(Path::new("/tmp/not-a-directory-at-all")),
            [] as [String; 0]
        );
    }

    #[test]
    fn a_folder_measures_its_whole_tree_without_following_links() {
        let root = std::env::temp_dir().join(format!(
            "harness-size-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let deep = root.join("debug/deps");
        std::fs::create_dir_all(&deep).expect("mkdir");
        std::fs::write(deep.join("big"), vec![7_u8; 256 * 1024]).expect("big");
        std::fs::write(root.join("small"), "x").expect("small");
        // A link to a folder far bigger than the tree: never counted.
        std::os::unix::fs::symlink("/usr", root.join("usr")).expect("link");
        let size = RealDisk.size_bytes(&root);
        assert!(
            (256 * 1024..2 * 1024 * 1024).contains(&size),
            "the tree's blocks, the link's target left out: {size}"
        );
        assert!(RealDisk.size_bytes(&deep.join("big")) >= 256 * 1024);
        assert_eq!(RealDisk.size_bytes(&root.join("absent")), 0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let modified = RealDisk.modified_secs(&root.join("small")).expect("a time");
        assert!(modified.abs_diff(now) < 600, "{modified} vs {now}");
        assert_eq!(RealDisk.modified_secs(&root.join("absent")), None);
        std::fs::remove_dir_all(&root).expect("cleaned");
    }
}
