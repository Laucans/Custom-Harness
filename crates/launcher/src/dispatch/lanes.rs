//! The lanes of a parallel `watch`: one child process per issue, up to
//! `--parallel` at once, each in its own workspace.
//!
//! The harness keeps driving **one session per process** (decision #3,
//! `?Send` throughout): parallelism is processes, not threads. The watch
//! spawns `harness --task <n> --use-workspace lane-<k>` for every runnable
//! task the board offers that no lane is on, then, in the lanes left,
//! `harness split <n>` / `harness refine <n>` with `--use-workspace
//! router-lane-<k>` for every milestone ready to split and every issue
//! waiting on its business refinement — whatever the tick's route — reaps
//! the children on each tick, and frees their lane. What
//! makes two tasks safe to run at once is the architecture's read side —
//! `split` chains the write side, so at most one of those is ever runnable;
//! two splits cut two different milestones, two refinements write two
//! different issue bodies, and nothing else is shared.
//!
//! A lane's output goes to `.llocal/lanes/lane-<k>.log`; the run itself
//! writes its own `run.log` under `.llocal/logs/`, which is what the view
//! reads.
//!
//! **A lane that stops (exit 1) parks its task.** Exit 1 is a run that halted
//! for a human — a question in its SPEC, or the breaker refusing to pay for a
//! prompt that already failed twice. Taking the task again next tick buys the
//! same stop, every tick, forever. So the task waits, parked, until its issue
//! changes (body or labels): the gesture that answers the stop is the one that
//! frees it. Or until [`PARKED_FOR`] has passed: a stop can come from outside
//! the issue (its milestone closed for a while, `ForgeDesMondes` 2026-10-22),
//! and a park that only the issue could lift held the factory idle for a
//! day while the cause was long gone. A stale retry costs one preflight.
//!
//! **A lane's build folder is pruned between jobs** ([`prune_target`]): a
//! lane builds the product in its own checkout, and its `target` grows with
//! every job — 8.2 GB measured with incremental on, five lanes ~40 GB. Over
//! `LANE_TARGET_CAP_GB` the incremental caches and the executables a
//! later build session superseded go; over twice the cap the build profiles
//! do (`debug`, `release`) — the next build is a cold one, 54 s with the
//! shared compilation cache — while what is not cargo's stays: the
//! product's `target/rules-runtime` holds the pids of a server a session
//! may have left running.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use harness_core::ports::shell::disk::Disk;
use harness_core::traces::{Event, Logbook};
use tokio::process::{Child, Command};

/// How long a stopped task stays parked when its issue does not move.
const PARKED_FOR: Duration = Duration::from_hours(1);

/// How much older than the newest executable of its name one must be to be
/// superseded, in seconds: a previous build session's. One session links
/// several current executables of a name — a bin, its test harness, the
/// lib's — within minutes of each other.
const SUPERSEDED_AFTER: u64 = 6 * 3600;

/// The build profiles a target over twice its cap loses: cargo's, and
/// nothing a product's scripts keep beside them.
const PROFILES: [&str; 2] = ["debug", "release"];

/// What [`prune_target`] did to a build folder over its cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pruned {
    /// Its size before, in bytes.
    pub before: u64,
    /// Its size after, in bytes.
    pub after: u64,
    /// What went, in words.
    pub what: String,
}

/// Prunes the build folder `target` when it takes more than `cap` bytes:
/// above twice the cap its build profiles go (`debug`, `release`), what is
/// not cargo's beside them staying; between the two, the debug profile's
/// incremental caches and every executable a later build session
/// superseded. `None` when there was nothing to do — a cap of `0`, a
/// folder within it or absent, nothing found to remove. Nothing here runs
/// while a build does: the lane is free when its next job is about to
/// start.
pub fn prune_target(disk: &dyn Disk, target: &Path, cap: u64) -> Option<Pruned> {
    if cap == 0 {
        return None;
    }
    let before = disk.size_bytes(target);
    if before <= cap {
        return None;
    }
    let mut what = Vec::new();
    if before > cap.saturating_mul(2) {
        let mut gone = Vec::new();
        let mut kept = Vec::new();
        for profile in PROFILES {
            let dir = target.join(profile);
            if !disk.exists(&dir) {
                continue;
            }
            match disk.remove_dir_all(&dir) {
                Ok(()) => gone.push(profile),
                Err(halt) => kept.push(format!("{profile} kept: {}", halt.reason())),
            }
        }
        match gone.as_slice() {
            [] => {}
            [one] => what.push(format!("the {one} profile, above twice the cap")),
            many => what.push(format!(
                "the {} profiles, above twice the cap",
                many.join(" and ")
            )),
        }
        what.extend(kept);
    } else {
        let incremental = target.join("debug").join("incremental");
        if disk.exists(&incremental) {
            match disk.remove_dir_all(&incremental) {
                Ok(()) => what.push("incremental caches".to_string()),
                Err(halt) => what.push(format!("incremental kept: {}", halt.reason())),
            }
        }
        let (removed, kept) = remove_stale(disk, &target.join("debug"));
        if removed > 0 {
            what.push(format!("{removed} stale executable(s)"));
        }
        what.extend(kept);
    }
    // Nothing removed and nothing refused: no news, the next job says it.
    if what.is_empty() {
        return None;
    }
    Some(Pruned {
        before,
        after: disk.size_bytes(target),
        what: what.join(", "),
    })
}

/// Removes from the profile's `deps` every executable a later build session
/// superseded — `<name>-<16 hex>` more than [`SUPERSEDED_AFTER`] older than
/// the newest of its name — with the files cargo wrote beside it
/// (`<name>-<hash>.d`, its objects). How many went, and why any could not.
/// A name whose builds' times cannot all be read keeps them all, and so
/// does a name cargo uplifted into the profile (`debug/<name>`): a bin,
/// whose hard link would keep the space anyway.
fn remove_stale(disk: &dyn Disk, profile: &Path) -> (usize, Vec<String>) {
    let deps = profile.join("deps");
    let files = disk.file_names(&deps);
    let mut builds: HashMap<&str, Vec<(Option<u64>, &str)>> = HashMap::new();
    for file in &files {
        if let Some(name) = executable_name(file) {
            builds
                .entry(name)
                .or_default()
                .push((disk.modified_secs(&deps.join(file)), file.as_str()));
        }
    }
    let mut removed = 0_usize;
    let mut kept = Vec::new();
    let mut names: Vec<_> = builds.into_iter().collect();
    names.sort_unstable();
    for (name, built) in names {
        let times: Option<Vec<u64>> = built.iter().map(|(at, _)| *at).collect();
        let Some(newest) = times.and_then(|times| times.into_iter().max()) else {
            continue;
        };
        if built.len() < 2 || disk.exists(&profile.join(name)) {
            continue;
        }
        let superseded = built
            .iter()
            .filter(|(at, _)| at.is_some_and(|at| newest.saturating_sub(at) > SUPERSEDED_AFTER));
        for (_, stale) in superseded {
            let siblings = files
                .iter()
                .filter(|file| *file == stale || file.starts_with(&format!("{stale}.")));
            let mut whole = true;
            for file in siblings {
                if let Err(halt) = disk.remove_file(&deps.join(file)) {
                    whole = false;
                    kept.push(format!("{file} kept: {}", halt.reason()));
                }
            }
            if whole {
                removed = removed.saturating_add(1);
            }
        }
    }
    (removed, kept)
}

/// The crate a test executable in `deps` was built from: `name` of
/// `name-0123456789abcdef`, an executable having no extension — never a
/// library (`libname-<hash>.rlib`) nor cargo's notes beside it.
fn executable_name(file: &str) -> Option<&str> {
    let (name, hash) = file.rsplit_once('-')?;
    let hashed = hash.len() == 16
        && hash
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    (hashed && !name.is_empty() && !file.contains('.')).then_some(name)
}

/// A lane's process ended, as an event.
fn ended(slot: usize, task: u64, status: std::process::ExitStatus) -> Event {
    Event::LaneEnded {
        lane: u32::try_from(slot).unwrap_or(u32::MAX),
        issue: task,
        status: status.to_string(),
        code: status.code(),
    }
}

/// One running lane.
struct Lane {
    task: u64,
    /// The jobs that may never run side by side share a group
    /// (`routing::Job::exclusive`).
    group: Option<&'static str>,
    child: Child,
}

/// The lanes, by slot.
pub struct Lanes {
    slots: Vec<Option<Lane>>,
    /// The issue version each dispatched task was taken at.
    taken_at: HashMap<u64, String>,
    /// Tasks whose lane stopped, with the issue version it stopped on and
    /// when.
    parked: HashMap<u64, (String, Instant)>,
    /// Set once a soft stop is asked: no slot is free from then on.
    closed: Arc<AtomicBool>,
    /// Lanes that ended on exit 3 (`QUOTA`) since the watch last asked.
    quota_hits: usize,
    /// Lanes that ended on exit 0 since the watch last asked: a session went
    /// through, so the quota is back.
    went: usize,
}

impl Lanes {
    /// `parallel` lanes, all free.
    #[must_use]
    pub fn new(parallel: usize) -> Self {
        Self {
            slots: (0..parallel.max(1)).map(|_| None).collect(),
            taken_at: HashMap::new(),
            parked: HashMap::new(),
            closed: Arc::new(AtomicBool::new(false)),
            quota_hits: 0,
            went: 0,
        }
    }

    /// Ties the lanes to `flag`: once it is set, no slot is free, so no
    /// new task starts while the running ones finish.
    #[must_use]
    pub fn closed_by(mut self, flag: Arc<AtomicBool>) -> Self {
        self.closed = flag;
        self
    }

    /// Whether a soft stop closed the lanes.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Frees the lanes whose child has exited, saying how it went, and
    /// returns how many it freed — a freed lane is a reason to read the
    /// board again even when nothing on it moved.
    pub fn reap(&mut self, log: &Logbook) -> usize {
        let mut freed = 0;
        for (slot, lane) in self.slots.iter_mut().enumerate() {
            let done = match lane {
                Some(running) => match running.child.try_wait() {
                    Ok(Some(status)) => {
                        log.event(&ended(slot, running.task, status));
                        let version = self.taken_at.remove(&running.task);
                        if status.code() == Some(1)
                            && let Some(version) = version
                        {
                            log.event(&Event::TaskParked { task: running.task });
                            self.parked.insert(running.task, (version, Instant::now()));
                        }
                        if status.code() == Some(3) {
                            self.quota_hits = self.quota_hits.saturating_add(1);
                        }
                        if status.code() == Some(0) {
                            self.went = self.went.saturating_add(1);
                        }
                        true
                    }
                    Ok(None) => false,
                    Err(e) => {
                        log.warn(&format!(
                            "watch: lanes -> lane {slot} (#{}) cannot be waited on: {e}",
                            running.task
                        ));
                        true
                    }
                },
                None => false,
            };
            if done {
                *lane = None;
                freed += 1;
            }
        }
        freed
    }

    /// Whether a lane ended on `QUOTA` (exit 3) since this was last asked.
    /// Asking takes the answer: the watch rests once per stop.
    pub const fn struck_by_quota(&mut self) -> bool {
        let struck = self.quota_hits > 0;
        self.quota_hits = 0;
        struck
    }

    /// Whether a lane ended on exit 0 since this was last asked — what clears
    /// a quota rest, the paid work running on lanes only. Asking takes the
    /// answer.
    pub const fn went_through(&mut self) -> bool {
        let went = self.went > 0;
        self.went = 0;
        went
    }

    /// The exclusive groups the lanes hold right now.
    #[must_use]
    pub fn groups(&self) -> Vec<&'static str> {
        self.slots
            .iter()
            .filter_map(|lane| lane.as_ref().and_then(|running| running.group))
            .collect()
    }

    /// Notes the issue version `task` is taken at, so a stop can park it on
    /// that version.
    pub fn taken(&mut self, task: u64, version: String) {
        self.taken_at.insert(task, version);
    }

    /// Whether `task` is parked on this very `version`. A task whose issue
    /// moved since its stop, or parked for [`PARKED_FOR`], is unparked here,
    /// and runs again.
    pub fn is_parked(&mut self, task: u64, version: &str) -> bool {
        self.is_parked_at(task, version, Instant::now())
    }

    /// [`Self::is_parked`], at `now`.
    fn is_parked_at(&mut self, task: u64, version: &str, now: Instant) -> bool {
        match self.parked.get(&task) {
            Some((stopped_on, at))
                if stopped_on == version && now.saturating_duration_since(*at) < PARKED_FOR =>
            {
                true
            }
            Some(_) => {
                self.parked.remove(&task);
                false
            }
            None => false,
        }
    }

    /// The tasks lanes are on right now.
    #[must_use]
    pub fn running(&self) -> Vec<u64> {
        self.slots
            .iter()
            .filter_map(|lane| lane.as_ref().map(|running| running.task))
            .collect()
    }

    /// How many slots are free — none once the lanes are closed.
    #[must_use]
    pub fn free_slots(&self) -> usize {
        if self.is_closed() {
            return 0;
        }
        self.slots.iter().filter(|lane| lane.is_none()).count()
    }

    /// The slots no lane runs on, closed or not: nothing builds in their
    /// checkouts, so the disk floor may prune them.
    #[must_use]
    pub fn idle(&self) -> Vec<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, lane)| lane.is_none().then_some(slot))
            .collect()
    }

    /// A free slot, if any — never once the lanes are closed.
    #[must_use]
    pub fn free_slot(&self) -> Option<usize> {
        if self.is_closed() {
            return None;
        }
        self.slots.iter().position(Option::is_none)
    }

    /// Starts `command` on `slot` for `task`, its output appended to the
    /// lane's log under `here`. Returns the child's pid.
    ///
    /// # Errors
    /// If the log file cannot be opened or the process cannot start.
    pub fn spawn(
        &mut self,
        slot: usize,
        task: u64,
        command: Command,
        here: &Path,
    ) -> std::io::Result<u32> {
        self.spawn_in(slot, task, None, command, here)
    }

    /// [`Self::spawn`], the lane holding `group` while it runs.
    ///
    /// # Errors
    /// If the log file cannot be opened or the process cannot start.
    pub fn spawn_in(
        &mut self,
        slot: usize,
        task: u64,
        group: Option<&'static str>,
        mut command: Command,
        here: &Path,
    ) -> std::io::Result<u32> {
        let dir = here.join(".llocal").join("lanes");
        std::fs::create_dir_all(&dir)?;
        let out = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("lane-{slot}.log")))?;
        let err = out.try_clone()?;
        command.stdin(Stdio::null()).stdout(out).stderr(err);
        let child = command.spawn()?;
        let pid = child.id().unwrap_or(0);
        if let Some(lane) = self.slots.get_mut(slot) {
            *lane = Some(Lane { task, group, child });
        }
        Ok(pid)
    }

    /// Waits for every lane to finish — what `--once` does before it
    /// returns, so one pass is one whole pass.
    pub async fn wait_all(&mut self, log: &Logbook) {
        for (slot, lane) in self.slots.iter_mut().enumerate() {
            if let Some(running) = lane.take() {
                let mut child = running.child;
                match child.wait().await {
                    Ok(status) => log.event(&ended(slot, running.task, status)),
                    Err(e) => log.warn(&format!(
                        "watch: lanes -> lane {slot} (#{}) cannot be waited on: {e}",
                        running.task
                    )),
                }
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use harness_core::domain::{Halt, Outcome};

    use super::*;

    /// A disk of files with sizes and times: a folder's size is its files',
    /// a removal takes a file or a whole folder away, and one path may be
    /// set to refuse its removal.
    #[derive(Default)]
    pub struct Builds {
        files: RefCell<Vec<(PathBuf, u64, Option<u64>)>>,
        refused: Option<PathBuf>,
        pub removed: RefCell<Vec<PathBuf>>,
    }

    impl Builds {
        pub fn with(files: &[(&str, u64, Option<u64>)]) -> Self {
            Self {
                files: RefCell::new(
                    files
                        .iter()
                        .map(|(path, size, at)| (PathBuf::from(path), *size, *at))
                        .collect(),
                ),
                ..Self::default()
            }
        }

        fn gone(&self, path: &Path) -> Outcome<()> {
            if self.refused.as_deref() == Some(path) {
                return Err(Halt::Failed("operation not permitted".to_string()));
            }
            self.removed.borrow_mut().push(path.to_path_buf());
            self.files
                .borrow_mut()
                .retain(|(file, _, _)| !file.starts_with(path));
            Ok(())
        }
    }

    impl Disk for Builds {
        fn exists(&self, path: &Path) -> bool {
            self.files
                .borrow()
                .iter()
                .any(|(file, _, _)| file.starts_with(path))
        }
        fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
            Ok(())
        }
        fn remove_dir_all(&self, path: &Path) -> Outcome<()> {
            self.gone(path)
        }
        fn remove_file(&self, path: &Path) -> Outcome<()> {
            self.gone(path)
        }
        fn dir_names(&self, _path: &Path) -> Vec<String> {
            Vec::new()
        }
        fn file_names(&self, path: &Path) -> Vec<String> {
            let mut names: Vec<String> = self
                .files
                .borrow()
                .iter()
                .filter(|(file, _, _)| file.parent() == Some(path))
                .filter_map(|(file, _, _)| file.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
        fn size_bytes(&self, path: &Path) -> u64 {
            self.files
                .borrow()
                .iter()
                .filter(|(file, _, _)| file.starts_with(path))
                .map(|(_, size, _)| size)
                .sum()
        }
        fn modified_secs(&self, path: &Path) -> Option<u64> {
            self.files
                .borrow()
                .iter()
                .find(|(file, _, _)| file == path)
                .and_then(|(_, _, at)| *at)
        }
        fn read_to_string(&self, _path: &Path) -> Option<String> {
            None
        }
        fn write_to_string(&self, _path: &Path, _content: &str) -> Outcome<()> {
            Ok(())
        }
    }

    const T: &str = "/ws/lane-0/target";

    /// An hour, in seconds.
    const H: u64 = 3600;

    #[test]
    fn within_its_cap_or_without_one_a_target_is_left_alone() {
        let disk = Builds::with(&[(
            "/ws/lane-0/target/debug/deps/a-0123456789abcdef",
            6,
            Some(1),
        )]);
        assert_eq!(prune_target(&disk, Path::new(T), 6), None, "at the cap");
        assert_eq!(prune_target(&disk, Path::new(T), 0), None, "no cap");
        assert_eq!(prune_target(&disk, Path::new("/ws/lane-9/target"), 1), None);
        assert!(disk.removed.borrow().is_empty());
    }

    #[test]
    fn over_its_cap_a_target_drops_incremental_and_the_stale_executables() {
        let deps = "/ws/lane-0/target/debug/deps";
        let disk = Builds::with(&[
            (
                "/ws/lane-0/target/debug/incremental/app-1x/s-abc/dep-graph.bin",
                30,
                Some(5),
            ),
            // Two builds of one test executable, a session apart: the older
            // goes with its notes.
            (&format!("{deps}/journey-00000000000000aa"), 10, Some(100)),
            (&format!("{deps}/journey-00000000000000aa.d"), 1, Some(100)),
            (
                &format!("{deps}/journey-00000000000000bb"),
                10,
                Some(100 + 7 * H),
            ),
            (
                &format!("{deps}/journey-00000000000000bb.d"),
                1,
                Some(100 + 7 * H),
            ),
            // Three of another, the newest kept whatever its hash.
            (&format!("{deps}/app-ffffffffffffffff"), 4, Some(0)),
            (&format!("{deps}/app-1111111111111111"), 4, Some(30 * H)),
            (&format!("{deps}/app-eeeeeeeeeeeeeeee"), 4, Some(20 * H)),
            // A library and a lone executable stay.
            (&format!("{deps}/libapp-00000000000000aa.rlib"), 5, Some(1)),
            (&format!("{deps}/single-0123456789abcdef"), 2, Some(1)),
        ]);
        let pruned = prune_target(&disk, Path::new(T), 50).expect("over the cap");
        assert_eq!(
            pruned,
            Pruned {
                before: 71,
                after: 22,
                what: "incremental caches, 3 stale executable(s)".to_string(),
            }
        );
        let names = disk.file_names(Path::new(deps));
        assert_eq!(
            names,
            [
                "app-1111111111111111",
                "journey-00000000000000bb",
                "journey-00000000000000bb.d",
                "libapp-00000000000000aa.rlib",
                "single-0123456789abcdef",
            ]
        );
    }

    #[test]
    fn above_twice_its_cap_the_profiles_go_and_what_is_not_cargo_s_stays() {
        let disk = Builds::with(&[
            (
                "/ws/lane-0/target/debug/deps/a-0123456789abcdef",
                10,
                Some(1),
            ),
            ("/ws/lane-0/target/release/a", 2, Some(1)),
            // The product's rules runtime: a server's pids, its status.
            ("/ws/lane-0/target/rules-runtime/server.pid", 1, Some(1)),
            ("/ws/lane-0/target/rules-status.jsonl", 1, Some(1)),
            ("/ws/lane-0/src/main.rs", 1, Some(1)),
        ]);
        let pruned = prune_target(&disk, Path::new(T), 6).expect("over twice the cap");
        assert_eq!(pruned.before, 14);
        assert_eq!(pruned.after, 2);
        assert_eq!(
            pruned.what,
            "the debug and release profiles, above twice the cap"
        );
        assert_eq!(
            *disk.removed.borrow(),
            [
                PathBuf::from("/ws/lane-0/target/debug"),
                PathBuf::from("/ws/lane-0/target/release"),
            ]
        );
        for stays in [
            "/ws/lane-0/target/rules-runtime/server.pid",
            "/ws/lane-0/target/rules-status.jsonl",
            "/ws/lane-0/src/main.rs",
        ] {
            assert!(disk.exists(Path::new(stays)), "{stays} stays");
        }
        // One profile alone is said alone.
        let debug = Builds::with(&[("/ws/lane-0/target/debug/x", 13, Some(1))]);
        let pruned = prune_target(&debug, Path::new(T), 6).expect("over twice the cap");
        assert_eq!(pruned.what, "the debug profile, above twice the cap");
        // Exactly twice is pruned, not dropped — and with nothing to prune,
        // nothing is said.
        let twice = Builds::with(&[(
            "/ws/lane-0/target/debug/deps/a-0123456789abcdef",
            12,
            Some(1),
        )]);
        assert_eq!(prune_target(&twice, Path::new(T), 6), None);
        assert!(twice.removed.borrow().is_empty());
        // Over twice, but nothing of cargo's to remove: no news either.
        let foreign = Builds::with(&[("/ws/lane-0/target/rules-runtime/db", 13, Some(1))]);
        assert_eq!(prune_target(&foreign, Path::new(T), 6), None);
    }

    #[test]
    fn executables_of_one_build_session_stay_whatever_their_number() {
        let deps = "/ws/lane-0/target/debug/deps";
        // The product's server: a bin, its test harness and the lib's, all
        // named `server`, linked minutes apart; `report`'s two, five hours
        // apart, still within a session's margin.
        let disk = Builds::with(&[
            (&format!("{deps}/server-00000000000000a1"), 10, Some(10 * H)),
            (
                &format!("{deps}/server-00000000000000a2"),
                10,
                Some(10 * H + 120),
            ),
            (
                &format!("{deps}/server-00000000000000a3"),
                10,
                Some(10 * H + 300),
            ),
            (&format!("{deps}/report-00000000000000c1"), 10, Some(0)),
            (&format!("{deps}/report-00000000000000c2"), 10, Some(5 * H)),
        ]);
        assert_eq!(prune_target(&disk, Path::new(T), 40), None, "none is stale");
        assert!(disk.removed.borrow().is_empty());
        // A name cargo uplifted is a bin's: its twins stay, however old.
        let uplifted = Builds::with(&[
            ("/ws/lane-0/target/debug/server", 10, Some(30 * H)),
            (&format!("{deps}/server-00000000000000a1"), 10, Some(0)),
            (&format!("{deps}/server-00000000000000a2"), 10, Some(30 * H)),
        ]);
        assert_eq!(prune_target(&uplifted, Path::new(T), 20), None);
        assert!(uplifted.removed.borrow().is_empty());
    }

    #[test]
    fn a_build_exactly_the_margin_older_than_the_newest_stays() {
        let deps = "/ws/lane-0/target/debug/deps";
        let built = |older: u64| {
            Builds::with(&[
                (&format!("{deps}/report-00000000000000c1"), 10, Some(0)),
                (&format!("{deps}/report-00000000000000c2"), 10, Some(older)),
            ])
        };
        let at_margin = built(SUPERSEDED_AFTER);
        let profile = Path::new("/ws/lane-0/target/debug");
        assert_eq!(remove_stale(&at_margin, profile), (0, Vec::new()));
        assert!(at_margin.removed.borrow().is_empty());
        let past = built(SUPERSEDED_AFTER + 1);
        assert_eq!(remove_stale(&past, profile), (1, Vec::new()));
    }

    #[test]
    fn a_removal_refused_is_said_and_unknown_times_keep_every_build() {
        let deps = "/ws/lane-0/target/debug/deps";
        let whole = Builds {
            refused: Some(PathBuf::from(T).join("debug")),
            ..Builds::with(&[(&format!("{deps}/a-0123456789abcdef"), 20, Some(1))])
        };
        let pruned = prune_target(&whole, Path::new(T), 6).expect("tried");
        assert_eq!(pruned.what, "debug kept: operation not permitted");
        assert_eq!(pruned.after, 20);
        let incremental = "/ws/lane-0/target/debug/incremental";
        let stuck = Builds {
            refused: Some(PathBuf::from(incremental)),
            ..Builds::with(&[
                (&format!("{incremental}/x"), 4, Some(1)),
                (&format!("{deps}/a-0000000000000001"), 2, Some(1)),
                (&format!("{deps}/a-0000000000000002"), 2, Some(7 * H)),
                (&format!("{deps}/b-0000000000000001"), 2, None),
                (&format!("{deps}/b-0000000000000002"), 2, Some(7 * H)),
            ])
        };
        let pruned = prune_target(&stuck, Path::new(T), 7).expect("tried");
        assert_eq!(
            pruned.what,
            "incremental kept: operation not permitted, 1 stale executable(s)"
        );
        assert_eq!(pruned.after, 10);
        let stale = format!("{deps}/a-0000000000000001");
        let refusing = Builds {
            refused: Some(PathBuf::from(&stale)),
            ..Builds::with(&[
                (&stale, 2, Some(1)),
                (&format!("{deps}/a-0000000000000002"), 2, Some(7 * H)),
            ])
        };
        let pruned = prune_target(&refusing, Path::new(T), 3).expect("tried");
        assert_eq!(
            pruned.what,
            "a-0000000000000001 kept: operation not permitted"
        );
    }

    #[test]
    fn only_a_name_and_sixteen_hex_digits_without_an_extension_is_an_executable() {
        assert_eq!(executable_name("journey-0123456789abcdef"), Some("journey"));
        assert_eq!(
            executable_name("data_layer-0123456789abcdef"),
            Some("data_layer")
        );
        assert_eq!(
            executable_name("my-crate-0123456789abcdef"),
            Some("my-crate")
        );
        for not in [
            "journey-0123456789abcdef.d",
            "libjourney-0123456789abcdef.rlib",
            "journey-0123456789ABCDEF",
            "journey-0123456789abcde",
            "journey-0123456789abcdefa",
            "journey-0123456789abcdeg",
            "-0123456789abcdef",
            "journey",
        ] {
            assert_eq!(executable_name(not), None, "{not}");
        }
    }

    proptest::proptest! {
        #[test]
        fn a_name_then_sixteen_lowercase_hex_digits_is_an_executable(
            name in "[a-z_][a-z0-9_-]{0,20}",
            hash in "[0-9a-f]{16}",
        ) {
            let file = format!("{name}-{hash}");
            proptest::prop_assert_eq!(executable_name(&file), Some(name.as_str()));
        }

        #[test]
        fn a_file_with_a_dot_is_never_an_executable(
            before in "\\PC*",
            after in "\\PC*",
        ) {
            let file = format!("{before}.{after}");
            proptest::prop_assert_eq!(executable_name(&file), None);
        }

        #[test]
        fn a_hash_other_than_sixteen_lowercase_hex_digits_is_never_an_executable(
            name in "[a-z_]{1,10}",
            hash in "[0-9a-zA-Z_]{0,24}",
        ) {
            proptest::prop_assume!(
                hash.len() != 16
                    || !hash.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
            );
            let file = format!("{name}-{hash}");
            proptest::prop_assert_eq!(executable_name(&file), None);
        }
    }

    #[test]
    fn a_real_target_loses_its_incremental_and_older_test_executables() {
        use harness_adapters::shell::disk::RealDisk;
        let root = std::env::temp_dir().join(format!(
            "harness-prune-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let target = root.join("target");
        let deps = target.join("debug/deps");
        let incremental = target.join("debug/incremental/app-1x");
        std::fs::create_dir_all(&deps).expect("deps");
        std::fs::create_dir_all(&incremental).expect("incremental");
        std::fs::write(incremental.join("query-cache.bin"), vec![0_u8; 64 * 1024]).expect("cache");
        let at = |secs| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        for (name, secs) in [
            ("journey-00000000000000aa", 1_000),
            ("journey-00000000000000aa.d", 1_000),
            // Seven hours on: a later build session's.
            ("journey-00000000000000bb", 1_000 + 7 * 3600),
        ] {
            let path = deps.join(name);
            std::fs::write(&path, vec![1_u8; 16 * 1024]).expect("exe");
            std::fs::File::options()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_modified(at(secs)))
                .expect("mtime");
        }
        let before = RealDisk.size_bytes(&target);
        // A cap the target is over, but not twice over.
        let pruned = prune_target(&RealDisk, &target, before * 2 / 3).expect("over the cap");
        assert_eq!(pruned.before, before);
        assert!(pruned.after < before, "{pruned:?}");
        assert_eq!(pruned.what, "incremental caches, 1 stale executable(s)");
        assert!(!target.join("debug/incremental").exists());
        assert_eq!(RealDisk.file_names(&deps), ["journey-00000000000000bb"]);
        std::fs::remove_dir_all(&root).expect("cleaned");
    }

    #[test]
    fn lanes_start_free_and_never_fewer_than_one() {
        let lanes = Lanes::new(3);
        assert_eq!(lanes.free_slot(), Some(0));
        assert_eq!(lanes.idle(), [0, 1, 2]);
        assert_eq!(lanes.running(), [] as [u64; 0]);
        assert_eq!(Lanes::new(0).slots.len(), 1);
    }

    #[test]
    fn a_parked_task_waits_until_its_issue_moves() {
        let mut lanes = Lanes::new(1);
        lanes.parked.insert(15, ("v1".to_string(), Instant::now()));
        assert!(lanes.is_parked(15, "v1"));
        assert!(!lanes.is_parked(16, "v1"));
        assert!(!lanes.is_parked(15, "v2"), "a changed issue is free again");
        assert!(!lanes.is_parked(15, "v1"), "and stays free");
    }

    #[test]
    fn a_parked_task_runs_again_once_the_park_is_over() {
        let mut lanes = Lanes::new(1);
        let at = Instant::now();
        lanes.parked.insert(15, ("v1".to_string(), at));
        let almost = at + PARKED_FOR.saturating_sub(Duration::from_secs(1));
        assert!(
            lanes.is_parked_at(15, "v1", almost),
            "still within the park"
        );
        assert!(
            !lanes.is_parked_at(15, "v1", at + PARKED_FOR),
            "an hour on, the unchanged issue is tried again"
        );
        assert!(!lanes.is_parked_at(15, "v1", almost), "and stays free");
    }

    #[tokio::test]
    async fn a_lane_that_stops_parks_its_task_and_one_that_fails_does_not() {
        let dir = std::env::temp_dir().join(format!("lanes-park-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let log = Logbook::null();
        let mut lanes = Lanes::new(2);
        for (slot, task, code) in [(0, 15, 1), (1, 16, 2)] {
            let mut command = Command::new("sh");
            command.arg("-c").arg(format!("exit {code}"));
            lanes.taken(task, "v1".to_string());
            lanes.spawn(slot, task, command, &dir).expect("spawned");
        }
        for _ in 0..100 {
            lanes.reap(&log);
            if lanes.running().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(lanes.running(), [] as [u64; 0]);
        assert!(lanes.is_parked(15, "v1"), "exit 1 is a stop: parked");
        assert!(!lanes.is_parked(16, "v1"), "exit 2 is a failure: retried");
        assert!(!lanes.struck_by_quota(), "neither was a quota stop");
        assert!(!lanes.went_through(), "neither went through");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_lane_that_ends_on_quota_is_told_once() {
        let dir = std::env::temp_dir().join(format!("lanes-quota-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let log = Logbook::null();
        let mut lanes = Lanes::new(1);
        let mut command = Command::new("sh");
        command.arg("-c").arg("exit 3");
        lanes.taken(15, "v1".to_string());
        lanes.spawn(0, 15, command, &dir).expect("spawned");
        for _ in 0..100 {
            lanes.reap(&log);
            if lanes.running().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            !lanes.is_parked(15, "v1"),
            "exit 3 is not a stop: not parked"
        );
        assert!(lanes.struck_by_quota(), "exit 3 is a quota stop");
        assert!(!lanes.struck_by_quota(), "asking takes the answer");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn closed_lanes_offer_no_slot() {
        let flag = Arc::new(AtomicBool::new(false));
        let lanes = Lanes::new(2).closed_by(Arc::clone(&flag));
        assert_eq!(lanes.free_slot(), Some(0));
        assert_eq!(lanes.free_slots(), 2);
        flag.store(true, Ordering::SeqCst);
        assert!(lanes.is_closed());
        assert_eq!(lanes.free_slot(), None);
        assert_eq!(lanes.free_slots(), 0);
    }

    #[tokio::test]
    async fn a_lane_holds_its_group_while_it_runs_and_a_success_is_told_once() {
        let dir = std::env::temp_dir().join(format!("lanes-group-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let log = Logbook::null();
        let mut lanes = Lanes::new(3);
        let mut ok = Command::new("sh");
        ok.arg("-c").arg("sleep 0.2; exit 0");
        lanes
            .spawn_in(0, 7, Some("tech-refinement"), ok, &dir)
            .expect("spawned");
        let mut plain = Command::new("sh");
        plain.arg("-c").arg("sleep 0.2; exit 2");
        lanes.spawn(1, 8, plain, &dir).expect("spawned");
        assert_eq!(lanes.groups(), ["tech-refinement"]);
        assert_eq!(lanes.free_slots(), 1);
        assert!(!lanes.went_through(), "nothing ended yet");
        for _ in 0..100 {
            lanes.reap(&log);
            if lanes.running().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            lanes.groups().is_empty(),
            "the group is freed with its lane"
        );
        assert_eq!(lanes.free_slots(), 3);
        assert!(lanes.went_through(), "exit 0 went through");
        assert!(!lanes.went_through(), "asking takes the answer");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_spawned_lane_is_running_until_reaped_and_wait_all_drains_it() {
        let dir = std::env::temp_dir().join(format!("harness-lanes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        let mut lanes = Lanes::new(2);
        let command = Command::new("true");
        let pid = lanes.spawn(0, 42, command, &dir).expect("spawned");
        assert!(pid > 0);
        assert_eq!(lanes.running(), [42]);
        assert_eq!(lanes.free_slot(), Some(1));
        lanes.wait_all(&Logbook::null()).await;
        assert_eq!(lanes.running(), [] as [u64; 0]);
        assert!(dir.join(".llocal/lanes/lane-0.log").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
