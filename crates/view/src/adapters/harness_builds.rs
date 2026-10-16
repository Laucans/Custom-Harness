//! The harness's own commands, started for the data pane: `harness
//! data-snapshot` and `harness data-model-draft`, each a child of the view,
//! its output in `.llocal/logs/view-builds/`.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::domain::builds::{self, Build, State, What};
use crate::ports::Builds;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How much of a session's own log the build's log carries.
const SESSION_TAIL: usize = 64 * 1024;

/// How many of a build's last lines the page reads.
const TAIL: usize = 6;

struct Run {
    id: String,
    what: What,
    branch: String,
    started: String,
    out: PathBuf,
    child: Option<Child>,
    state: State,
}

/// The harness binary, started from the checkout.
pub struct HarnessBuilds {
    /// Where the binary may be, in order of preference: the first that
    /// exists when a build starts is started — one put in place after the
    /// view started is found.
    candidates: Vec<PathBuf>,
    root: PathBuf,
    runs: Mutex<Vec<Run>>,
}

impl HarnessBuilds {
    /// Starts the first of `candidates` that exists (the harness binary)
    /// from `root`, the checkout.
    #[must_use]
    pub const fn new(candidates: Vec<PathBuf>, root: PathBuf) -> Self {
        Self {
            candidates,
            root,
            runs: Mutex::new(Vec::new()),
        }
    }
}

impl HarnessBuilds {
    /// Whether the harness binary knows the command `what` starts — a binary
    /// built before the command existed answers `unrecognized subcommand`,
    /// and starting it would only fail the same way, later.
    /// The binary a build starts now: the first candidate that exists, else
    /// the last — whose absence the probe then reports.
    fn program(&self) -> PathBuf {
        self.candidates
            .iter()
            .find(|path| path.is_file())
            .or_else(|| self.candidates.last())
            .cloned()
            .unwrap_or_else(|| PathBuf::from("harness"))
    }

    fn knows(&self, program: &PathBuf, what: What) -> Result<(), String> {
        let command = what.args("")[0].clone();
        let probe = Command::new(program)
            .args([command.as_str(), "--help"])
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("the harness binary {} cannot start: {e}", program.display()))?;
        if probe.status.success() {
            return Ok(());
        }
        Err(format!(
            "the harness binary {} does not know `{command}`: it was built before it. Rebuild it \
             from the harness checkout (`cargo build --release -p harness-launcher`), or point the \
             view at a fresh one with --harness-bin",
            program.display()
        ))
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
    fn start(&self, what: What, branch: &str) -> Result<String, String> {
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
        let program = self.program();
        self.knows(&program, what)?;
        let dir = self.root.join(".llocal/logs/view-builds");
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let started = now();
        let id = format!(
            "{}-{}-{}",
            started.replace(':', ""),
            match what {
                What::Schema => "schema",
                What::Model => "model",
            },
            branch.replace('/', "__")
        );
        let out = dir.join(format!("{id}.out"));
        let file = File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        let err = file.try_clone().map_err(|e| e.to_string())?;
        let child = Command::new(&program)
            .args(what.args(branch))
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(file)
            .stderr(err)
            .spawn()
            .map_err(|e| format!("{} cannot start: {e}", program.display()))?;
        lock(&self.runs).insert(
            0,
            Run {
                id: id.clone(),
                what,
                branch: branch.to_string(),
                started,
                out,
                child: Some(child),
                state: State::Running,
            },
        );
        Ok(id)
    }

    fn log(&self, id: &str) -> Option<String> {
        let out = lock(&self.runs).iter().find(|r| r.id == id)?.out.clone();
        let mut text = fs::read_to_string(out).ok()?;
        // The sessions the command opened say what they do in their own
        // journal: followed too, as long as it lies in this checkout's logs.
        let logs = self.root.join(".llocal/logs");
        for dir in builds::journals(&text.clone()) {
            let dir = PathBuf::from(dir);
            if !dir.starts_with(&logs)
                || dir
                    .components()
                    .any(|c| c == std::path::Component::ParentDir)
            {
                continue;
            }
            if let Ok(session) = fs::read_to_string(dir.join("session.log")) {
                let start = session.len().saturating_sub(SESSION_TAIL);
                let start = (start..session.len())
                    .find(|&k| session.is_char_boundary(k))
                    .unwrap_or(0);
                let _ = write!(text, "\n--- the session · {} ---\n", dir.display());
                text.push_str(&session[start..]);
            }
        }
        Some(text)
    }

    fn builds(&self) -> Vec<Build> {
        let mut runs = lock(&self.runs);
        runs.iter_mut()
            .map(|run| {
                refresh(run);
                Build {
                    id: run.id.clone(),
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

    /// Writes an executable script through a child shell: no writable handle
    /// on it ever lives in this process, so a test forking meanwhile cannot
    /// hold one and make its exec fail with "Text file busy".
    fn executable(path: &std::path::Path, text: &str) {
        let written = Command::new("/bin/sh")
            .args([
                "-c",
                "printf '%s' \"$2\" > \"$1\" && chmod 755 \"$1\"",
                "sh",
            ])
            .arg(path)
            .arg(text)
            .status()
            .expect("script");
        assert!(written.success(), "the script is written");
    }

    #[test]
    fn a_build_runs_reports_its_tail_and_refuses_a_twin() {
        let root = std::env::temp_dir().join(format!("harness-builds-{}", std::process::id()));
        fs::create_dir_all(&root).expect("root");
        // A stand-in for the harness: prints its arguments, then fails.
        let program = root.join("fake-harness.sh");
        executable(
            &program,
            "#!/bin/sh\ncase \"$2\" in --help) exit 0 ;; esac\necho \"ran $*\"\nsleep 0.3\nexit 3\n",
        );
        // The first candidate is missing: the second is started.
        let builds = HarnessBuilds::new(vec![root.join("absent"), program], root.clone());
        let id = builds.start(What::Schema, "main").expect("started");
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
        assert_eq!(schema.id, id);
        assert_eq!(
            builds.log(&id).as_deref(),
            Some("ran data-snapshot --branch main\n")
        );
        assert!(builds.log("nope").is_none());

        // A binary built before the command: refused up front, with the fix.
        let old = root.join("old-harness.sh");
        executable(
            &old,
            "#!/bin/sh\necho \"error: unrecognized subcommand '$1'\" >&2\nexit 2\n",
        );
        let stale = HarnessBuilds::new(vec![old], root.clone());
        let refused = stale.start(What::Schema, "main").expect_err("refused");
        assert!(
            refused.contains("built before it") && refused.contains("cargo build --release"),
            "{refused}"
        );
        assert!(stale.builds().is_empty(), "nothing started");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_build_s_log_follows_the_sessions_its_command_journals() {
        let root =
            std::env::temp_dir().join(format!("harness-builds-journal-{}", std::process::id()));
        let journal = root.join(".llocal/logs/data-model-draft/1");
        fs::create_dir_all(&journal).expect("journal");
        fs::write(
            journal.join("session.log"),
            "[  0m03s] → Read crates/dataguard/src/queue.rs\n",
        )
        .expect("session");
        let outside =
            std::env::temp_dir().join(format!("harness-builds-outside-{}", std::process::id()));
        fs::create_dir_all(&outside).expect("outside");
        fs::write(outside.join("session.log"), "secret").expect("outside");
        let program = root.join("fake-harness.sh");
        executable(
            &program,
            &format!(
                "#!/bin/sh\ncase \"$2\" in --help) exit 0 ;; esac\necho \"journal: {}\"\necho \"journal: {}\"\n",
                journal.display(),
                outside.display()
            ),
        );
        let builds = HarnessBuilds::new(vec![program], root.clone());
        let id = builds.start(What::Model, "main_agent").expect("started");
        for _ in 0..50 {
            if builds.builds().iter().all(|b| b.state != State::Running) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let log = builds.log(&id).expect("a log");
        assert!(log.contains("--- the session ·"), "{log}");
        assert!(
            log.contains("→ Read crates/dataguard/src/queue.rs"),
            "{log}"
        );
        assert!(
            !log.contains("secret"),
            "a journal outside the checkout's logs is not read"
        );
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }
}
