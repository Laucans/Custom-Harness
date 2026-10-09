//! The rate limits, read through the two CLIs the plant already uses:
//! a minimal `claude -p` for the subscription windows, `gh api rate_limit`
//! for GitHub's buckets.
//!
//! The probe is the cheapest session there is: the smallest model, no tool,
//! a one-line system prompt, no setting or MCP server loaded, run from an
//! empty directory — about seven hundred input tokens, a tenth of a cent.

use std::io::Read as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use harness_core::adapters::agent::stream_log;
use harness_core::domain::quota::Reading;

use crate::domain::limits;
use crate::ports::{GithubWindow, Limits};

/// The model the probe asks: the cheapest, since only the event matters.
const PROBE_MODEL: &str = "claude-haiku-4-5-20251001";

/// How long a probe or a `gh` read may take.
const READ_TIMEOUT: Duration = Duration::from_secs(45);

/// The two CLIs, found once.
pub struct CliLimits {
    claude: PathBuf,
}

impl CliLimits {
    /// Reads through `claude` (the binary the steward runs) and `gh`.
    #[must_use]
    pub const fn new(claude: PathBuf) -> Self {
        Self { claude }
    }
}

/// Runs `command` to its end or to [`READ_TIMEOUT`], and returns its stdout.
///
/// The pipes are read once the process is over: both answers are a few
/// kilobytes, well under what a pipe holds, so neither CLI blocks on them.
fn output_of(mut command: Command, what: &str) -> Result<String, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{what} cannot start: {e}"))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > READ_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{what} gave no answer within {}s",
                    READ_TIMEOUT.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(format!("{what} cannot be waited on: {e}")),
        }
    };
    let mut out = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut out);
    }
    if status.success() {
        Ok(out)
    } else {
        let mut err = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut err);
        }
        Err(format!("{what} failed ({status}): {}", err.trim()))
    }
}

impl Limits for CliLimits {
    fn claude(&self) -> Result<Reading, String> {
        let empty = std::env::temp_dir().join("harness-view-probe");
        std::fs::create_dir_all(&empty).map_err(|e| format!("{}: {e}", empty.display()))?;
        let mut command = Command::new(&self.claude);
        command.current_dir(&empty).args([
            "-p",
            "ok",
            "--model",
            PROBE_MODEL,
            "--output-format",
            "stream-json",
            "--verbose",
            "--tools",
            "",
            "--system-prompt",
            "Reply with the single word ok.",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--setting-sources",
            "",
            "--disable-slash-commands",
        ]);
        let out = output_of(command, "the claude probe")?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        stream_log::last_rate_limit(&out, now)
            .ok_or_else(|| "the probe's stream carried no rate-limit event".to_string())
    }

    fn github(&self) -> Result<Vec<GithubWindow>, String> {
        let mut command = Command::new("gh");
        command.args(["api", "rate_limit"]);
        limits::github_windows(&output_of(command, "gh api rate_limit")?)
    }
}
