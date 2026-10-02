//! Where log lines are written: the console and the run log file.
//!
//! Both, always. **The file keeps everything** regardless of the level
//! requested — it's what we re-read after the fact, and a `--quiet` that had
//! truncated the only trace of a night run would be a false economy. The
//! level only concerns the console, and `harness_core::traces::Logbook`
//! already handles it.

use std::cell::RefCell;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use harness_core::traces::Sink;

/// The console and the run log file.
pub struct Both {
    path: PathBuf,
    file: RefCell<Option<std::fs::File>>,
}

impl Both {
    /// The log for this run, written here in addition to the console.
    ///
    /// The file is opened now rather than at the first line: a log directory
    /// that cannot be created is worth knowing before a session is charged, not
    /// after.
    ///
    /// # Errors
    ///
    /// If the directory or file could not be created.
    pub fn new(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            file: RefCell::new(Some(file)),
        })
    }

    /// Where this log is written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Sink for Both {
    fn emit(&self, line: &str) {
        println!("{line}");
        // A file that no longer writes should not kill a run in flight, and
        // should not complain on each line either: we release it once,
        // saying so once.
        let mut held = self.file.borrow_mut();
        if let Some(file) = held.as_mut()
            && writeln!(file, "{line}").is_err()
        {
            *held = None;
            eprintln!("warning: log file {} no longer writes", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("harness-sink-{tag}"));
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
    fn the_log_directory_is_created_rather_than_demanded() {
        let dir = Dir::new("creates");
        let path = dir.0.join("20261002/run.log");
        let sink = Both::new(&path).expect("a log");
        sink.emit("a line");
        assert_eq!(std::fs::read_to_string(&path).expect("re-read"), "a line\n");
    }

    #[test]
    fn a_second_run_appends_rather_than_truncating() {
        let dir = Dir::new("appends");
        let path = dir.0.join("run.log");
        Both::new(&path).expect("a log").emit("first");
        Both::new(&path).expect("a log").emit("second");
        let text = std::fs::read_to_string(&path).expect("re-read");
        assert_eq!(text, "first\nsecond\n");
    }
}
