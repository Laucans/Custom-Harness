//! Approach C: one process per action, stitched together via `--resume`.
//!
//! Each turn is `claude -p --output-format stream-json`. The first call creates the
//! conversation; subsequent calls resume it via `--resume <session_id>`, with
//! the identifier learned from the first response rather than imposed — this is
//! the pattern the `headless` docs describe, and avoids wondering what would
//! happen with a `--session-id` already in use.
//!
//! "Open session" is thus an assumed fiction: continuity lives on disk, in the
//! transcript, not in a live process. This is what `docs/SESSION-CARRIER.md`
//! chose — minimal code for exact cost and free turn boundaries, with
//! approach A (tmux) as a target if attachability becomes necessary.
//!
//! # Cost: read `total_cost_usd` without summing blindly
//!
//! **Since Claude Code v2.1.277**, a call that resumes a session returns the
//! total of **the entire conversation**, including costs from prior calls. The
//! cost of a stage is therefore the value of the **last** turn, and summing
//! turns would double-count. Before that version, each call returned only its
//! own, and you had to sum.
//!
//! This adapter does not decide: it reports faithfully what the turn said, in
//! [`Spend::cost_usd`](crate::domain::Spend). It is up to the spending ledger
//! — which does not yet exist —
//! to accumulate, and **it must verify the version** rather than assume. A
//! preflight gate on `claude --version` costs one local call; false cost data
//! is invisible.
//!
//! `total_cost_usd` is moreover a **client-side estimate**, calculated from an
//! embedded price table, not billing data. Good for budgeting, never for
//! billing anyone.
//!
//! # Why the stream, and not one object at the end
//!
//! `--output-format json` answers once, when the turn is over. A `/code` turn
//! runs for tens of minutes, so the harness had nothing to show for all that
//! time and no way to tell work from a hang. `stream-json` emits one event per
//! action; each is rendered by
//! [`stream_log`](crate::adapters::agent::stream_log) and appended to
//! the run's own folder as it arrives, so `tail -f` follows a live session.
//!
//! # What a session leaves behind
//!
//! Three files in the run folder, because three different questions get asked
//! of a session after the fact:
//!
//! - `session.log` — the events, rendered one line each, timestamped by how
//!   far into the turn they happened. What a human follows live.
//! - `stream.jsonl` — the same events, **verbatim**. The rendering keeps what
//!   is worth watching and drops the rest; this keeps everything, for the
//!   question nobody thought to ask when the renderer was written.
//! - `prompts.md` — every prompt actually sent, in order. A session's answer
//!   can only be judged against what it was asked, and until this existed the
//!   question was reconstructed by hand from the code.
//!
//! The turn's own result is the `result` event, the last one — the same object
//! the single-shot format returned, which is why nothing about cost or
//! classification changed here.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::adapters::agent::stream_log;
use crate::adapters::shell::process;
use crate::domain::{Halt, Outcome, Spend, Tokens, markers};
use crate::ports::agent::{Reply, Session, SessionFactory, SessionSpec};
use crate::ports::shell::process::{Ran, last_line};

/// The called binary. Named here so a test can read it.
const BINARY: &str = "claude";

/// How long one turn may run before it's treated as hung.
///
/// A high-effort session can run long — this is deliberately far more
/// generous than `adapters::shell::process`'s 60s, which is for a `git`/`gh`
/// round-trip, not a paid turn.
const TIMEOUT: Duration = Duration::from_mins(30);

/// Phrases that mark a failure as an exhausted quota.
///
/// In one place: this is a heuristic on error text, so it will drift, and
/// when it drifts we want one place to fix. Searched only in a turn
/// **already failing** — a session that succeeds while mentioning "rate limit"
/// is not a quota.
///
/// `session limit` is here because it drifted once already: the CLI answered
/// "You've hit your session limit · resets 4pm", none of the other phrases
/// matched, and the turn was filed as a failure. A quota read as a failure is
/// not cosmetic — it is a session that never ran being counted against the
/// prompt it never sent (see
/// [`breaker`](crate::domain::breaker)).
const QUOTA_PHRASES: [&str; 5] = [
    "usage limit",
    "session limit",
    "rate limit",
    "quota",
    "too many requests",
];

/// The HTTP status the API returns for a refused-for-now request.
const TOO_MANY_REQUESTS: u16 = 429;

/// What we read from the turn's `result` event.
///
/// All fields are intentionally optional: Claude Code adds them over versions,
/// and an unknown field must not fail a turn that went well.
#[derive(Debug, Deserialize)]
struct CliResult {
    /// The event kind. Empty when reading a single-object response, which is
    /// why [`result_line`] accepts an absent one.
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    subtype: String,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    result: String,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default)]
    num_turns: Option<u32>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    usage: CliUsage,
    /// The HTTP status behind the failure, when the CLI reports one.
    ///
    /// Structured, so it is read before the prose: a status is what the API
    /// actually said, where [`QUOTA_PHRASES`] is a guess at how it was
    /// phrased this month.
    #[serde(default)]
    api_error_status: Option<u16>,
}

/// Tokens as reported by the `result` message.
///
/// **Undercounts subagents**: the docs are explicit, `usage` covers only the
/// main loop while `total_cost_usd` includes subagents. The `code` stage
/// launches them, so those tokens are a floor, not a total. This is the cost
/// to read for budgeting, not the tokens.
// Field names are from the API, not ours: renaming to please
// `struct_field_names` would lie to `Deserialize`.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Default, Deserialize)]
struct CliUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

/// A conversation conducted via successive calls to the `claude` binary.
pub struct ClaudeCli {
    cwd: PathBuf,
    model: String,
    effort: String,
    permission_mode: String,
    /// Learned from the first turn's response. `None` = conversation does not
    /// yet exist, so no `--resume` to pass.
    session_id: Option<String>,
    /// The run's folder, where this session's traces are appended.
    /// `None` = nothing kept.
    traces: Option<PathBuf>,
    /// Which turn of this session the next `ask` is, counted from 1.
    turn: u32,
}

impl ClaudeCli {
    /// A conversation that has not yet taken place.
    ///
    /// `traces` is the run's folder, where this session appends what it did;
    /// `None` keeps nothing.
    #[must_use]
    pub fn new(
        cwd: PathBuf,
        spec: &SessionSpec,
        permission_mode: &str,
        traces: Option<PathBuf>,
    ) -> Self {
        Self {
            cwd,
            model: spec.model.clone(),
            effort: spec.effort.clone(),
            permission_mode: permission_mode.to_string(),
            session_id: None,
            traces,
            turn: 0,
        }
    }

    /// The rendered log — one readable line per event.
    fn rendered_log(&self) -> Option<PathBuf> {
        self.traces.as_ref().map(|dir| dir.join("session.log"))
    }

    /// The verbatim stream, one JSON object per line.
    fn raw_log(&self) -> Option<PathBuf> {
        self.traces.as_ref().map(|dir| dir.join("stream.jsonl"))
    }

    /// Every prompt this run sent, in order.
    fn prompt_log(&self) -> Option<PathBuf> {
        self.traces.as_ref().map(|dir| dir.join("prompts.md"))
    }

    /// Appends these lines to the rendered log, if there is one.
    fn trace(&self, lines: &[String]) {
        append(self.rendered_log().as_deref(), lines);
    }

    /// The arguments for this turn.
    ///
    /// Pure, and separated from the call: this is the part that tests without
    /// spending a cent, and where the only real logic lives — whether to pass
    /// `--resume` or not.
    ///
    /// The prompt travels as an argument rather than stdin. A few kilobytes
    /// stay well under `ARG_MAX`; if a preamble became huge, that would be
    /// the time to pass it via stdin.
    ///
    /// It comes last, after `--`: a prompt that starts with `-` (the repo map
    /// opens with `---`) would otherwise be parsed as an unknown option.
    fn argv(&self, prompt: &str) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            // Required by the CLI alongside `-p --output-format stream-json`,
            // and it is also what makes the events worth logging.
            "--verbose".to_string(),
            "--model".to_string(),
            self.model.clone(),
            "--effort".to_string(),
            self.effort.clone(),
            "--permission-mode".to_string(),
            self.permission_mode.clone(),
        ];
        if let Some(id) = &self.session_id {
            args.push("--resume".to_string());
            args.push(id.clone());
        }
        args.push("--".to_string());
        args.push(prompt.to_string());
        args
    }
}

/// How to classify a failed turn.
///
/// A quota is neither "repaired" nor "abandoned": it is the same work to
/// restart later, unchanged. Distinguishing it from a failure prevents
/// replaying a session that had nothing broken.
fn halt_for(subtype: &str, text: &str, status: Option<u16>) -> Halt {
    if status == Some(TOO_MANY_REQUESTS) {
        return Halt::Quota(text.to_string());
    }
    let haystack = format!("{subtype} {text}").to_lowercase();
    if QUOTA_PHRASES.iter().any(|phrase| haystack.contains(phrase)) {
        return Halt::Quota(text.to_string());
    }
    Halt::Failed(text.to_string())
}

/// The turn's own answer, among the events of the stream.
///
/// The **last** `result` event, read as a whole object rather than looked for
/// by position: the stream puts it last today, and a format that one day adds
/// something after it must not break the cost of a turn.
///
/// A whole response that is one object is read as that object, so the
/// single-shot `--output-format json` form — pretty-printed or not — reads
/// exactly as it did before the stream.
fn result_line(stdout: &str) -> Option<CliResult> {
    if let Ok(single) = serde_json::from_str::<CliResult>(stdout.trim()) {
        return Some(single);
    }
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<CliResult>(line.trim()).ok())
        .rfind(|event| event.kind == "result")
}

/// Now, in seconds since the epoch, for stamping a quota reading.
fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// What a turn returned, plus the conversation identifier.
///
/// Pure: everything after the binary call tests by passing it a string.
///
/// # Errors
///
/// - [`Halt::Failed`] if no `result` event is readable, or if the turn
///   succeeded without returning anything — an empty turn is not usable by
///   the next action;
/// - [`Halt::Quota`] or [`Halt::Failed`] depending on the status and what the
///   failure says, see [`halt_for`].
fn parse(stdout: &str, now: u64) -> Outcome<(Reply, String)> {
    let Some(parsed) = result_line(stdout) else {
        return Err(Halt::Failed(format!(
            "no readable result event from {BINARY} — it said: {}",
            last_line(stdout)
        )));
    };

    if parsed.is_error {
        return Err(halt_for(
            &parsed.subtype,
            &parsed.result,
            parsed.api_error_status,
        ));
    }
    if parsed.result.trim().is_empty() {
        return Err(Halt::Failed(format!(
            "{BINARY} succeeded without returning anything (subtype {:?})",
            parsed.subtype
        )));
    }

    let spend = Spend {
        cost_usd: parsed.total_cost_usd,
        turns: parsed.num_turns,
        duration_ms: parsed.duration_ms,
        tokens: Tokens {
            input: parsed.usage.input_tokens,
            output: parsed.usage.output_tokens,
            cache_read: parsed.usage.cache_read_input_tokens,
            cache_write: parsed.usage.cache_creation_input_tokens,
        },
        session: (!parsed.session_id.is_empty()).then(|| parsed.session_id.clone()),
        // Translating the carrier's own shape into the domain's: the only place
        // that knows `rate_limit_event` exists. A different carrier translates
        // whatever *it* reports into the same `Reading`, and nothing downstream
        // changes.
        quota: stream_log::last_rate_limit(stdout, now),
    };
    let reply = Reply {
        stop_line: markers::stop_line(&parsed.result),
        text: parsed.result,
        spend,
    };
    Ok((reply, parsed.session_id))
}

/// What a failed process leaves to read.
///
/// The `result` event first: a non-zero exit still emits it, and it carries
/// both the reason and the HTTP status — which classify the failure far better
/// than an exit code does.
///
/// Then `stderr`: when `claude` rejects a flag, the reason is there and no
/// event was ever emitted.
///
/// Only **the last line** of stdout as a last resort. The stream is hundreds
/// of lines of JSON, and putting all of it into a `Halt` message floods the
/// run log with the transcript — which it did, once.
fn failed_process(out: &Ran) -> Halt {
    if let Some(event) = result_line(&out.stdout) {
        return halt_for(&event.subtype, &event.result, event.api_error_status);
    }
    let stderr = out.stderr.trim();
    let said = if stderr.is_empty() {
        last_line(&out.stdout)
    } else {
        stderr.to_string()
    };
    let code = out
        .code
        .map_or_else(|| "killed by signal".to_string(), |c| format!("code {c}"));
    // No structured status to read here: the process died without saying one.
    halt_for("", &format!("{BINARY} stopped ({code}): {said}"), None)
}

/// The skill a prompt opens with, which is what names the stage in the log.
///
/// The first line of the prompt by construction (`prompts::build` puts the
/// lead there) — but only a prompt built that way opens with a `/command`. A
/// one-shot prompt (the repo map, the human advice) opens with prose, and its
/// first line made headers like `── turn 1 --- REPO MAP (established once …`.
/// Those read as a quotation, not as a name, so prose is cut to its first few
/// words.
fn lead_of(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or_default().trim();
    if first.starts_with('/') {
        return first.to_string();
    }
    let words: Vec<&str> = first
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .take(5)
        .collect();
    if words.is_empty() {
        return "(prose)".to_string();
    }
    format!("{}…", words.join(" "))
}

/// How long into the turn, as `12m04s`.
///
/// Elapsed rather than a wall clock: `harness-core` reads no clock, and for
/// the question this log answers — where did it stall — the gap between two
/// lines is the signal, not the hour.
fn since(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{:>3}m{:02}s", seconds / 60, seconds % 60)
}

/// Each line, prefixed by how far into the turn it happened.
fn stamped(lines: &[String], elapsed: std::time::Duration) -> Vec<String> {
    let at = since(elapsed);
    lines.iter().map(|line| format!("[{at}] {line}")).collect()
}

/// Appends these lines to `path`, and says nothing if it cannot.
///
/// Opened and closed per call, unbuffered: a log that is only readable once
/// the turn ends is the thing this exists to replace, and a buffered writer
/// would do exactly that. A few hundred lines per turn makes the cost of the
/// open irrelevant next to a paid session.
///
/// A failure to log is deliberately silent. The log is an observation of the
/// run, and losing the observation must never lose the run.
fn append(path: Option<&Path>, lines: &[String]) {
    use std::io::Write as _;
    let Some(path) = path else { return };
    if lines.is_empty() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = file.write_all(lines.join("\n").as_bytes());
    let _ = file.write_all(b"\n");
}

#[async_trait(?Send)]
impl Session for ClaudeCli {
    async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
        self.ask_within(prompt, TIMEOUT).await
    }
}

impl ClaudeCli {
    /// [`Session::ask`], with the deadline as a parameter — the seam a test
    /// uses to prove a hung turn is a failure without waiting out the real,
    /// 30-minute [`TIMEOUT`].
    async fn ask_within(&mut self, prompt: &str, timeout: Duration) -> Outcome<Reply> {
        let started = std::time::Instant::now();
        self.turn = self.turn.saturating_add(1);
        let lead = lead_of(prompt);
        self.trace(&[format!("── turn {} {lead} ──", self.turn)]);
        // The prompt before the call, not after: a turn that never returns is
        // exactly the one whose question we need to read.
        append(
            self.prompt_log().as_deref(),
            &[format!(
                "## turn {} — {lead} ({}/{})\n\n```\n{prompt}\n```\n",
                self.turn, self.model, self.effort
            )],
        );
        let out = {
            let rendered_to = self.rendered_log();
            let raw_to = self.raw_log();
            let mut watch = |stream: process::Stream, line: &str| {
                let rendered = match stream {
                    process::Stream::Out => {
                        // Verbatim first: the rendering is a reading, and a
                        // reading can be wrong in a way the bytes are not.
                        append(raw_to.as_deref(), &[line.to_string()]);
                        stream_log::render(line)
                    }
                    // Not an event of the stream: whatever the binary itself
                    // had to say, and the reason a turn produced none.
                    process::Stream::Err => vec![format!("! {line}")],
                };
                append(
                    rendered_to.as_deref(),
                    &stamped(&rendered, started.elapsed()),
                );
            };
            process::run_streaming(BINARY, &self.argv(prompt), &self.cwd, timeout, &mut watch).await
        };
        let out = match out {
            Ok(out) => out,
            Err(halt) => {
                self.trace(&[format!("! {}", halt.reason())]);
                return Err(halt);
            }
        };

        if !out.ok() {
            return Err(failed_process(&out));
        }

        let (reply, session_id) = parse(&out.stdout, now_seconds())?;
        // The first turn learns the identifier; later turns return it as-is,
        // and rewriting it costs nothing.
        if !session_id.is_empty() {
            self.session_id = Some(session_id);
        }
        Ok(reply)
    }
}

/// Opens `claude` conversations in a given directory.
pub struct ClaudeCliFactory {
    cwd: PathBuf,
    permission_mode: String,
    traces: Option<PathBuf>,
}

impl ClaudeCliFactory {
    /// A factory that runs each session in `cwd`.
    ///
    /// `cwd` is the root of the run's checkout — the clone, not the repo
    /// from which the run is launched. `traces` is the run's folder, where
    /// every session of this run appends what it did, live.
    #[must_use]
    pub fn new(cwd: &Path, permission_mode: &str, traces: Option<PathBuf>) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            permission_mode: permission_mode.to_string(),
            traces,
        }
    }
}

#[async_trait(?Send)]
impl SessionFactory for ClaudeCliFactory {
    /// Under approach C, opening launches nothing.
    ///
    /// The conversation is born at the first `ask`. This is the assumed trade-off
    /// of the choice: nothing to tear down, nothing that leaks if a stage dies
    /// along the way.
    async fn open(&self, spec: &SessionSpec) -> Outcome<Box<dyn Session>> {
        Ok(Box::new(ClaudeCli::new(
            self.cwd.clone(),
            spec,
            &self.permission_mode,
            self.traces.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli() -> ClaudeCli {
        ClaudeCli::new(
            PathBuf::from("/tmp/workspace"),
            &SessionSpec {
                model: "opus".to_string(),
                effort: "high".to_string(),
            },
            "bypassPermissions",
            None,
        )
    }

    // --- what names a turn in the log ---------------------------------------

    #[test]
    fn a_command_prompt_is_named_by_its_command() {
        assert_eq!(lead_of("/code\n\nthe rest"), "/code");
    }

    #[test]
    fn a_prose_prompt_is_cut_rather_than_quoted_whole() {
        // The header this fixes: `── turn 1 --- REPO MAP (established once for
        // this run) --- ──`, which read as a quotation, not as a name.
        let said = lead_of("--- REPO MAP (established once for this run) ---\nbody");
        assert_eq!(said, "REPO MAP (established once for…");
    }

    #[test]
    fn a_prompt_opening_on_nothing_still_has_a_name() {
        assert_eq!(lead_of(""), "(prose)");
        assert_eq!(lead_of("--- ---\nbody"), "(prose)");
    }

    // --- argv ---------------------------------------------------------------

    #[test]
    fn the_first_turn_carries_no_resume() {
        let args = cli().argv("/business-analyst");
        assert!(!args.contains(&"--resume".to_string()));
        assert_eq!(args[0], "-p");
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args[args.len() - 1], "/business-analyst");
    }

    #[test]
    fn a_prompt_starting_with_dashes_is_never_read_as_an_option() {
        let args = cli().argv("--- REPO MAP ---");
        let at = args.iter().position(|a| a == "--").expect("--");
        assert_eq!(args[at + 1], "--- REPO MAP ---");
        assert!(!args[..at].contains(&"--- REPO MAP ---".to_string()));
    }

    #[test]
    fn a_later_turn_resumes_the_session_it_learned() {
        let mut c = cli();
        c.session_id = Some("abc-123".to_string());
        let args = c.argv("/code");
        let at = args.iter().position(|a| a == "--resume").expect("--resume");
        assert_eq!(args[at + 1], "abc-123");
    }

    #[test]
    fn model_effort_and_permission_mode_all_reach_the_command_line() {
        let args = cli().argv("x").join(" ");
        assert!(args.contains("--model opus"));
        assert!(args.contains("--effort high"));
        assert!(args.contains("--permission-mode bypassPermissions"));
        assert!(args.contains("--output-format stream-json"));
        // The CLI refuses the stream without it.
        assert!(args.contains("--verbose"));
    }

    // --- parse --------------------------------------------------------------

    /// The documented form of the JSON from `--output-format json`, including
    /// unknown fields: the parser must ignore them, not choke on them.
    const SUCCESS: &str = r#"{
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "duration_ms": 45000,
        "duration_api_ms": 2300,
        "num_turns": 3,
        "result": "voici ce que j'ai fait\nAGENT_LOOP_OK: spec écrit",
        "session_id": "sess-42",
        "total_cost_usd": 0.1234,
        "usage": { "input_tokens": 100, "output_tokens": 20 },
        "modelUsage": { "claude-opus-5": { "costUSD": 0.1234 } },
        "un_champ_que_cette_version_ne_connait_pas": true
    }"#;

    #[test]
    fn a_successful_turn_yields_its_text_cost_and_session() {
        let (reply, session) = parse(SUCCESS, 0).expect("parse");
        assert_eq!(session, "sess-42");
        assert!((reply.spend.cost_usd.expect("cost") - 0.1234).abs() < f64::EPSILON);
        assert_eq!(reply.spend.turns, Some(3));
        assert_eq!(reply.spend.duration_ms, Some(45000));
        assert_eq!(reply.spend.tokens.input, Some(100));
        assert_eq!(reply.spend.tokens.output, Some(20));
        assert_eq!(reply.spend.session.as_deref(), Some("sess-42"));
        assert!(reply.text.contains("voici ce que j'ai fait"));
        // AGENT_LOOP_OK is not a stop marker.
        assert!(reply.stop_line.is_none());
    }

    #[test]
    fn a_stop_marker_in_the_text_becomes_the_stop_line() {
        let json = r#"{"is_error":false,"result":"AGENT_LOOP_STOP: le SPEC est vide",
                       "session_id":"s","total_cost_usd":0.01}"#;
        let (reply, _) = parse(json, 0).expect("parse");
        assert_eq!(
            reply.stop_line.as_deref(),
            Some("AGENT_LOOP_STOP: le SPEC est vide")
        );
    }

    #[test]
    fn a_turn_with_no_cost_field_parses_and_reports_none() {
        // A carrier or version that does not return cost: the turn stays valid.
        // That is why `Reply::cost` is an Option.
        let json = r#"{"is_error":false,"result":"fait","session_id":"s"}"#;
        let (reply, _) = parse(json, 0).expect("parse");
        assert!(reply.spend.cost_usd.is_none());
        // Nothing observed, especially not zeros: a ledger must be able to
        // write "not measured" rather than a free session.
        assert!(reply.spend.is_blind());
    }

    #[test]
    fn an_empty_result_is_a_failure_not_an_empty_success() {
        let json = r#"{"is_error":false,"result":"   ","session_id":"s"}"#;
        let err = parse(json, 0).expect_err("doit échouer");
        assert!(matches!(err, Halt::Failed(_)));
    }

    #[test]
    fn unreadable_output_fails_rather_than_panicking() {
        let err = parse("ceci n'est pas du json", 0).expect_err("doit échouer");
        assert!(matches!(err, Halt::Failed(_)));
    }

    // --- classement des échecs ---------------------------------------------

    #[test]
    fn an_exhausted_window_is_a_quota_not_a_failure() {
        let json = r#"{"is_error":true,"subtype":"error_during_execution",
                       "result":"Claude usage limit reached","session_id":"s"}"#;
        let err = parse(json, 0).expect_err("must fail");
        assert!(
            matches!(err, Halt::Quota(_)),
            "a quota restarts the same work later; a failure replays it"
        );
    }

    #[test]
    fn the_session_limit_wording_is_a_quota_too() {
        // The real answer, which no earlier phrase matched: it was filed as a
        // failure, which charges it against the prompt that never ran.
        let json = r#"{"is_error":true,"subtype":"success","api_error_status":429,
                       "result":"You've hit your session limit · resets 4pm (America/Toronto)",
                       "session_id":"s"}"#;
        let err = parse(json, 0).expect_err("must fail");
        assert!(matches!(err, Halt::Quota(_)), "{err}");
    }

    #[test]
    fn the_status_is_read_before_the_prose() {
        // 429 is what the API said; the phrase list is a guess at how it was
        // worded this month.
        let json = r#"{"is_error":true,"subtype":"success","api_error_status":429,
                       "result":"refused for now","session_id":"s"}"#;
        assert!(matches!(
            parse(json, 0).expect_err("must fail"),
            Halt::Quota(_)
        ));
    }

    #[test]
    fn another_status_stays_a_failure() {
        let json = r#"{"is_error":true,"subtype":"success","api_error_status":500,
                       "result":"server exploded","session_id":"s"}"#;
        assert!(matches!(
            parse(json, 0).expect_err("must fail"),
            Halt::Failed(_)
        ));
    }

    #[test]
    fn any_other_error_subtype_is_a_plain_failure() {
        let json = r#"{"is_error":true,"subtype":"error_during_execution",
                       "result":"the tool crashed","session_id":"s"}"#;
        assert!(matches!(
            parse(json, 0).expect_err("must fail"),
            Halt::Failed(_)
        ));
    }

    #[test]
    fn a_dead_process_that_printed_its_session_limit_is_still_a_quota() {
        // The shape actually seen: the CLI exits 1 and prints the result JSON
        // on stderr, so there is no parsed status — only the prose.
        let said = r#"claude stopped (code 1): {"api_error_status":429,
                      "result":"You've hit your session limit · resets 4pm"}"#;
        assert!(matches!(halt_for("", said, None), Halt::Quota(_)));
    }

    #[test]
    fn a_failed_process_is_classified_by_its_result_event_not_its_transcript() {
        // The regression this prevents: the whole stream-json transcript —
        // hundreds of lines — ended up inside one Halt message in the run log.
        let out = Ran {
            code: Some(1),
            stdout: [
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}"#,
                r#"{"type":"result","is_error":true,"api_error_status":429,"result":"You've hit your session limit"}"#,
            ]
            .join("\n"),
            stderr: String::new(),
        };
        let halt = failed_process(&out);
        assert!(matches!(halt, Halt::Quota(_)), "{halt}");
        assert_eq!(halt.reason(), "You've hit your session limit");
        assert!(
            !halt.reason().contains("\"type\""),
            "no transcript leaks in"
        );
    }

    #[test]
    fn without_any_event_the_stderr_is_what_explains_a_dead_process() {
        let out = Ran {
            code: Some(2),
            stdout: String::new(),
            stderr: "error: unknown option '--nope'".to_string(),
        };
        let halt = failed_process(&out);
        assert!(matches!(halt, Halt::Failed(_)));
        assert!(halt.reason().contains("--nope"), "{}", halt.reason());
    }

    #[test]
    fn quota_phrases_are_matched_case_insensitively() {
        assert!(matches!(
            halt_for("", "Rate Limit Exceeded", None),
            Halt::Quota(_)
        ));
    }

    // --- la fabrique --------------------------------------------------------

    #[tokio::test]
    async fn a_hung_turn_is_a_failure_not_an_infinite_wait() {
        // `BINARY` is the const "claude", possibly absent on this machine —
        // either way `ask_within`'s 1ms deadline must win the race against
        // launching it, so the outcome is a failure regardless of whether
        // `claude` exists here.
        let mut session = cli();
        let err = session
            .ask_within("x", Duration::from_millis(1))
            .await
            .expect_err("a 1ms deadline must not succeed");
        assert!(matches!(err, Halt::Failed(_)));
    }

    #[tokio::test]
    async fn opening_a_session_spawns_nothing() {
        // Under C, `open` is free: the conversation is born at the first `ask`.
        // This test passes without any `claude` binary existing.
        let factory = ClaudeCliFactory::new(Path::new("/tmp/workspace"), "bypassPermissions", None);
        let spec = SessionSpec {
            model: "sonnet".to_string(),
            effort: "high".to_string(),
        };
        assert!(factory.open(&spec).await.is_ok());
    }

    /// The only test that actually spends money and touches the real binary.
    /// Ignored by default — the hermetic tests above run against a frozen
    /// response, and a frozen response can lie when Claude Code renames a field.
    /// This one is here to close that gap, on demand:
    ///
    /// ```text
    /// cargo test -p harness-core -- --ignored live_
    /// ```
    #[tokio::test]
    #[ignore = "calls the real claude binary and spends quota"]
    async fn live_two_turns_share_one_session() {
        let factory = ClaudeCliFactory::new(Path::new("."), "bypassPermissions", None);
        let spec = SessionSpec {
            model: "haiku".to_string(),
            effort: "low".to_string(),
        };
        let mut session = factory.open(&spec).await.expect("open");

        let first = session
            .ask("Réponds exactement: un")
            .await
            .expect("1st turn");
        assert!(
            first.spend.cost_usd.is_some(),
            "the first turn must return a cost"
        );

        // The second turn must see the first: if it does not, `--resume` did not
        // work, and the entire approach C is wrong.
        let second = session
            .ask("Quel mot venais-tu de répondre ?")
            .await
            .expect("2nd turn");
        assert!(
            second.text.to_lowercase().contains("un"),
            "2nd turn did not see the 1st — --resume did not work: {}",
            second.text
        );
    }
}
