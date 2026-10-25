//! The disk floor: under `DISK_FLOOR_GB` free on the volume the lanes build
//! on, the watch starts no lane.
//!
//! A cold product build takes 3.4 GB of `target`; five lanes starting one
//! each after a milestone split took a 460 GB disk to 1 GB free
//! (2026-10-24 22:28). A build that cannot write fails in ways the doctor
//! cannot treat, so the watch waits for room instead. The free work — the
//! merges, run in the tick — goes on: it builds nothing new.
//!
//! The crossing is said once, each way ([`Event::DiskUnderFloor`],
//! [`Event::DiskAboveFloor`]); notify raises the first and clears it on the
//! second, or on the next `WatchStarted`: a new watch starts unheld and says
//! the floor again on its first tick if the disk is still short. The volume is measured every tick — one `statvfs`, no read of
//! the board — and the tick that finds room again reads the board, since
//! the jobs held back did not move it.

use std::path::PathBuf;
use std::rc::Rc;

use harness_core::ports::shell::volume::Volume;
use harness_core::traces::{Event, Logbook};

/// One gigabyte, as `df -g` counts it.
pub const GIB: u64 = 1 << 30;

/// What a tick may start, as far as the disk is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Room {
    /// Above the floor, as at the last tick — or no floor at all.
    Enough,
    /// Back above the floor since the last tick: a reason to read the board.
    Back,
    /// Under the floor: no lane starts.
    Short,
}

/// The floor and whether the last tick found the volume under it.
pub struct Floor {
    /// `DISK_FLOOR_GB`; `0` is no floor.
    gigabytes: u64,
    /// The folder whose volume is measured: the lanes' workspaces.
    at: PathBuf,
    /// The volume, asked through its port.
    volume: Rc<dyn Volume>,
    /// Whether the last measure was under the floor.
    held: bool,
}

impl Floor {
    /// A floor of `floor_gb` on the volume holding `at`.
    #[must_use]
    pub fn new(floor_gb: u64, at: PathBuf, volume: Rc<dyn Volume>) -> Self {
        Self {
            gigabytes: floor_gb,
            at,
            volume,
            held: false,
        }
    }

    /// Measures the volume and says a crossing. A volume that cannot be
    /// asked changes nothing: unheld, it holds nothing back — a floor that
    /// blocked on an unreadable `statvfs` would stop the factory on a
    /// question about the disk — and held, it stays held, since only a
    /// measure above the floor says the room notify clears on.
    ///
    /// Under the floor, `reclaim` runs once — the free lanes' build folders
    /// pruned — and the volume is measured again: the crossing said is the
    /// one after it, so space the watch owns never holds it.
    pub fn check(&mut self, log: &Logbook, reclaim: impl FnOnce()) -> Room {
        if self.gigabytes == 0 {
            return Room::Enough;
        }
        let Some(mut free_gb) = self.measure(log) else {
            return if self.held { Room::Short } else { Room::Enough };
        };
        if free_gb < self.gigabytes {
            reclaim();
            free_gb = self.measure(log).unwrap_or(free_gb);
        }
        if free_gb >= self.gigabytes {
            return self.released(free_gb, log);
        }
        let under = Event::DiskUnderFloor {
            free_gb,
            floor_gb: self.gigabytes,
        };
        if self.held {
            // Said once, when it fell: the journal's tick lines carry on.
            log.debug(&under.line().unwrap_or_default());
        } else {
            log.event(&under);
            self.held = true;
        }
        Room::Short
    }

    /// Whole gigabytes free on the volume, `None` when it cannot be asked.
    fn measure(&self, log: &Logbook) -> Option<u64> {
        match self.volume.free_bytes(&self.at) {
            Ok(free) => Some(free / GIB),
            Err(halt) => {
                log.debug(&format!("disk: {}", halt.reason()));
                None
            }
        }
    }

    /// Above the floor: [`Room::Back`], said with the room found, when the
    /// last tick held.
    fn released(&mut self, free_gb: u64, log: &Logbook) -> Room {
        if !self.held {
            return Room::Enough;
        }
        self.held = false;
        log.event(&Event::DiskAboveFloor {
            free_gb,
            floor_gb: self.gigabytes,
        });
        Room::Back
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::Path;

    use harness_core::domain::{Halt, Outcome};
    use harness_core::traces::{Sink, Verbosity};

    use super::*;

    /// A volume whose free space the test sets, and that notes where it was
    /// asked.
    #[derive(Default)]
    struct Gauge {
        free: Cell<Option<u64>>,
        asked: RefCell<Vec<PathBuf>>,
    }

    impl Volume for Gauge {
        fn free_bytes(&self, path: &Path) -> Outcome<u64> {
            self.asked.borrow_mut().push(path.to_path_buf());
            self.free
                .get()
                .ok_or_else(|| Halt::Failed("statvfs refused".to_string()))
        }
    }

    #[derive(Default)]
    struct Lines(RefCell<Vec<String>>);

    impl Sink for Lines {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
        // The debug lines a run's log keeps: not what the journal says.
        fn keep(&self, _line: &str) {}
    }

    fn said(lines: &Lines) -> Vec<String> {
        lines.0.borrow_mut().drain(..).collect()
    }

    #[test]
    fn the_floor_holds_the_lanes_and_says_each_crossing_once() {
        let gauge = Rc::new(Gauge::default());
        let lines = Rc::new(Lines::default());
        let log = Logbook::new(Rc::clone(&lines) as Rc<dyn Sink>, Verbosity::Normal);
        let mut floor = Floor::new(
            12,
            PathBuf::from("/factories/harness-workspaces"),
            Rc::clone(&gauge) as Rc<dyn Volume>,
        );
        gauge.free.set(Some(30 * GIB));
        assert_eq!(floor.check(&log, || {}), Room::Enough);
        assert!(said(&lines).is_empty(), "room enough is not news");
        // One byte short of 12 GB is 11 whole ones: under.
        gauge.free.set(Some(12 * GIB - 1));
        assert_eq!(floor.check(&log, || {}), Room::Short);
        let first = said(&lines);
        assert_eq!(first.len(), 1, "{first:?}");
        assert!(
            first[0].contains("idle: disk under the floor (11 GB < 12 GB)"),
            "{first:?}"
        );
        gauge.free.set(Some(GIB));
        assert_eq!(floor.check(&log, || {}), Room::Short);
        assert!(said(&lines).is_empty(), "said once, when it fell");
        // Exactly the floor is enough.
        gauge.free.set(Some(12 * GIB));
        assert_eq!(floor.check(&log, || {}), Room::Back);
        let back = said(&lines);
        assert_eq!(back.len(), 1, "{back:?}");
        assert!(
            back[0].contains("disk: back above the floor (12 GB >= 12 GB)"),
            "{back:?}"
        );
        assert_eq!(floor.check(&log, || {}), Room::Enough, "back is said once");
        assert_eq!(
            gauge.asked.borrow().first().map(PathBuf::as_path),
            Some(Path::new("/factories/harness-workspaces"))
        );
    }

    #[test]
    fn a_free_lane_pruned_under_the_floor_lifts_it_in_the_same_tick() {
        let gauge = Rc::new(Gauge::default());
        let lines = Rc::new(Lines::default());
        let log = Logbook::new(Rc::clone(&lines) as Rc<dyn Sink>, Verbosity::Normal);
        let mut floor = Floor::new(12, PathBuf::from("/f"), Rc::clone(&gauge) as Rc<dyn Volume>);
        // Above the floor nothing is reclaimed.
        gauge.free.set(Some(30 * GIB));
        let mut asked = false;
        assert_eq!(floor.check(&log, || asked = true), Room::Enough);
        assert!(!asked, "room enough prunes nothing");
        // Under it, a pruned lane frees 9 GB: never held, nothing said.
        gauge.free.set(Some(5 * GIB));
        let freed = || gauge.free.set(Some(14 * GIB));
        assert_eq!(floor.check(&log, freed), Room::Enough);
        assert!(said(&lines).is_empty(), "{:?}", lines.0.borrow());
        // Not enough freed: held, said once with what is left.
        gauge.free.set(Some(3 * GIB));
        let some = || gauge.free.set(Some(8 * GIB));
        assert_eq!(floor.check(&log, some), Room::Short);
        let held = said(&lines);
        assert!(held[0].contains("(8 GB < 12 GB)"), "{held:?}");
        // Held, then a pruned lane lifts it: back, in the same tick.
        gauge.free.set(Some(3 * GIB));
        let lifted = || gauge.free.set(Some(20 * GIB));
        assert_eq!(floor.check(&log, lifted), Room::Back);
        assert!(said(&lines)[0].contains("(20 GB >= 12 GB)"));
    }

    #[test]
    fn exactly_the_floor_free_prunes_nothing() {
        let gauge = Rc::new(Gauge::default());
        let log = Logbook::null();
        let mut floor = Floor::new(12, PathBuf::from("/f"), Rc::clone(&gauge) as Rc<dyn Volume>);
        gauge.free.set(Some(12 * GIB));
        let mut asked = false;
        assert_eq!(floor.check(&log, || asked = true), Room::Enough);
        assert!(!asked, "the floor itself is room enough: nothing pruned");
        assert_eq!(gauge.asked.borrow().len(), 1, "measured once");
    }

    #[test]
    fn no_floor_asks_nothing_and_an_unreadable_volume_changes_nothing() {
        let gauge = Rc::new(Gauge::default());
        let lines = Rc::new(Lines::default());
        let log = Logbook::new(Rc::clone(&lines) as Rc<dyn Sink>, Verbosity::Normal);
        let mut none = Floor::new(0, PathBuf::from("/f"), Rc::clone(&gauge) as Rc<dyn Volume>);
        gauge.free.set(Some(0));
        assert_eq!(none.check(&log, || {}), Room::Enough);
        assert!(gauge.asked.borrow().is_empty(), "no floor, no measure");
        let mut floor = Floor::new(12, PathBuf::from("/f"), Rc::clone(&gauge) as Rc<dyn Volume>);
        gauge.free.set(None);
        assert_eq!(floor.check(&log, || {}), Room::Enough);
        gauge.free.set(Some(0));
        assert_eq!(floor.check(&log, || {}), Room::Short);
        said(&lines);
        // Held, then unreadable: still held, nothing claimed.
        gauge.free.set(None);
        assert_eq!(floor.check(&log, || {}), Room::Short);
        assert!(said(&lines).is_empty(), "no room is claimed unmeasured");
        // The next real measure above the floor releases it, and says so.
        gauge.free.set(Some(20 * GIB));
        assert_eq!(floor.check(&log, || {}), Room::Back);
        assert_eq!(said(&lines).len(), 1, "back is said");
        assert_eq!(floor.check(&log, || {}), Room::Enough);
    }
}
