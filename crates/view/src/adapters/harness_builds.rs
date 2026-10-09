//! The harness's own commands, started for the data pane: `harness
//! data-snapshot` and `harness data-model-draft`, each a child of the view,
//! its output in `.llocal/logs/view-builds/`.

use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::domain::builds::{Build, State, What};
use crate::ports::Builds;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How many of a build's last lines the page reads.
const TAIL: usize = 6;

struct Run {
    what: What,
    branch: String,
    started: String,
    out: PathBuf,
    child: Option<Child>,
    state: State,
}

/// The harness binary, started from the checkout.
pub struct HarnessBuilds {
    program: PathBuf,
    root: PathBuf,
    runs: Mutex<Vec<Run>>,
}

impl HarnessBuilds {
    /// Starts `program` (the harness binary) from `root`, the checkout.
    #[must_use]
    pub const fn new(program: PathBuf, root: PathBuf) -> Self {
        Self {
            program,
            root,
            runs: Mutex::new(Vec::new()),
        }
    }
}

fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn tail(path: &PathBuf) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(TAIL)..].join("\n")
}

impl Builds for HarnessBuilds {
    fn start(&self, what: What, branch: &str) -> Result<(), String> {
        let twin = {
            let mut runs = lock(&self.runs);
            runs.iter_mut().any(|r| {
                refresh(r);
                r.what == what && r.branch == branch && r.state == State::Running
            })
        };
        if twin {
            return Err(format!("already building for {branch}"));
        }
        let dir = self.root.join(".llocal/logs/view-builds");
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let started = now();
        let out = dir.join(format!(
            "{}-{}-{}.out",
            started.replace(':', ""),
            match what {
                What::Schema => "schema",
                What::Model => "model",
            },
            branch.replace('/', "__")
        ));
        let file = File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        let err = file.try_clone().map_err(|e| e.to_string())?;
        let child = Command::new(&self.program)
            .args(what.args(branch))
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(file)
            .stderr(err)
            .spawn()
            .map_err(|e| format!("{} cannot start: {e}", self.program.display()))?;
        lock(&self.runs).insert(
            0,
            Run {
                what,
                branch: branch.to_string(),
                started,
                out,
                child: Some(child),
                state: State::Running,
            },
        );
        Ok(())
    }

    fn builds(&self) -> Vec<Build> {
        let mut runs = lock(&self.runs);
        runs.iter_mut()
            .map(|run| {
                refresh(run);
                Build {
                    what: run.what,
                    branch: run.branch.clone(),
                    state: run.state,
                    started: run.started.clone(),
                    tail: tail(&run.out),
                }
            })
            .collect()
    }
}

/// A running build that ended is reaped, its state read from its exit.
fn refresh(run: &mut Run) {
    let Some(child) = run.child.as_mut() else {
        return;
    };
    if let Ok(Some(status)) = child.try_wait() {
        run.state = if status.success() {
            State::Done
        } else {
            State::Failed
        };
        run.child = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_runs_reports_its_tail_and_refuses_a_twin() {
        let root = std::env::temp_dir().join(format!("harness-builds-{}", std::process::id()));
        fs::create_dir_all(&root).expect("root");
        // A stand-in for the harness: prints its arguments, then fails.
        let program = root.join("fake-harness.sh");
        fs::write(&program, "#!/bin/sh\necho \"ran $*\"\nsleep 0.3\nexit 3\n").expect("script");
        #[allow(clippy::permissions_set_readonly_false)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let builds = HarnessBuilds::new(program, root.clone());
        builds.start(What::Schema, "main").expect("started");
        assert!(
            builds.start(What::Schema, "main").is_err(),
            "the same build twice"
        );
        builds.start(What::Model, "main").expect("another build");
        for _ in 0..50 {
            if builds.builds().iter().all(|b| b.state != State::Running) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let all = builds.builds();
        assert_eq!(all.len(), 2);
        let schema = all.iter().find(|b| b.what == What::Schema).expect("schema");
        assert_eq!(schema.state, State::Failed);
        assert_eq!(schema.tail, "ran data-snapshot --branch main");
        let _ = fs::remove_dir_all(&root);
    }
}
