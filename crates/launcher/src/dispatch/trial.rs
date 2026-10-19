//! `harness trial`: a prompt's dry-run, asked from the view.
//!
//! Replays one past run's prompt with another version of a stage's text, in
//! a real session that cannot act, and reports what it answered, how long it
//! took and what it cost.
//!
//! - **The question** is the past run's own: its `prompts.md` holds every
//!   prompt it sent, its `prompt-texts.json` the text each stage used then
//!   (the default, for a run older than that record). The values that text
//!   was filled with are read back, the version tried is filled with them,
//!   and the framework's blocks around stay as they were
//!   (`prompt_book::replay`).
//! - **The session cannot act**: `--permission-mode plan`, so the model reads
//!   and answers but edits nothing and runs no command that writes; and it
//!   works in a clone of its own (`prompt-trial`), never in one a lane is
//!   using. No stage, no gate, no label, no checkpoint: nothing of the
//!   factory moves.
//! - **The answer** goes to `.llocal/trials/<id>/trial.json`, written when
//!   the trial starts and again when it ends, with the session's traces
//!   beside it. Nothing a run reads.
//!
//! The text tried comes on stdin: a draft never has to be written anywhere
//! a run could read it.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use harness_adapters::agent::claude_cli::{ClaudeCliFactory, sent_prompts};
use harness_adapters::store::prompt_book::read_used;
use harness_adapters::store::trial::{self, Report, State};
use harness_core::domain::prompt_book::{ReplayError, replay};
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome};
use harness_core::ports::agent::{SessionFactory, SessionSpec};
use harness_core::traces::{Logbook, Sink, Verbosity};

use crate::adapters::sink::Both;
use crate::cli::TrialArgs;
use crate::dispatch::shared;

/// The clone a trial reads, apart from every lane's.
const CLONE: &str = "prompt-trial";

/// What makes a session unable to act.
const READ_ONLY: &str = "plan";

/// Runs one trial and records its report.
///
/// # Errors
///
/// Only when the report itself cannot be written; anything else — a run
/// without that prompt, a session that failed — is the report's `failed`.
pub async fn run(args: &TrialArgs, here: &Path) -> Outcome<()> {
    if !safe(&args.id) || !safe(&args.run) || !safe(&args.workflow) {
        return Err(Halt::Halted(
            "trial: an id, a run or a line is a plain name, never a path".to_string(),
        ));
    }
    let workspace = Workspace::new(here);
    let dir = workspace
        .state_root()
        .join(".llocal")
        .join(trial::TRIALS)
        .join(&args.id);
    let mut report = Report {
        state: State::Running,
        key: args.key.clone(),
        version: args.version.clone(),
        workflow: args.workflow.clone(),
        run: args.run.clone(),
        model: args.model.clone(),
        effort: args.effort.clone(),
        started_at: crate::dispatch::dev_loop::now_seconds(),
        ..Report::default()
    };
    trial::write_report(&dir, &report)?;
    match attempt(args, here, &workspace, &dir).await {
        Ok((chars, reply)) => {
            report.state = State::Done;
            report.prompt_chars = Some(chars);
            report.output = Some(reply.text);
            report.cost_usd = reply.spend.cost_usd;
            report.duration_ms = reply.spend.duration_ms;
            report.turns = reply.spend.turns;
        }
        Err(halt) => {
            report.state = State::Failed;
            report.reason = Some(halt.reason().to_string());
        }
    }
    report.finished_at = Some(crate::dispatch::dev_loop::now_seconds());
    trial::write_report(&dir, &report)
}

/// Builds the prompt, opens the session, asks once.
async fn attempt(
    args: &TrialArgs,
    here: &Path,
    workspace: &Workspace,
    dir: &Path,
) -> Outcome<(usize, harness_core::ports::agent::Reply)> {
    let mut tried = String::new();
    std::io::stdin()
        .read_to_string(&mut tried)
        .map_err(|e| Halt::Failed(format!("the text to try could not be read: {e}")))?;
    let prompt = question(args, workspace, &tried)?;

    let sink = Rc::new(
        Both::new(&dir.join("run.log"))
            .map_err(|e| Halt::Failed(format!("log file cannot be opened: {e}")))?,
    );
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);
    log.say(&format!(
        "trial: {} ({}) replayed on {}/{}, {} chars, {}/{} in plan mode",
        args.key,
        args.version,
        args.workflow,
        args.run,
        prompt.chars().count(),
        args.model,
        args.effort
    ));
    let mount = shared::mount_named(
        here,
        &args.target_repo_url,
        &args.branch,
        CLONE,
        false,
        &args.id,
        false,
        &log,
    )
    .await?;
    let sessions =
        ClaudeCliFactory::new(mount.workspace.root(), READ_ONLY, Some(dir.to_path_buf()));
    let answered = async {
        let mut session = sessions
            .open(&SessionSpec {
                model: args.model.clone(),
                effort: args.effort.clone(),
            })
            .await?;
        session.ask(&prompt).await
    }
    .await;
    shared::release_checkout(&mount, &log).await;
    Ok((prompt.chars().count(), answered?))
}

/// The past run's prompt, with the text tried in place of the one it used.
fn question(args: &TrialArgs, workspace: &Workspace, tried: &str) -> Outcome<String> {
    let prompt = harness_workflows::prompts::find(&args.key)
        .ok_or_else(|| Halt::Halted(format!("trial: no prompt named {}", args.key)))?;
    let run_dir: PathBuf = workspace.log_dir(&args.workflow).join(&args.run);
    let recorded = std::fs::read_to_string(run_dir.join("prompts.md")).map_err(|e| {
        Halt::Halted(format!(
            "trial: {}/{} kept no prompt ({e}) — pick a run that opened a session",
            args.workflow, args.run
        ))
    })?;
    let used = read_used(&run_dir)?
        .and_then(|texts| texts.get(prompt.key).cloned())
        .unwrap_or_else(|| prompt.default.to_string());
    let mut why = ReplayError::NotFound;
    for sent in sent_prompts(&recorded).iter().rev() {
        match replay(sent, &used, tried) {
            Ok(question) => return Ok(question),
            Err(error) => why = error,
        }
    }
    Err(Halt::Halted(format!(
        "trial: {}/{} did not send {} as it reads now — {why}",
        args.workflow, args.run, prompt.key
    )))
}

/// A name with no way out of its folder.
fn safe(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(run: &str) -> TrialArgs {
        TrialArgs {
            workflow: "split".to_string(),
            run: run.to_string(),
            key: harness_workflows::prompts::SLICE.to_string(),
            version: "draft".to_string(),
            model: "sonnet".to_string(),
            effort: "high".to_string(),
            id: "t1".to_string(),
            target_repo_url: String::new(),
            branch: "main_agent".to_string(),
        }
    }

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_past_run_is_replayed_with_the_text_tried_and_its_blocks_kept() {
        let dir = Dir(std::env::temp_dir().join(format!("harness-trial-q-{}", std::process::id())));
        let workspace = Workspace::new(&dir.0);
        let run_dir = workspace.log_dir("split").join("r1");
        std::fs::create_dir_all(&run_dir).expect("dir");
        let used = "Milestone #{num}: cut it.";
        harness_adapters::store::prompt_book::record_used(
            &run_dir,
            &std::collections::BTreeMap::from([(
                harness_workflows::prompts::SLICE.to_string(),
                used.to_string(),
            )]),
        )
        .expect("recorded");
        std::fs::write(
            run_dir.join("prompts.md"),
            "## turn 1 — (prose) (opus/high)\n\n```\n--- CONTEXT ---\nMilestone #12: cut it.\n```\n\n",
        )
        .expect("written");
        assert_eq!(
            question(&args("r1"), &workspace, "Slice #{num} in three.").expect("replayed"),
            "--- CONTEXT ---\nSlice #12 in three."
        );
        let refused = question(&args("missing"), &workspace, "x").expect_err("no run");
        assert!(
            refused.reason().contains("kept no prompt"),
            "{}",
            refused.reason()
        );
    }

    #[test]
    fn a_name_never_leaves_its_folder() {
        assert!(safe("20261009-120000-12"));
        assert!(!safe("../etc") && !safe("a/b") && !safe("") && !safe(".hidden"));
    }
}
