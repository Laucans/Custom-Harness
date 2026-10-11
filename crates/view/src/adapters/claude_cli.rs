//! Claude, asked once (`claude -p`), and what Claude Code keeps of the
//! sessions it ran (`~/.claude/projects/<folder>/<session>.jsonl`).

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::ports::{Oneshot, Transcripts};

/// The longest a one-shot answer may take: a summary of a long conversation.
const TIMEOUT: Duration = Duration::from_secs(300);

/// The longest a session at work may take: a mock-up of several pages.
const WORK_TIMEOUT: Duration = Duration::from_mins(40);

/// What a session at work may do: read and write files, nothing that runs.
const WORK_TOOLS: &str = "Read,Write,Edit,Glob,Grep";

/// Whether more than `timeout` went by since `started`.
fn overdue(started: Instant, timeout: Duration) -> bool {
    started.elapsed().saturating_sub(timeout) != Duration::ZERO
}

/// `claude -p`, the prompt on its standard input, no tool, nothing kept.
pub struct ClaudeOneshot {
    claude: PathBuf,
}

impl ClaudeOneshot {
    /// Asks through this `claude` binary.
    #[must_use]
    pub const fn new(claude: PathBuf) -> Self {
        Self { claude }
    }
}

impl Oneshot for ClaudeOneshot {
    fn ask(&self, model: &str, prompt: &str) -> Result<String, String> {
        // An empty folder: a one-shot answer reads nothing of any repository.
        let empty = std::env::temp_dir().join("harness-view-oneshot");
        std::fs::create_dir_all(&empty).map_err(|e| format!("{}: {e}", empty.display()))?;
        self.run(model, prompt, &empty, "", TIMEOUT)
    }

    fn ask_in(&self, model: &str, prompt: &str, dir: &std::path::Path) -> Result<String, String> {
        // `Read` alone: the images and files of `dir` are seen, nothing is
        // written and nothing runs.
        self.run(model, prompt, dir, "Read", TIMEOUT)
    }

    fn work_in(&self, model: &str, prompt: &str, dir: &std::path::Path) -> Result<String, String> {
        self.run(model, prompt, dir, WORK_TOOLS, WORK_TIMEOUT)
    }
}

impl ClaudeOneshot {
    /// `claude -p` in `dir`, with `tools` (empty: none) and the prompt on
    /// its standard input, killed past `timeout`.
    fn run(
        &self,
        model: &str,
        prompt: &str,
        dir: &std::path::Path,
        tools: &str,
        timeout: Duration,
    ) -> Result<String, String> {
        // `dir` may not exist yet (a generation asked before any asset was
        // uploaded there): the process needs a folder to start in, not a
        // promise that one is there.
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut command = Command::new(&self.claude);
        command.current_dir(dir).args([
            "-p",
            "--model",
            model,
            "--output-format",
            "text",
            "--tools",
            tools,
            "--no-session-persistence",
            "--strict-mcp-config",
            "--setting-sources",
            "",
            "--disable-slash-commands",
        ]);
        if !tools.is_empty() {
            command.args(["--allowedTools", tools]);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("claude cannot start: {e}"))?;
        // Written from a thread: a long prompt fills the pipe before claude
        // reads it, and claude's output would fill the other one meanwhile.
        let mut stdin = child.stdin.take().ok_or("claude has no input")?;
        let text = prompt.to_string();
        let writer = std::thread::spawn(move || stdin.write_all(text.as_bytes()));
        let mut stdout = child.stdout.take().ok_or("claude has no output")?;
        let reader = std::thread::spawn(move || {
            let mut out = String::new();
            let _ = stdout.read_to_string(&mut out);
            out
        });
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if overdue(started, timeout) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "claude gave no answer within {}s",
                        timeout.as_secs()
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                Err(e) => return Err(format!("claude cannot be waited on: {e}")),
            }
        };
        let _ = writer.join();
        let out = reader.join().unwrap_or_default();
        if !status.success() {
            let mut err = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                let _ = pipe.read_to_string(&mut err);
            }
            return Err(format!("claude failed ({status}): {}", err.trim()));
        }
        let answer = out.trim().to_string();
        if answer.is_empty() {
            return Err("claude answered nothing".to_string());
        }
        Ok(answer)
    }
}

/// Claude Code's transcripts, one folder per working directory.
pub struct ClaudeTranscripts {
    projects: PathBuf,
}

impl ClaudeTranscripts {
    /// Under `projects` — `~/.claude/projects`.
    #[must_use]
    pub const fn new(projects: PathBuf) -> Self {
        Self { projects }
    }
}

impl Transcripts for ClaudeTranscripts {
    fn read(&self, id: &str) -> Option<String> {
        // An id is a UUID: nothing in it can climb out of the folder.
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            return None;
        }
        let file = format!("{id}.jsonl");
        std::fs::read_dir(&self.projects)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path().join(&file))
            .find(|path| path.is_file())
            .and_then(|path| std::fs::read_to_string(path).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    /// A `claude` that runs `body` (sh), in a folder of its own.
    fn fake(tag: &str, body: &str) -> (PathBuf, ClaudeOneshot) {
        let dir =
            std::env::temp_dir().join(format!("harness-fake-claude-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let bin = dir.join("claude");
        std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).expect("script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("mode");
        (dir, ClaudeOneshot::new(bin))
    }

    #[test]
    fn an_answer_is_what_claude_prints_trimmed() {
        let (dir, claude) = fake("answer", "cat >/dev/null; echo '  the answer  '");
        assert_eq!(claude.ask("m", "q"), Ok("the answer".to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_printed_or_a_failure_is_an_error_in_claudes_words() {
        let (dir, silent) = fake("silent", "cat >/dev/null");
        assert_eq!(
            silent.ask("m", "q"),
            Err("claude answered nothing".to_string())
        );
        let (other, failing) = fake("failing", "cat >/dev/null; echo 'quota' >&2; exit 3");
        let why = failing.ask("m", "q").expect_err("failed");
        assert!(why.contains("quota"), "{why}");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(other);
    }

    #[test]
    fn each_way_of_asking_gives_its_own_tools_and_folder() {
        let (dir, claude) = fake("tools", "cat >/dev/null; pwd; echo \"$@\"");
        let read = claude.ask_in("m", "q", &dir).expect("answered");
        assert!(read.contains("--allowedTools Read"), "{read}");
        assert!(read.contains("harness-fake-claude-tools"), "{read}");
        let work = claude.work_in("m", "q", &dir).expect("answered");
        assert!(
            work.contains("--allowedTools Read,Write,Edit,Glob,Grep"),
            "{work}"
        );
        let bare = claude.ask("m", "q").expect("answered");
        assert!(!bare.contains("--allowedTools"), "{bare}");
        assert!(bare.contains("harness-view-oneshot"), "{bare}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_that_does_not_exist_yet_is_made_before_claude_starts() {
        let (dir, claude) = fake("mkdir", "pwd");
        let missing = dir.join("design").join("moodboard");
        let read = claude.ask_in("m", "q", &missing).expect("answered");
        assert!(read.ends_with("design/moodboard"), "{read}");
        assert!(missing.is_dir());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_claude_past_its_time_is_stopped() {
        let (dir, claude) = fake("slow", "sleep 5; echo late");
        let started = Instant::now();
        let why = claude
            .run("m", "q", &dir, "", Duration::from_millis(300))
            .expect_err("stopped");
        assert!(why.contains("no answer within"), "{why}");
        assert!(started.elapsed() < Duration::from_secs(4));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn overdue_is_strictly_past_the_timeout() {
        let now = Instant::now();
        assert!(!overdue(now, Duration::from_secs(60)));
        assert!(overdue(
            now.checked_sub(Duration::from_secs(2)).expect("a past"),
            Duration::from_secs(1)
        ));
    }

    #[test]
    fn a_transcript_is_found_in_whichever_project_folder_holds_it() {
        let root = std::env::temp_dir().join(format!("harness-transcripts-{}", std::process::id()));
        let id = "0b6c1d2e-0000-4000-8000-000000000001";
        std::fs::create_dir_all(root.join("-Users-x-repo")).expect("dir");
        std::fs::create_dir_all(root.join("-Users-y")).expect("dir");
        std::fs::write(
            root.join("-Users-x-repo").join(format!("{id}.jsonl")),
            "{}\n",
        )
        .expect("written");
        let transcripts = ClaudeTranscripts::new(root.clone());
        assert_eq!(transcripts.read(id).as_deref(), Some("{}\n"));
        assert!(
            transcripts
                .read("0b6c1d2e-0000-4000-8000-000000000002")
                .is_none()
        );
        assert!(transcripts.read("../x").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
