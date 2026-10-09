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
