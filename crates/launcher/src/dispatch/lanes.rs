//! The lanes of a parallel `watch`: one child process per issue, up to
//! `--parallel` at once, each in its own workspace.
//!
//! The harness keeps driving **one session per process** (decision #3,
//! `?Send` throughout): parallelism is processes, not threads. The watch
//! spawns `harness --task <n> --use-workspace lane-<k>` for every runnable
//! task the board offers that no lane is on — or `harness split <n>` /
//! `harness refine <n>` with `--use-workspace router-lane-<k>` for every
//! milestone ready to split, then every issue waiting on its business
//! refinement — reaps the children on each tick, and frees their lane. What
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
//! frees it.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use harness_core::traces::Logbook;
use tokio::process::{Child, Command};

/// One running lane.
struct Lane {
    task: u64,
    child: Child,
}

/// The lanes, by slot.
pub struct Lanes {
    slots: Vec<Option<Lane>>,
    /// The issue version each dispatched task was taken at.
    taken_at: HashMap<u64, String>,
    /// Tasks whose lane stopped, with the issue version it stopped on.
    parked: HashMap<u64, String>,
    /// Set once a soft stop is asked: no slot is free from then on.
    closed: Arc<AtomicBool>,
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

    /// Frees the lanes whose child has exited, saying how it went.
    pub fn reap(&mut self, log: &Logbook) {
        for (slot, lane) in self.slots.iter_mut().enumerate() {
            let done = match lane {
                Some(running) => match running.child.try_wait() {
                    Ok(Some(status)) => {
                        log.say(&format!(
                            "watch: lanes -> lane {slot} done with #{} ({status})",
                            running.task
                        ));
                        let version = self.taken_at.remove(&running.task);
                        if status.code() == Some(1)
                            && let Some(version) = version
                        {
                            log.say(&format!(
                                "watch: lanes -> #{} parked: it stopped for a human, and is not \
                                 taken again until its issue changes",
                                running.task
                            ));
                            self.parked.insert(running.task, version);
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
            }
        }
    }

    /// Notes the issue version `task` is taken at, so a stop can park it on
    /// that version.
    pub fn taken(&mut self, task: u64, version: String) {
        self.taken_at.insert(task, version);
    }

    /// Whether `task` is parked on this very `version`. A task whose issue
    /// moved since its stop is unparked here, and runs again.
    pub fn is_parked(&mut self, task: u64, version: &str) -> bool {
        match self.parked.get(&task) {
            Some(stopped_on) if stopped_on == version => true,
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
            *lane = Some(Lane { task, child });
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
                    Ok(status) => log.say(&format!(
                        "watch: lanes -> lane {slot} done with #{} ({status})",
                        running.task
                    )),
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
mod tests {
    use super::*;

    #[test]
    fn lanes_start_free_and_never_fewer_than_one() {
        let lanes = Lanes::new(3);
        assert_eq!(lanes.free_slot(), Some(0));
        assert_eq!(lanes.running(), [] as [u64; 0]);
        assert_eq!(Lanes::new(0).slots.len(), 1);
    }

    #[test]
    fn a_parked_task_waits_until_its_issue_moves() {
        let mut lanes = Lanes::new(1);
        lanes.parked.insert(15, "v1".to_string());
        assert!(lanes.is_parked(15, "v1"));
        assert!(!lanes.is_parked(16, "v1"));
        assert!(!lanes.is_parked(15, "v2"), "a changed issue is free again");
        assert!(!lanes.is_parked(15, "v1"), "and stays free");
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
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn closed_lanes_offer_no_slot() {
        let flag = Arc::new(AtomicBool::new(false));
        let lanes = Lanes::new(2).closed_by(Arc::clone(&flag));
        assert_eq!(lanes.free_slot(), Some(0));
        flag.store(true, Ordering::SeqCst);
        assert!(lanes.is_closed());
        assert_eq!(lanes.free_slot(), None);
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
