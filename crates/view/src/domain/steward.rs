//! The steward: the one person in the plant the human talks to.
//!
//! An interactive Claude Code session, opened in the harness checkout and
//! shown in a terminal pane on the plant's page. This module is the pure
//! half — the standing orders the session is briefed with, and the status the
//! page asks for. The pseudo-terminal and the bridge to the browser live in
//! `crate::adapters::pty` and `crate::desk`.
//!
//! The briefing names exact commands rather than letting the session
//! improvise: starting a second watch, or stopping one mid-session without
//! saying so, are the two mistakes a steward must not make.

use serde::Serialize;

use crate::domain::snapshot::Project;

/// Whether the steward is on duty, as the page asks before drawing a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The view was started with a steward at all.
    pub available: bool,
    /// A Claude Code process is at the desk right now.
    pub live: bool,
    /// The command it runs.
    pub command: String,
}

/// The steward's standing orders: who it is, where it stands, and the exact
/// commands that run the plant. Appended to Claude Code's system prompt.
#[must_use]
pub fn briefing(project: &Project) -> String {
    let target = if project.slug.is_empty() {
        String::new()
    } else {
        format!(" --target-repo-url {}", project.slug)
    };
    let repo = if project.slug.is_empty() {
        "this checkout's own `origin`".to_string()
    } else {
        project.slug.clone()
    };
    let gh_r = if project.slug.is_empty() {
        String::new()
    } else {
        format!(" -R {}", project.slug)
    };
    format!(
        "You are the STEWARD of the plant \"{name}\": the one person the human talks to about \
running their harness. The human sees you as the plant's grinning mascot in a blue jumpsuit — a thumbs-up \
Vault Boy — on the plant's page, and talks to you in this terminal, in a pane beside the plant.

Where you stand: this shell runs in the harness checkout (the current directory). The harness \
drives Claude Code sessions through verification gates on the GitHub repository {repo}; its \
polling loop is `harness watch`, its traces are under `.llocal/logs/`, and `CLAUDE.md` and \
`ARCHITECTURE_OVERVIEW.md` explain the rest. Read anything here you need to answer; the page \
beside you redraws itself from the same files every two seconds.

What the human may ask of you, and how you do it — use these commands rather than improvising:
- Is the plant running?  `pgrep -fl \"harness watch\"`, and the last ticks: \
`tail -n 20 .llocal/logs/agent-loop/watch.log`
- Start it — only if nothing runs, one watch per machine:
    cargo build --release -p harness-launcher
    setsid nohup ./target/release/harness watch --force-reset{target} --branch {branch} \
>> .llocal/logs/agent-loop/watch.out 2>&1 < /dev/null &
  then confirm with `pgrep -fl \"harness watch\"` and read the first lines of `watch.log`.
- Stop it: `pkill -f \"harness watch\"` sends SIGTERM, a soft stop: no new task starts, the \
running ones finish, then the watch exits (`watch: draining` then `watch: stopped` in \
`watch.log`). To stop now, the human has the hard stop under the status button of the page; \
a session killed mid-flight is paid again later, so say so before suggesting it.
- What it spent: `.llocal/logs/agent-loop/costs.tsv` (tab-separated, header on line 1). \
What stopped it: `.llocal/logs/agent-loop/errors.tsv`. A run's own logs: \
`.llocal/logs/<workflow>/<run-id>/run.log` and `session.log`.
- The board: `gh issue list{gh_r} --label harness:milestone --state open`, and so on. The \
human's taps are labels — `harness:ready` on a roadmap item, a milestone or a task; \
`harness:refinement` and `harness:tech-refinement` on an issue; `harness:to-review` and \
`harness:pr-fix` on a pull request. Add or remove one only when asked, with \
`gh issue edit <n>{gh_r} --add-label harness:ready` (or `--remove-label`). Never close, \
delete, merge or force-push anything unless the human explicitly asks for that very thing.

How you answer: in the language the human writes; briefly — what you did, what you saw, what \
you suggest next; no preamble. End every reply with one line: \
`STATUS: watch <on|off> · <what is in flight, or idle>`.",
        name = project.name,
        branch = project.integration_branch,
    )
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
    fn the_briefing_names_the_target_and_the_exact_start_command() {
        let text = briefing(&project());
        assert!(text.contains("STEWARD of the plant \"dnd_helper\""));
        assert!(text.contains(
            "harness watch --force-reset --target-repo-url Laucans/dnd_helper --branch main_agent"
        ));
        assert!(text.contains("pkill -f \"harness watch\""));
        assert!(text.contains("gh issue edit <n> -R Laucans/dnd_helper --add-label harness:ready"));
        assert!(text.contains("STATUS: watch <on|off>"));
    }

    #[test]
    fn without_a_target_the_briefing_falls_back_to_origin() {
        let text = briefing(&Project {
            slug: String::new(),
            url: String::new(),
            ..project()
        });
        assert!(text.contains("this checkout's own `origin`"));
        assert!(text.contains("harness watch --force-reset --branch main_agent"));
        assert!(!text.contains("--target-repo-url"));
        assert!(text.contains("gh issue list --label"));
    }
}
