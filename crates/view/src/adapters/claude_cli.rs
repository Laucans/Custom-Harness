//! Claude, asked once (`claude -p`), and what Claude Code keeps of the
//! sessions it ran (`~/.claude/projects/<folder>/<session>.jsonl`).

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::ports::{Oneshot, Spend, Transcripts, Usage};

/// The longest a one-shot answer may take — a whole mock-up's plan is one
/// such answer: only a process that died silently is cut short.
const TIMEOUT: Duration = Duration::from_mins(40);

/// The longest a session at work may take: a mock-up of several pages.
const WORK_TIMEOUT: Duration = Duration::from_mins(40);

/// What a session at work may do: read and write files, nothing that runs.
const WORK_TOOLS: &str = "Read,Write,Edit,Glob,Grep";

/// Whether more than `timeout` went by since `started`.
fn overdue(started: Instant, timeout: Duration) -> bool {
    started.elapsed().saturating_sub(timeout) != Duration::ZERO
}

/// `read`, `write`, or `ask` (no tool at all) — the mode a call's `tools`
/// string stands for, kept beside its [`Spend`] so the dashboard reads who
/// could touch what.
fn mode_of(tools: &str) -> &'static str {
    if tools.is_empty() {
        "ask"
    } else if tools == WORK_TOOLS {
        "write"
    } else {
        "read"
    }
}

/// Tokens as the turn's `result` event reports them.
#[derive(Debug, Default, Deserialize)]
struct TurnUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

/// What `--output-format stream-json`'s terminal event carries. Every field
/// optional: an event this cannot make sense of is read as carrying none,
/// never as a reason to fail an answer that otherwise came through.
#[derive(Debug, Default, Deserialize)]
struct TurnResult {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    result: String,
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    usage: TurnUsage,
}

/// The **last** `result` event among the stream's lines — read by kind, not
/// position, the same reason the dev loop's own parser does: a format that
/// adds something after it must not break the answer.
fn result_of(stdout: &str) -> Option<TurnResult> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<TurnResult>(line.trim()).ok())
        .rfind(|event| event.kind == "result")
}

/// `claude -p`, the prompt on its standard input, no tool, nothing kept —
/// except what it cost, logged through `usage` under `who`'s name.
pub struct ClaudeOneshot {
    claude: PathBuf,
    who: String,
    usage: Option<Arc<dyn Usage>>,
}

impl ClaudeOneshot {
    /// Asks through this `claude` binary. `who` names the asker on the
    /// dashboard (`creation`, `kit`, `mf`, `notes`, `units`); `usage` is
    /// where what it cost is kept — `None` logs nothing.
    #[must_use]
    pub fn new(claude: PathBuf, who: &str, usage: Option<Arc<dyn Usage>>) -> Self {
        Self {
            claude,
            who: who.to_string(),
            usage,
        }
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
    /// Keeps what a turn cost; never fails the call it rides along with.
    fn log(&self, model: &str, tools: &str, event: &TurnResult) {
        let Some(usage) = &self.usage else { return };
        let at = jiff::Timestamp::now()
            .strftime("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let spend = Spend {
            at,
            who: self.who.clone(),
            model: model.to_string(),
            mode: mode_of(tools).to_string(),
            cost_usd: event.total_cost_usd,
            duration_ms: event.duration_ms,
            input_tokens: event.usage.input_tokens,
            output_tokens: event.usage.output_tokens,
        };
        if let Err(why) = usage.log(&spend) {
            tracing::warn!("what claude cost was not kept: {why}");
        }
    }

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
            "stream-json",
            // Required by the CLI alongside `-p --output-format stream-json`,
            // and what makes the turn's `result` event carry `total_cost_usd`.
            "--verbose",
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
        let Some(event) = result_of(&out) else {
            return Err("claude answered nothing".to_string());
        };
        self.log(model, tools, &event);
        let answer = event.result.trim().to_string();
        if event.is_error {
            return Err(if answer.is_empty() {
                "claude reported an error with no detail".to_string()
            } else {
                answer
            });
        }
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
    /// Reads transcripts under `projects` (`~/.claude/projects`).
    #[must_use]
    pub const fn new(projects: PathBuf) -> Self {
        Self { projects }
    }
}

impl Transcripts for ClaudeTranscripts {
    fn read(&self, id: &str) -> Option<String> {
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
    use std::sync::Mutex;

    /// A `claude` that runs `body` (sh), in a folder of its own, with `kept`
    /// collecting what the calls logged.
    fn fake(tag: &str, body: &str) -> (PathBuf, ClaudeOneshot) {
        fake_logged(tag, body, None)
    }

    fn fake_logged(
        tag: &str,
        body: &str,
        usage: Option<Arc<dyn Usage>>,
    ) -> (PathBuf, ClaudeOneshot) {
        let dir =
            std::env::temp_dir().join(format!("harness-fake-claude-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let bin = dir.join("claude");
        // Written by a child shell: no writable handle on the script ever
        // lives in this process, so a test forking meanwhile cannot hold one
        // and make its exec fail with "Text file busy".
        let written = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                "printf '%s\\n' \"$2\" > \"$1\" && chmod 755 \"$1\"",
                "sh",
            ])
            .arg(&bin)
            .arg(format!("#!/bin/sh\n{body}"))
            .status()
            .expect("script");
        assert!(written.success(), "the script is written");
        (dir, ClaudeOneshot::new(bin, "test", usage))
    }

    /// A stream-json `result` line, success or not, with a cost.
    fn result_line(text: &str, cost: f64, is_error: bool) -> String {
        format!(
            r#"{{"type":"result","is_error":{is_error},"result":"{text}","total_cost_usd":{cost},"duration_ms":42,"usage":{{"input_tokens":10,"output_tokens":5}}}}"#
        )
    }

    /// A `Usage` kept in memory, for a test to read back.
    #[derive(Default)]
    struct Logged(Mutex<Vec<Spend>>);

    impl Usage for Logged {
        fn log(&self, spend: &Spend) -> Result<(), String> {
            self.0.lock().expect("held").push(spend.clone());
            Ok(())
        }

        fn all(&self) -> Result<Vec<Spend>, String> {
            Ok(self.0.lock().expect("held").clone())
        }
    }

    #[test]
    fn an_answer_is_the_results_text_trimmed() {
        let line = result_line("  the answer  ", 0.01, false);
        let (dir, claude) = fake("answer", &format!("cat >/dev/null; echo '{line}'"));
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
        let line = result_line("", 0.0, true);
        let (bad, errored) = fake("errored", &format!("cat >/dev/null; echo '{line}'"));
        assert_eq!(
            errored.ask("m", "q"),
            Err("claude reported an error with no detail".to_string())
        );
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(other);
        let _ = std::fs::remove_dir_all(bad);
    }

    #[test]
    fn each_way_of_asking_gives_its_own_tools_and_folder() {
        // Built by the shell at run time: embedding `$(pwd)` in a Rust string
        // destined for a double-quoted `echo` only invites quoting trouble.
        let body = r#"cat >/dev/null
printf '{"type":"result","is_error":false,"result":"%s [%s]","total_cost_usd":0.0}' "$(pwd)" "$*""#;
        let (dir, claude) = fake("tools", body);
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
        let body = r#"cat >/dev/null
printf '{"type":"result","is_error":false,"result":"%s","total_cost_usd":0.0}' "$(pwd)""#;
        let (dir, claude) = fake("mkdir", body);
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
    fn mode_names_match_the_tools_a_call_was_given() {
        assert_eq!(mode_of(""), "ask");
        assert_eq!(mode_of("Read"), "read");
        assert_eq!(mode_of(WORK_TOOLS), "write");
    }

    #[test]
    fn a_successful_call_logs_its_cost_and_tokens_under_the_asker_who_built_it() {
        let kept: Arc<dyn Usage> = Arc::new(Logged::default());
        let line = result_line("ok", 0.0321, false);
        let (dir, claude) = fake_logged(
            "logs",
            &format!("cat >/dev/null; echo '{line}'"),
            Some(Arc::clone(&kept)),
        );
        claude.ask_in("opus", "q", &dir).expect("answered");
        let spends = kept.all().expect("read");
        assert_eq!(spends.len(), 1);
        assert_eq!(spends[0].who, "test");
        assert_eq!(spends[0].model, "opus");
        assert_eq!(spends[0].mode, "read");
        assert_eq!(spends[0].cost_usd, Some(0.0321));
        assert_eq!(spends[0].duration_ms, Some(42));
        assert_eq!(spends[0].input_tokens, Some(10));
        assert_eq!(spends[0].output_tokens, Some(5));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_usage_store_nothing_is_logged_and_the_answer_still_comes_through() {
        let line = result_line("ok", 0.01, false);
        let (dir, claude) = fake("no-store", &format!("cat >/dev/null; echo '{line}'"));
        assert_eq!(claude.ask("m", "q"), Ok("ok".to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_call_that_cannot_be_parsed_logs_nothing_and_still_errs() {
        let kept: Arc<dyn Usage> = Arc::new(Logged::default());
        let (dir, claude) = fake_logged(
            "unparseable",
            "cat >/dev/null; echo 'not json'",
            Some(Arc::clone(&kept)),
        );
        assert_eq!(
            claude.ask("m", "q"),
            Err("claude answered nothing".to_string())
        );
        assert!(kept.all().expect("read").is_empty(), "expected nothing");
        let _ = std::fs::remove_dir_all(dir);
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
        assert_eq!(transcripts.read(id), Some("{}\n".to_string()));
        assert_eq!(transcripts.read("nope"), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
