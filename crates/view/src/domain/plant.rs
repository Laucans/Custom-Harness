//! The plant's switch: what the page's start, soft stop and hard stop mean,
//! decided before anything is sent.
//!
//! A soft stop is `SIGTERM` to the watch alone — it closes its lanes, waits
//! for the running tasks, then exits. A hard stop is `SIGKILL` to the
//! watch's whole tree — its lanes and their sessions — and to its process
//! group as well when that group holds nothing else: a session that daemonised
//! a helper (a database, a server) left it in the group, orphaned. A group
//! that holds anything outside the tree is never killed, since it then holds
//! processes that are not the plant's.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::ports::{Plant, Process, Running, Signal, Target};

/// What the page asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    /// Start the watch.
    Start,
    /// Let the running tasks finish, start none, then stop.
    Soft,
    /// Stop everything now.
    Hard,
}

impl Gesture {
    /// `start`, `soft` or `hard`, as the route names them.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "start" => Some(Self::Start),
            "soft" => Some(Self::Soft),
            "hard" => Some(Self::Hard),
            _ => None,
        }
    }
}

/// One thing to do for a gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Start a watch.
    Start,
    /// Send a signal.
    Send(Signal, Target),
}

/// What the page shows of the switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The view was started with a hand on the plant at all.
    pub available: bool,
    /// The watch process, if one runs.
    pub running: Option<Running>,
}

/// Parses a `ps -axo pid=,ppid=,pgid=,command=` listing.
#[must_use]
pub fn parse_ps(listing: &str) -> Vec<Process> {
    listing
        .lines()
        .filter_map(|line| {
            let mut rest = line.trim_start();
            let mut number = || -> Option<u32> {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let (word, tail) = rest.split_at(end);
                rest = tail.trim_start();
                word.parse().ok()
            };
            let pid = number()?;
            let parent = number()?;
            let group = number()?;
            Some(Process {
                pid,
                parent,
                group,
                command: rest.to_string(),
            })
        })
        .collect()
}

/// Whether a command line is `harness watch …`: a program named `harness`
/// whose first argument is `watch`.
fn is_watch(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let program = words.next().and_then(|p| p.rsplit('/').next());
    program == Some("harness") && words.next() == Some("watch")
}

/// The watch of this checkout, if one runs.
#[must_use]
pub fn running(plant: &dyn Plant) -> Option<Running> {
    plant
        .processes()
        .into_iter()
        .filter(|p| is_watch(&p.command))
        .find(|p| plant.works_here(p.pid))
        .map(|p| Running {
            pid: p.pid,
            group: p.group,
        })
}

/// `root` and every process descended from it.
fn tree(processes: &[Process], root: u32) -> BTreeSet<u32> {
    let mut found = BTreeSet::from([root]);
    loop {
        let before = found.len();
        for p in processes {
            if found.contains(&p.parent) {
                found.insert(p.pid);
            }
        }
        if found.len() == before {
            return found;
        }
    }
}

/// Decides what a gesture does, given the watch that runs and the process
/// table.
///
/// # Errors
///
/// The gesture makes no sense now: a start while a watch runs, a stop with
/// none.
pub fn decide(
    gesture: Gesture,
    running: Option<Running>,
    processes: &[Process],
) -> Result<Vec<Action>, String> {
    match (gesture, running) {
        (Gesture::Start, None) => Ok(vec![Action::Start]),
        (Gesture::Start, Some(watch)) => Err(format!(
            "a watch already runs (pid {}) — one per checkout",
            watch.pid
        )),
        (Gesture::Soft | Gesture::Hard, None) => Err("no watch runs".to_string()),
        (Gesture::Soft, Some(watch)) => {
            Ok(vec![Action::Send(Signal::Term, Target::Process(watch.pid))])
        }
        (Gesture::Hard, Some(watch)) => {
            let tree = tree(processes, watch.pid);
            let group_is_ours = processes
                .iter()
                .filter(|p| p.group == watch.group)
                .all(|p| tree.contains(&p.pid) || p.parent == 1);
            let mut actions: Vec<Action> = tree
                .iter()
                .map(|&pid| Action::Send(Signal::Kill, Target::Process(pid)))
                .collect();
            if group_is_ours {
                actions.push(Action::Send(Signal::Kill, Target::Group(watch.group)));
            }
            Ok(actions)
        }
    }
}

#[cfg(test)]
pub mod fake {
    //! An in-memory switch: what a test sets runs, what the page sends is kept.

    use std::sync::Mutex;

    use crate::ports::{Plant, Process, Signal, Target};

    /// A plant whose process table is what the test wrote.
    #[derive(Default)]
    pub struct Switch {
        /// The process table.
        pub processes: Vec<Process>,
        /// Every start and signal, in order.
        pub sent: Mutex<Vec<String>>,
    }

    impl Plant for Switch {
        fn processes(&self) -> Vec<Process> {
            self.processes.clone()
        }

        fn works_here(&self, _pid: u32) -> bool {
            true
        }

        fn start(&self) -> Result<u32, String> {
            if let Ok(mut sent) = self.sent.lock() {
                sent.push("start".to_string());
            }
            Ok(4242)
        }

        fn send(&self, signal: Signal, target: Target) -> Result<(), String> {
            if let Ok(mut sent) = self.sent.lock() {
                sent.push(format!("{signal:?} {target:?}"));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn process(pid: u32, parent: u32, group: u32, command: &str) -> Process {
        Process {
            pid,
            parent,
            group,
            command: command.to_string(),
        }
    }

    const WATCH: Running = Running {
        pid: 100,
        group: 50,
    };

    /// A watch whose group leader is gone, a lane, its session, and a
    /// database the session daemonised.
    fn plant() -> Vec<Process> {
        vec![
            process(100, 1, 50, "./target/debug/harness watch --parallel 10"),
            process(110, 100, 50, "./target/debug/harness --task 15"),
            process(120, 110, 50, "claude -p"),
            process(130, 1, 50, "postgres -D pgdata"),
            process(900, 1, 900, "/bin/zsh"),
        ]
    }

    #[test]
    fn a_start_needs_an_empty_plant() {
        assert_eq!(decide(Gesture::Start, None, &[]), Ok(vec![Action::Start]));
        assert!(decide(Gesture::Start, Some(WATCH), &plant()).is_err());
    }

    #[test]
    fn a_stop_needs_a_watch() {
        assert!(decide(Gesture::Soft, None, &[]).is_err());
        assert!(decide(Gesture::Hard, None, &[]).is_err());
    }

    #[test]
    fn a_soft_stop_asks_the_watch_alone() {
        assert_eq!(
            decide(Gesture::Soft, Some(WATCH), &plant()),
            Ok(vec![Action::Send(Signal::Term, Target::Process(100))])
        );
    }

    #[test]
    fn a_hard_stop_kills_the_tree_and_a_group_that_holds_only_the_plant() {
        let kill = |target| Action::Send(Signal::Kill, target);
        assert_eq!(
            decide(Gesture::Hard, Some(WATCH), &plant()),
            Ok(vec![
                kill(Target::Process(100)),
                kill(Target::Process(110)),
                kill(Target::Process(120)),
                kill(Target::Group(50)),
            ])
        );
    }

    #[test]
    fn a_hard_stop_spares_a_group_shared_with_a_stranger() {
        let mut shared = plant();
        // A shell still alive in the watch's group: not the plant's.
        shared.push(process(60, 40, 50, "/bin/zsh"));
        let actions = decide(Gesture::Hard, Some(WATCH), &shared).expect("decided");
        assert!(!actions.contains(&Action::Send(Signal::Kill, Target::Group(50))));
        assert_eq!(actions.len(), 3);
    }

    #[test]
    fn a_ps_listing_parses_with_its_padding() {
        let listing = "  100     1    50 ./target/debug/harness watch --parallel 10\n\
                       garbage\n\
                       120   110    50 claude -p --model x\n";
        assert_eq!(
            parse_ps(listing),
            [
                process(100, 1, 50, "./target/debug/harness watch --parallel 10"),
                process(120, 110, 50, "claude -p --model x"),
            ]
        );
    }

    #[test]
    fn only_a_harness_watch_is_a_watch() {
        assert!(is_watch("./target/debug/harness watch --parallel 10"));
        assert!(is_watch("/usr/local/bin/harness watch"));
        assert!(!is_watch("./target/debug/harness --task 15"));
        assert!(!is_watch("/x/target/debug/harness-view --port 7879"));
        assert!(!is_watch("/bin/zsh -c pgrep -fl harness watch"));
    }

    #[test]
    fn the_running_watch_is_found_in_the_table() {
        let switch = fake::Switch {
            processes: plant(),
            ..fake::Switch::default()
        };
        assert_eq!(running(&switch), Some(WATCH));
        assert_eq!(running(&fake::Switch::default()), None);
    }

    #[test]
    fn gestures_are_named_as_the_route_names_them() {
        assert_eq!(Gesture::parse("start"), Some(Gesture::Start));
        assert_eq!(Gesture::parse("soft"), Some(Gesture::Soft));
        assert_eq!(Gesture::parse("hard"), Some(Gesture::Hard));
        assert_eq!(Gesture::parse("kill"), None);
    }

    proptest! {
        #[test]
        fn any_listing_parses_without_panicking(text in "\\PC{0,300}") {
            let _ = parse_ps(&text);
        }
    }
}
