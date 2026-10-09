//! Telling the event store, from the launcher's side: which store a log
//! belongs to, who is telling, and the clock.
//!
//! The store is never a reason to fail a run: an event that cannot be kept
//! is said once on stderr, and the run goes on — its journal line was written
//! regardless.

use std::cell::Cell;
use std::path::Path;

use harness_core::adapters::store::events::SqliteEvents;
use harness_core::domain::workspace::{LOGS, Workspace};
use harness_core::ports::store::events::EventLog;
use harness_core::traces::Event;

use crate::adapters::spending;

/// A store, and the name events are told under.
pub struct Teller {
    store: SqliteEvents,
    source: String,
    failed: Cell<bool>,
}

impl Teller {
    /// The store of the checkout that holds `log`, told under the run the log
    /// belongs to — `watch` for the watch's journal, `<workflow>/<run>` for a
    /// run's `run.log`. `None` for a log outside a checkout's `.llocal/logs`.
    #[must_use]
    pub fn for_log(log: &Path) -> Option<Self> {
        let logs = log.ancestors().find(|dir| dir.ends_with(LOGS))?;
        let root = logs.ancestors().nth(2)?;
        let within = log.strip_prefix(logs).ok()?;
        let source = if within.file_name().is_some_and(|name| name == "watch.log") {
            "watch".to_string()
        } else {
            within
                .parent()
                .map(|run| run.to_string_lossy().into_owned())
                .filter(|run| !run.is_empty())?
        };
        Self::open(root, &source)
    }

    /// The store of the checkout at `root`, told under `source`.
    #[must_use]
    pub fn open(root: &Path, source: &str) -> Option<Self> {
        match SqliteEvents::open(&Workspace::new(root).events()) {
            Ok(store) => Some(Self {
                store,
                source: source.to_string(),
                failed: Cell::new(false),
            }),
            Err(why) => {
                eprintln!("warning: no event store — {}", why.reason());
                None
            }
        }
    }

    /// Keeps `event`, stamped now.
    pub fn tell(&self, event: &Event) {
        if let Err(why) = self.store.append(&spending::now(), &self.source, event)
            && !self.failed.replace(true)
        {
            eprintln!("warning: events are no longer kept — {}", why.reason());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_tells_under_its_run_and_the_journal_under_watch() {
        let root = std::env::temp_dir().join(format!("harness-teller-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let run = root.join(".llocal/logs/agent-loop/20261009-004034-15/run.log");
        let teller = Teller::for_log(&run).expect("a run log");
        assert_eq!(teller.source, "agent-loop/20261009-004034-15");
        let watch =
            Teller::for_log(&root.join(".llocal/logs/agent-loop/watch.log")).expect("journal");
        assert_eq!(watch.source, "watch");
        teller.tell(&Event::WatchStopped);
        let kept = SqliteEvents::open(&Workspace::new(&root).events())
            .expect("store")
            .after(0, 10)
            .expect("read");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].source, "agent-loop/20261009-004034-15");
        assert!(Teller::for_log(Path::new("/tmp/elsewhere/run.log")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
