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

use harness_core::traces::{Event, Sink};

use crate::adapters::events::Teller;

/// The console alone.
///
/// For a command that mounts no run directory, and therefore has no run log to
/// keep: `doctor` reads a ledger and runs a `git`, so there is nothing to
/// re-read after the fact that is not already in the ledger.
pub struct Console;

impl Sink for Console {
    fn emit(&self, line: &str) {
        println!("{line}");
    }

    /// Nowhere to keep it: this sink *is* the console.
    fn keep(&self, _line: &str) {}
}

/// The clock reading a file line carries: the same UTC shape as the ledgers.
///
/// UTC, and saying so with the `Z`. Local time would read better, but a
/// `watch` is launched from a shell whose time zone the harness cannot count on
/// — the first try printed UTC *without* the `Z` and looked like a clock four
/// hours wrong. Sharing the ledgers' exact shape has its own use: a line here
/// and a row in `costs.tsv` or `errors.tsv` can be lined up by string.
fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// A line as the file keeps it.
fn stamped(at: &str, line: &str) -> String {
    format!("[{at}] {line}")
}

/// The console and the run log file.
pub struct Both {
    path: PathBuf,
    file: RefCell<Option<std::fs::File>>,
    /// `alive.lock` beside the log, held with an OS lock for as long as this
    /// run lives — the OS releases it when the process dies, which is how the
    /// view tells a run at work from one that stopped. `None` when the lock
    /// could not be taken: the run goes on, the view falls back to the age of
    /// its files.
    _alive: Option<std::fs::File>,
    /// The checkout's event store, told under this log's run — `None` for a
    /// log outside a checkout, or a store that could not open.
    events: Option<Teller>,
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
        let alive = path
            .parent()
            .and_then(|dir| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(dir.join(harness_core::traces::RUN_ALIVE))
                    .ok()
            })
            .filter(|lock| lock.try_lock().is_ok());
        Ok(Self {
            path: path.to_path_buf(),
            file: RefCell::new(Some(file)),
            _alive: alive,
            events: Teller::for_log(path),
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
        self.keep(line);
    }

    fn keep(&self, line: &str) {
        // Stamped in the file only. The console is read as it happens, where a
        // clock on every line is noise; the file is read hours later, where a
        // line without one says nothing — a `watch` journal is hundreds of
        // identical ticks whose only distinguishing mark is when they happened.
        //
        // A file that no longer writes should not kill a run in flight, and
        // should not complain on each line either: we release it once,
        // saying so once.
        let mut held = self.file.borrow_mut();
        if let Some(file) = held.as_mut()
            && writeln!(file, "{}", stamped(&now(), line)).is_err()
        {
            *held = None;
            eprintln!("warning: log file {} no longer writes", self.path.display());
        }
    }

    fn record(&self, event: &Event) {
        if let Some(events) = &self.events {
            events.tell(event);
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
        let text = std::fs::read_to_string(&path).expect("re-read");
        assert!(text.ends_with(" a line\n"), "got {text:?}");
    }

    #[test]
    fn a_second_run_appends_rather_than_truncating() {
        let dir = Dir::new("appends");
        let path = dir.0.join("run.log");
        Both::new(&path).expect("a log").emit("first");
        Both::new(&path).expect("a log").emit("second");
        let text = std::fs::read_to_string(&path).expect("re-read");
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().next().expect("a line").ends_with(" first"));
    }

    #[test]
    fn a_file_line_carries_a_clock_the_console_line_does_not() {
        // The failure this answers: a watch journal of four hundred identical
        // `tick: Nothing` lines, with nothing saying when any of them ran.
        assert_eq!(
            stamped("2026-10-06T14:03:11Z", "tick: Nothing"),
            "[2026-10-06T14:03:11Z] tick: Nothing"
        );
    }

    #[test]
    fn a_line_the_console_hides_is_still_written_to_the_file() {
        // `Logbook::debug` lands here: the snapshot a tick decided from is
        // noise live and the whole autopsy afterwards.
        let dir = Dir::new("keeps");
        let path = dir.0.join("watch.log");
        Both::new(&path).expect("a log").keep("saw: 3 milestones");
        let text = std::fs::read_to_string(&path).expect("re-read");
        assert!(text.ends_with(" saw: 3 milestones\n"), "got {text:?}");
    }

    #[test]
    fn the_clock_reads_to_the_second_and_names_its_zone() {
        // The `Z` is the point: a bare reading was taken for local time.
        let at = now();
        assert_eq!(at.len(), "2026-10-06T14:03:11Z".len(), "got {at:?}");
        assert!(at.starts_with("20") && at.ends_with('Z'), "got {at:?}");
    }
}
