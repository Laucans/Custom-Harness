//! A pseudo-terminal running the steward's program — the one module of the
//! view that spawns a process.
//!
//! `portable-pty` opens the pair; the program runs in the harness checkout
//! with a real terminal, so Claude Code shows its interactive screen rather
//! than its headless one.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::ports::{TerminalFactory, TerminalIo};

/// The steward's program, ready to be started in a terminal.
pub struct Pty {
    cwd: PathBuf,
    command: String,
    args: Vec<String>,
}

impl Pty {
    /// `command args…`, to be run in `cwd`.
    #[must_use]
    pub fn new(cwd: &Path, command: &str, args: Vec<String>) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            command: command.to_string(),
            args,
        }
    }
}

const fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

struct Open {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    reader: Option<Box<dyn Read + Send>>,
}

impl TerminalIo for Open {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        self.master
            .resize(size(cols, rows))
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn output(&mut self) -> Option<Box<dyn Read + Send>> {
        self.reader.take()
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        self.kill();
    }
}

impl TerminalFactory for Pty {
    fn spawn(&self, cols: u16, rows: u16) -> Result<Box<dyn TerminalIo>, String> {
        let pair = native_pty_system()
            .openpty(size(cols, rows))
            .map_err(|e| format!("no pseudo-terminal: {e}"))?;
        let mut command = CommandBuilder::new(&self.command);
        command.args(&self.args);
        command.cwd(&self.cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("cannot start `{}`: {e}", self.command))?;
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("no terminal output: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("no terminal input: {e}"))?;
        Ok(Box::new(Open {
            master: pair.master,
            child,
            writer,
            reader: Some(reader),
        }))
    }

    fn command(&self) -> String {
        std::iter::once(self.command.clone())
            .chain(self.args.iter().map(|arg| {
                if arg.len() > 40 || arg.contains(' ') {
                    format!("'{}…'", arg.chars().take(24).collect::<String>())
                } else {
                    arg.clone()
                }
            }))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_named_with_long_arguments_cut() {
        let pty = Pty::new(
            Path::new("."),
            "claude",
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--append-system-prompt".to_string(),
                "You are the STEWARD of the plant and this goes on for a while".to_string(),
            ],
        );
        assert_eq!(
            pty.command(),
            "claude --permission-mode bypassPermissions --append-system-prompt 'You are the STEWARD of t…'"
        );
    }

    #[test]
    fn a_real_terminal_echoes_what_is_typed_into_cat() {
        // The real pseudo-terminal, not a fake: this module *is* the terminal.
        let pty = Pty::new(Path::new("."), "cat", vec![]);
        let mut open = pty.spawn(80, 24).expect("a pty with cat in it");
        let mut reader = open.output().expect("the output, once");
        assert!(open.output().is_none(), "handed over once");
        open.write(b"hello steward\n").expect("write");
        let mut buf = [0_u8; 256];
        let mut seen = String::new();
        for _ in 0..20 {
            let n = reader.read(&mut buf).expect("read");
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
            if seen.matches("hello steward").count() >= 2 {
                break;
            }
        }
        // Once echoed by the terminal, once by cat.
        assert!(seen.matches("hello steward").count() >= 2, "{seen:?}");
        open.resize(100, 30).expect("resize");
        open.kill();
    }

    #[test]
    fn a_program_that_does_not_exist_is_an_error_sentence_not_a_panic() {
        let pty = Pty::new(Path::new("."), "no-such-program-for-the-steward", vec![]);
        match pty.spawn(80, 24) {
            Ok(mut open) => {
                // Some platforms only fail once the child is reaped: the
                // program must then be gone immediately.
                let mut reader = open.output().expect("output");
                let mut buf = [0_u8; 64];
                let n = reader.read(&mut buf).unwrap_or(0);
                assert_eq!(n, 0, "nothing runs behind a missing program");
            }
            Err(why) => assert!(why.contains("cannot start"), "{why}"),
        }
    }
}
