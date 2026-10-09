//! Claude, asked once (`claude -p`), and what Claude Code keeps of the
//! sessions it ran (`~/.claude/projects/<folder>/<session>.jsonl`).

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::ports::{Oneshot, Transcripts};

/// The longest a one-shot answer may take: a summary of a long conversation.
const TIMEOUT: Duration = Duration::from_secs(300);

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
        let mut child = Command::new(&self.claude)
            .current_dir(&empty)
            .args([
                "-p",
                "--model",
                model,
                "--output-format",
                "text",
                "--tools",
                "",
                "--no-session-persistence",
                "--strict-mcp-config",
                "--setting-sources",
                "",
                "--disable-slash-commands",
            ])
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
                Ok(None) if started.elapsed() > TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "claude gave no answer within {}s",
                        TIMEOUT.as_secs()
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
