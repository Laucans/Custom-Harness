//! A prompt's dry-run, run as `harness trial` in a process of its own.
//!
//! The text tried goes on the process's stdin, never to a file: a draft is
//! nothing a run may read. The process writes its report under
//! `.llocal/trials/<id>/`; this side reads it back, and tells a process that
//! died before reporting from one still at work.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use harness_adapters::store::trial::{self, Report, State};
use harness_core::domain::workspace::Workspace;

use crate::ports::{TrialAsk, Trials};

/// `harness trial`, started from this checkout.
pub struct TrialProcess {
    workspace: Workspace,
    command: Vec<String>,
    children: Mutex<HashMap<String, Child>>,
}

impl TrialProcess {
    /// Trials of `workspace`, started as `command` (program first, a
    /// relative program resolved against the checkout).
    #[must_use]
    pub fn new(workspace: Workspace, command: &str) -> Self {
        Self {
            workspace,
            command: command.split_whitespace().map(str::to_string).collect(),
            children: Mutex::default(),
        }
    }

    fn dir(&self) -> PathBuf {
        self.workspace
            .state_root()
            .join(".llocal")
            .join(trial::TRIALS)
    }

    fn program(&self) -> Option<PathBuf> {
        let first = Path::new(self.command.first()?);
        Some(if first.is_relative() && first.components().count() > 1 {
            self.workspace.root().join(first)
        } else {
            first.to_path_buf()
        })
    }
}

impl Trials for TrialProcess {
    fn start(&self, ask: &TrialAsk) -> Result<(), String> {
        let program = self
            .program()
            .ok_or_else(|| "no trial command is configured".to_string())?;
        if program.components().count() > 1 && !program.is_file() {
            return Err(format!(
                "{} is not built — cargo build --release -p harness-launcher",
                program.display()
            ));
        }
        let dir = self.dir().join(&ask.id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let out_path = dir.join("trial.out");
        let out = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&out_path)
            .map_err(|e| format!("{}: {e}", out_path.display()))?;
        let err = out
            .try_clone()
            .map_err(|e| format!("{}: {e}", out_path.display()))?;
        let mut child = Command::new(&program)
            .args(self.command.iter().skip(1))
            .args([
                "--workflow",
                &ask.workflow,
                "--run",
                &ask.run,
                "--key",
                &ask.key,
                "--version",
                &ask.version,
                "--model",
                &ask.model,
                "--effort",
                &ask.effort,
                "--id",
                &ask.id,
            ])
            .current_dir(self.workspace.root())
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(out)
            .stderr(err)
            .spawn()
            .map_err(|e| format!("{} cannot start: {e}", program.display()))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(ask.text.as_bytes())
                .map_err(|e| format!("the text could not be handed over: {e}"))?;
        }
        if let Ok(mut children) = self.children.lock() {
            children.insert(ask.id.clone(), child);
        }
        Ok(())
    }

    fn reports(&self) -> Vec<(String, Report)> {
        let Ok(entries) = std::fs::read_dir(self.dir()) else {
            return Vec::new();
        };
        let mut children = self.children.lock().ok();
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let id = entry.file_name().to_string_lossy().into_owned();
                let mut report = trial::read_report(&entry.path());
                // A process gone with its report still saying "running" died
                // before it could say anything else.
                let ended = children.as_mut().and_then(|map| {
                    let child = map.get_mut(&id)?;
                    let status = child.try_wait().ok()??;
                    map.remove(&id);
                    Some(status)
                });
                match (&mut report, ended) {
                    (Some(found), Some(status)) if found.state == State::Running => {
                        found.state = State::Failed;
                        found.reason =
                            Some(format!("the trial ended ({status}) without reporting"));
                    }
                    (None, Some(status)) => {
                        let printed = std::fs::read_to_string(entry.path().join("trial.out"))
                            .unwrap_or_default();
                        report = Some(Report {
                            state: State::Failed,
                            reason: Some(format!("the trial ended ({status}): {}", printed.trim())),
                            ..Report::default()
                        });
                    }
                    _ => {}
                }
                report.map(|report| (id, report))
            })
            .collect()
    }
}
