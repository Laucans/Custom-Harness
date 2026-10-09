//! The doctor: the one who examines the plant, and treats it when asked.
//!
//! A second interactive Claude Code session beside the steward's, opened in
//! the harness checkout from the infirmary and from the header, with the
//! check-up already asked when it opens. This module is the pure half — the
//! standing orders the session is briefed with and the opening request.
//! The pseudo-terminal and the bridge to the browser are the steward's
//! (`crate::adapters::pty`, `crate::desk`), used twice.
//!
//! The briefing names `harness doctor` exactly, because the repair it
//! performs discards uncommitted files: the one mistake a doctor must not
//! make is to treat without being asked, or while a session is still writing.

use serde::Serialize;

use crate::domain::snapshot::Project;

/// The doctor's standing orders: who they are, what their instruments are,
/// and the one rule about the repair. Appended to Claude Code's system prompt.
#[must_use]
pub fn briefing(project: &Project) -> String {
    let repo = if project.slug.is_empty() {
        "this checkout's `origin`".to_string()
    } else {
        format!("`{}`", project.slug)
    };
    format!(
        "You are the DOCTOR of the plant \"{name}\": the one the human calls to examine the \
harness and, when asked, to treat it. The human sees you in the infirmary of the plant's page as \
a red, lobster-faced physician in a white coat with a head mirror — and talks to you in this \
terminal, in a pane beside the plant.

Where you stand: this shell runs in the harness checkout (the current directory). The harness \
drives Claude Code sessions through verification gates on the GitHub repository {repo}; its \
polling loop is `harness watch`, its traces are under `.llocal/logs/`, its binary is \
`./target/release/harness` (`cargo build --release -p harness-launcher` if it is missing). \
`CLAUDE.md` and `ARCHITECTURE_OVERVIEW.md` explain the rest. Read anything here you need.

Your instruments — use these rather than improvising:
- Pulse: `pgrep -fl \"harness watch\"`; `tail -n 20 .llocal/logs/agent-loop/watch.log`; a paid \
session in flight shows as `pgrep -fl \"claude -p\"`.
- Chart: `.llocal/logs/agent-loop/errors.tsv` — why the harness stopped, one row per stop, \
tab-separated, header on line 1, newest last. `.llocal/logs/agent-loop/costs.tsv` — what it \
spent. A run's own logs: `.llocal/logs/<workflow>/<run-id>/run.log` and `session.log`.
- Examination: `./target/release/harness doctor --dry-run` — reads the last stop, says what the \
repair would do, changes nothing, and names the folders under `.llocal/` nobody can account \
for. Always run this one first.
- Workspaces: `.llocal/agentic_workspaces/` and `.llocal/lanes/` hold the checkouts the runs \
work in; `git -C <workspace> status --short` shows what a run left uncommitted; `du -sh \
.llocal/*` shows where the disk goes.
- Treatment: `./target/release/harness doctor` — the repair: it discards the uncommitted files \
of the workspace the last failed run mounted, so the next run's clean-tree gate passes. It \
refuses by itself while a `claude -p` session is alive. You run it ONLY when the human says so \
in so many words, never on your own initiative, and never while the watch has a session in \
flight; say what it will discard before you ask for the go.
- The GitHub board, only to read: `gh issue list --label harness:milestone --state open`, and so \
on. Never close, delete, merge, force-push or edit anything there.

How you answer: in the language the human writes; as a bill of health, brief — VITALS (watch, \
sessions, last tick), SYMPTOMS (what the chart and the workspaces show), DIAGNOSIS (what is \
wrong and why, or that nothing is), TREATMENT (each gesture with its exact command, and whether \
it needs the human's go). No preamble. End every reply with one line: \
`HEALTH: <green|amber|red> · <one sentence>`.",
        name = project.name,
    )
}

/// The opening request: the full check-up, asked the moment the doctor sits
/// down, so the pane shows a diagnosis and not an empty prompt.
pub const CHECKUP: &str = "Give the plant a full check-up now — read, do not treat. \
1) Pulse: is `harness watch` running, what do the last 20 lines of its watch.log say, is a \
session in flight. \
2) Chart: the last 10 rows of errors.tsv, grouped by kind, with when the last one happened. \
3) Examination: `./target/release/harness doctor --dry-run` — what it would repair, and the \
stray folders it names (build the binary first if it is missing). \
4) Workspaces: uncommitted files in each workspace under .llocal/agentic_workspaces and \
.llocal/lanes, and the disk under .llocal. \
5) Spending: today's total from costs.tsv, by workflow. \
Then the bill of health: VITALS, SYMPTOMS, DIAGNOSIS, TREATMENT — each treatment with its exact \
command and whether it needs my go. Be brief; numbers over prose. Finish with the HEALTH line.";

/// What one agent's logs are worth handing over: the tail of `run.log`.
pub const RUN_LOG_BYTES: u64 = 8 * 1024;

/// And of `session.log`, where a session's own words are.
pub const SESSION_LOG_BYTES: u64 = 6 * 1024;

/// The question asked about one agent that failed or warned: who it is, where
/// its logs are, the tail of them inline, and the line to end on — worded so
/// the marker [`mark`] listens for never appears in the question itself.
#[must_use]
pub fn diagnosis(workflow: &str, run: &str, run_log: &str, session_log: &str) -> String {
    let session = if session_log.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n\nThe tail of its session.log:\n```\n{}\n```",
            printable(session_log)
        )
    };
    format!(
        "An agent of the plant stopped badly or warned: the run `{run}` of the `{workflow}` line. \
Its logs are `.llocal/logs/{workflow}/{run}/run.log` and `session.log` (read the full files if \
the tails below are not enough). Say what happened, in order: SYMPTOM (the line that says it \
stopped or warned, quoted), CAUSE (why, from the logs — a gate, a command, a quota, a timeout, \
the model), TREATMENT (what to do now, each gesture with its exact command, and whether it needs \
my go; `harness doctor --dry-run` first if a repair is in question). Read, do not treat. Be brief. \
Then end your reply with one line made of the word DIAGNOSIS, a colon, a space, the run id \
{run}, then ` · ` and one of green, amber, red.\n\nThe tail of its run.log:\n```\n{}\n```{session}",
        printable(run_log),
    )
}

/// What the doctor's reply ends with once this run is diagnosed, as it reads
/// in the terminal's stream with every escape and blank taken out.
#[must_use]
pub fn mark(run: &str) -> String {
    format!("DIAGNOSIS:{run}")
}

/// A diagnosis asked of the doctor, as the page polls it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Diagnosis {
    /// Asked, not yet answered.
    Running {
        /// When it was asked, a UTC clock.
        since: String,
    },
    /// The doctor ended a reply with this run's mark.
    Done {
        /// When it was asked, a UTC clock.
        since: String,
    },
    /// The doctor's program ended, or did not answer in time.
    Lost {
        /// When it was asked, a UTC clock.
        since: String,
    },
}

/// Whether the terminal's stream so far carries this run's mark: the stream
/// is flattened — escapes gone, no whitespace — because the program wraps
/// and repaints its lines as it likes.
#[must_use]
pub fn diagnosed(stream: &[u8], run: &str) -> bool {
    compact(stream).contains(&mark(run))
}

/// The stream without its escape sequences and without any whitespace.
#[must_use]
pub fn compact(stream: &[u8]) -> String {
    let text = String::from_utf8_lossy(stream);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                // CSI: parameters, then one final byte in `@`..=`~`.
                Some('[') => {
                    for d in chars.by_ref() {
                        if ('@'..='~').contains(&d) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC `\`.
                Some(']') => {
                    while let Some(d) = chars.next() {
                        if d == '\u{7}' {
                            break;
                        }
                        if d == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // A two-byte escape, or the end.
                _ => {}
            }
            continue;
        }
        if !c.is_whitespace() && !c.is_control() {
            out.push(c);
        }
    }
    out
}

/// A log excerpt safe to paste into a terminal: control characters other than
/// newlines and tabs are dropped, so the paste cannot end itself early.
fn printable(text: &str) -> String {
    text.chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        Project {
            name: "dnd_helper".to_string(),
            slug: "Laucans/dnd_helper".to_string(),
            url: "https://github.com/Laucans/dnd_helper".to_string(),
            integration_branch: "main_agent".to_string(),
        }
    }

    #[test]
    fn the_briefing_names_the_plant_the_instruments_and_the_one_rule() {
        let text = briefing(&project());
        assert!(text.contains("DOCTOR of the plant \"dnd_helper\""));
        assert!(text.contains("`Laucans/dnd_helper`"));
        assert!(text.contains("harness doctor --dry-run"));
        assert!(text.contains(".llocal/logs/agent-loop/errors.tsv"));
        assert!(text.contains("ONLY when the human says so"));
        assert!(text.contains("HEALTH: <green|amber|red>"));
    }

    #[test]
    fn without_a_target_the_briefing_falls_back_to_origin() {
        let mut p = project();
        p.slug = String::new();
        assert!(briefing(&p).contains("this checkout's `origin`"));
    }

    #[test]
    fn a_diagnosis_names_the_run_its_logs_and_never_its_own_mark() {
        let text = diagnosis(
            "agent-loop",
            "20261007-142103",
            "[12:00:01] FAILED: cargo test\x1b[0m broke",
            "── turn 1 ──\nhello",
        );
        assert!(text.contains(".llocal/logs/agent-loop/20261007-142103/run.log"));
        assert!(text.contains("FAILED: cargo test"));
        assert!(!text.contains('\u{1b}'), "no escape reaches the terminal");
        assert!(text.contains("session.log:\n```\n── turn 1"));
        assert!(
            !diagnosed(text.as_bytes(), "20261007-142103"),
            "the question echoed back is not the answer"
        );
        let short = diagnosis("split", "r1", "x", "   ");
        assert!(!short.contains("session.log:"), "no empty session block");
    }

    #[test]
    fn the_mark_is_heard_through_escapes_and_wrapped_lines() {
        let run = "20261007-142103";
        assert!(diagnosed(
            b"...\r\nDIAGNOSIS: 20261007-142103 \xc2\xb7 green\r\n",
            run
        ));
        assert!(diagnosed(
            b"\x1b[2K\x1b[1mDIAGNOSIS:\x1b[0m 202610\r\n\x1b[31m07-142103\x1b[0m \xc2\xb7 red",
            run
        ));
        assert!(diagnosed(
            b"\x1b]0;title\x07DIAGNOSIS: 20261007-142103",
            run
        ));
        assert!(!diagnosed(
            b"DIAGNOSIS: 20261007-000000 \xc2\xb7 green",
            run
        ));
        assert!(!diagnosed(b"HEALTH: green \xc2\xb7 fine", run));
        assert_eq!(compact(b"a \x1b[31mb\x1b[0m\n c"), "abc");
    }

    #[test]
    fn a_diagnosis_state_serializes_with_its_tag() {
        let json = serde_json::to_string(&Diagnosis::Running {
            since: "2026-10-09T10:00:00Z".to_string(),
        })
        .expect("json");
        assert_eq!(
            json,
            r#"{"state":"running","since":"2026-10-09T10:00:00Z"}"#
        );
    }

    #[test]
    fn the_check_up_reads_and_does_not_treat() {
        assert!(CHECKUP.contains("do not treat"));
        assert!(CHECKUP.contains("--dry-run"));
        assert!(
            !CHECKUP.contains("harness doctor`"),
            "no bare repair is asked"
        );
        assert!(CHECKUP.ends_with("HEALTH line."));
    }
}
