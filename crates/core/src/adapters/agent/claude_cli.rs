//! Approach C: one process per action, stitched together via `--resume`.
//!
//! Each turn is `claude -p --output-format json`. The first call creates the
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

use std::path::{Path, PathBuf};
use std::process::Output;

use async_trait::async_trait;
use serde::Deserialize;

use crate::adapters::agent::{Reply, Session, SessionFactory, SessionSpec};
use crate::domain::{Halt, Outcome, Spend, Tokens, markers};

/// The called binary. Named here so a test can read it.
const BINARY: &str = "claude";

/// Phrases that mark a failure as an exhausted quota.
///
/// In one place: this is a heuristic on error text, so it will drift, and
/// when it drifts we want one place to fix. Searched only in a turn
/// **already failing** — a session that succeeds while mentioning "rate limit"
/// is not a quota.
const QUOTA_PHRASES: [&str; 4] = ["usage limit", "rate limit", "quota", "too many requests"];

/// What we read from the JSON of `--output-format json`.
///
/// All fields are intentionally optional: Claude Code adds them over versions,
/// and an unknown field must not fail a turn that went well.
#[derive(Debug, Deserialize)]
struct CliResult {
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
}

impl ClaudeCli {
    /// A conversation that has not yet taken place.
    #[must_use]
    pub fn new(cwd: PathBuf, spec: &SessionSpec, permission_mode: &str) -> Self {
        Self {
            cwd,
            model: spec.model.clone(),
            effort: spec.effort.clone(),
            permission_mode: permission_mode.to_string(),
            session_id: None,
        }
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
    fn argv(&self, prompt: &str) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            prompt.to_string(),
            "--output-format".to_string(),
            "json".to_string(),
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
        args
    }
}

/// How to classify a failed turn.
///
/// A quota is neither "repaired" nor "abandoned": it is the same work to
/// restart later, unchanged. Distinguishing it from a failure prevents
/// replaying a session that had nothing broken.
fn halt_for(subtype: &str, text: &str) -> Halt {
    let haystack = format!("{subtype} {text}").to_lowercase();
    if QUOTA_PHRASES.iter().any(|phrase| haystack.contains(phrase)) {
        return Halt::Quota(text.to_string());
    }
    Halt::Failed(text.to_string())
}

/// What a turn returned, plus the conversation identifier.
///
/// Pure: everything after the binary call tests by passing it a string.
///
/// # Errors
///
/// - [`Halt::Failed`] if the JSON is unreadable, or if the turn succeeded
///   without returning anything — an empty turn is not usable by the next
///   action;
/// - [`Halt::Quota`] or [`Halt::Failed`] depending on what the failure says,
///   see [`halt_for`].
fn parse(stdout: &str) -> Outcome<(Reply, String)> {
    let parsed: CliResult = serde_json::from_str(stdout.trim())
        .map_err(|e| Halt::Failed(format!("unreadable response from {BINARY} : {e}")))?;

    if parsed.is_error {
        return Err(halt_for(&parsed.subtype, &parsed.result));
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
/// `stderr` first: when `claude` rejects a flag, the reason is there, and the
/// JSON on stdout is absent.
fn failed_process(out: &Output) -> Halt {
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let said = if stderr.is_empty() { stdout } else { stderr };
    let code = out
        .status
        .code()
        .map_or_else(|| "killed by signal".to_string(), |c| format!("code {c}"));
    halt_for("", &format!("{BINARY} stopped ({code}): {said}"))
}

#[async_trait(?Send)]
impl Session for ClaudeCli {
    async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
        let out = tokio::process::Command::new(BINARY)
            .args(self.argv(prompt))
            .current_dir(&self.cwd)
            .output()
            .await
            .map_err(|e| Halt::Failed(format!("{BINARY} could not be launched: {e}")))?;

        if !out.status.success() {
            return Err(failed_process(&out));
        }

        let (reply, session_id) = parse(&String::from_utf8_lossy(&out.stdout))?;
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
}

impl ClaudeCliFactory {
    /// A factory that runs each session in `cwd`.
    ///
    /// `cwd` is the root of the run's checkout — the clone, not the repo
    /// from which the run is launched.
    #[must_use]
    pub fn new(cwd: &Path, permission_mode: &str) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            permission_mode: permission_mode.to_string(),
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
        )
    }

    // --- argv ---------------------------------------------------------------

    #[test]
    fn the_first_turn_carries_no_resume() {
        let args = cli().argv("/business-analyst");
        assert!(!args.contains(&"--resume".to_string()));
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], "/business-analyst");
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
        assert!(args.contains("--output-format json"));
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
        let (reply, session) = parse(SUCCESS).expect("parse");
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
        let (reply, _) = parse(json).expect("parse");
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
        let (reply, _) = parse(json).expect("parse");
        assert!(reply.spend.cost_usd.is_none());
        // Nothing observed, especially not zeros: a ledger must be able to
        // write "not measured" rather than a free session.
        assert!(reply.spend.is_blind());
    }

    #[test]
    fn an_empty_result_is_a_failure_not_an_empty_success() {
        let json = r#"{"is_error":false,"result":"   ","session_id":"s"}"#;
        let err = parse(json).expect_err("doit échouer");
        assert!(matches!(err, Halt::Failed(_)));
    }

    #[test]
    fn unreadable_output_fails_rather_than_panicking() {
        let err = parse("ceci n'est pas du json").expect_err("doit échouer");
        assert!(matches!(err, Halt::Failed(_)));
    }

    // --- classement des échecs ---------------------------------------------

    #[test]
    fn an_exhausted_window_is_a_quota_not_a_failure() {
        let json = r#"{"is_error":true,"subtype":"error_during_execution",
                       "result":"Claude usage limit reached","session_id":"s"}"#;
        let err = parse(json).expect_err("must fail");
        assert!(
            matches!(err, Halt::Quota(_)),
            "a quota restarts the same work later; a failure replays it"
        );
    }

    #[test]
    fn any_other_error_subtype_is_a_plain_failure() {
        let json = r#"{"is_error":true,"subtype":"error_during_execution",
                       "result":"the tool crashed","session_id":"s"}"#;
        assert!(matches!(
            parse(json).expect_err("must fail"),
            Halt::Failed(_)
        ));
    }

    #[test]
    fn quota_phrases_are_matched_case_insensitively() {
        assert!(matches!(
            halt_for("", "Rate Limit Exceeded"),
            Halt::Quota(_)
        ));
    }

    // --- la fabrique --------------------------------------------------------

    #[tokio::test]
    async fn opening_a_session_spawns_nothing() {
        // Under C, `open` is free: the conversation is born at the first `ask`.
        // This test passes without any `claude` binary existing.
        let factory = ClaudeCliFactory::new(Path::new("/tmp/workspace"), "bypassPermissions");
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
        let factory = ClaudeCliFactory::new(Path::new("."), "bypassPermissions");
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
