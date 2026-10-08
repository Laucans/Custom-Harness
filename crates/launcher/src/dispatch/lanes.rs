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

use std::fs::OpenOptions;
use std::path::Path;
use std::process::Stdio;

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
}

impl Lanes {
    /// `parallel` lanes, all free.
    #[must_use]
    pub fn new(parallel: usize) -> Self {
        Self {
            slots: (0..parallel.max(1)).map(|_| None).collect(),
        }
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

    /// The tasks lanes are on right now.
    #[must_use]
    pub fn running(&self) -> Vec<u64> {
        self.slots
            .iter()
            .filter_map(|lane| lane.as_ref().map(|running| running.task))
            .collect()
    }

    /// A free slot, if any.
    #[must_use]
    pub fn free_slot(&self) -> Option<usize> {
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
