//! The watch process, through the OS: `ps` to find it, a detached spawn to
//! start it, `kill` to stop it.
//!
//! Only a watch whose working directory is this checkout counts: another
//! plant's watch on the same machine is none of this page's business.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use harness_core::domain::workspace::Workspace;

use crate::domain::plant;
use crate::ports::{Plant, Process, Signal, Target};

/// The watch of one checkout, started with one command line.
pub struct WatchProcess {
    workspace: Workspace,
    command: Vec<String>,
}

impl WatchProcess {
    /// The watch of `workspace`, started as `command` (program first, a
    /// relative program resolved against the checkout).
    #[must_use]
    pub fn new(workspace: Workspace, command: &str) -> Self {
        Self {
            workspace,
            command: command.split_whitespace().map(str::to_string).collect(),
        }
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

impl Plant for WatchProcess {
    fn processes(&self) -> Vec<Process> {
        Command::new("ps")
            .args(["-axo", "pid=,ppid=,pgid=,command="])
            .output()
            .map(|out| plant::parse_ps(&String::from_utf8_lossy(&out.stdout)))
            .unwrap_or_default()
    }

    fn works_here(&self, pid: u32) -> bool {
        let Ok(out) = Command::new("lsof")
            .args(["-a", "-d", "cwd", "-Fn", "-p", &pid.to_string()])
            .output()
        else {
            return false;
        };
        let here = self.workspace.root().canonicalize().ok();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix('n'))
            .any(|cwd| Path::new(cwd).canonicalize().ok() == here)
    }

    fn start(&self) -> Result<u32, String> {
        let program = self
            .program()
            .ok_or_else(|| "no watch command is configured".to_string())?;
        if program.components().count() > 1 && !program.is_file() {
            return Err(format!(
                "{} is not built — cargo build -p harness-launcher",
                program.display()
            ));
        }
        let journal = self.workspace.loop_dir();
        std::fs::create_dir_all(&journal).map_err(|e| format!("{}: {e}", journal.display()))?;
        let out_path = journal.join("watch.out");
        let out = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&out_path)
            .map_err(|e| format!("{}: {e}", out_path.display()))?;
        let err = out
            .try_clone()
            .map_err(|e| format!("{}: {e}", out_path.display()))?;
        // A group of its own: the view restarting does not take the watch
        // with it, and a hard stop reaches its lanes and nothing else. Tokio
        // reaps the child once it is dropped, so no zombie stays behind.
        let child = tokio::process::Command::new(&program)
            .args(self.command.iter().skip(1))
            .current_dir(self.workspace.root())
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .map_err(|e| format!("{} cannot start: {e}", program.display()))?;
        child
            .id()
            .ok_or_else(|| "the watch exited as soon as it started".to_string())
    }

    fn send(&self, signal: Signal, target: Target) -> Result<(), String> {
        let name = match signal {
            Signal::Term => "-TERM",
            Signal::Kill => "-KILL",
        };
        let who = match target {
            Target::Process(pid) => pid.to_string(),
            Target::Group(group) => format!("-{group}"),
        };
        let out = Command::new("kill")
            .args([name, "--", &who])
            .output()
            .map_err(|e| format!("kill cannot run: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "kill {name} {who}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
}
