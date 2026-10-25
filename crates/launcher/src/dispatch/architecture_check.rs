//! `harness architecture-check`: the closure check a session runs before
//! its pull request, through the product's `scripts/gates-local.sh`.
//!
//! The loop's own reading — [`closure_check`], the same `judge` the
//! architecture gate runs after the session — on a full copy of the work
//! (`--tree`), the round's changes listed in the session's checkout
//! (`--repo`, `--from` the merge base `--to` the snapshot), against the
//! task's declaration: the session facts (`--facts`, written by the loop
//! before the session) or, in a repair session that has none, the task's
//! body off GitHub (`--task`). One reading in one binary: what the session
//! sees red is what would have stopped its round.
//!
//! Prints `closure: ok`, or one line per finding of the round's own and the
//! older ones after them, marked; exits 1 on a finding of the round's own,
//! 2 when it cannot judge.

use std::path::Path;

use harness_adapters::shell::disk::RealDisk;
use harness_adapters::shell::git::GitCli;
use harness_adapters::shell::github::GhCli;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;
use harness_workflows::dev_loop::checks::architecture::{Judgement, closure_check};
use harness_workflows::dev_loop::data::facts::{Facts, architecture_of};

use crate::cli::ArchitectureCheckArgs;

/// What the check says, and its exit code.
pub async fn run(args: &ArchitectureCheckArgs) -> (String, u8) {
    match check(args).await {
        Ok(judged) => said(judged.as_ref()),
        Err(why) => (format!("closure: cannot judge — {why}"), 2),
    }
}

/// The task's declaration and skeleton, from the facts or its body.
async fn declared(
    args: &ArchitectureCheckArgs,
    disk: &dyn Disk,
) -> Result<(String, Vec<String>), String> {
    if !args.facts.is_empty() {
        let text = disk
            .read_to_string(Path::new(&args.facts))
            .ok_or_else(|| format!("no session facts at {}", args.facts))?;
        let facts = Facts::read(&text).ok_or_else(|| format!("{} holds no facts", args.facts))?;
        return Ok((facts.architecture, facts.skeleton));
    }
    let Some(task) = args.task else {
        return Err("neither --facts nor --task names the task".to_string());
    };
    let issue = GhCli::new(&args.repo)
        .issue(task)
        .await
        .map_err(|halt| format!("#{task} cannot be read: {}", halt.reason()))?;
    Ok((architecture_of(&issue.body), Vec::new()))
}

async fn check(args: &ArchitectureCheckArgs) -> Result<Option<Judgement>, String> {
    let disk = RealDisk;
    let (architecture, skeleton) = declared(args, &disk).await?;
    let repo = GitCli::new(&args.repo);
    closure_check(
        &disk,
        &args.tree,
        &repo,
        &architecture,
        &skeleton,
        args.from.as_deref(),
        &args.to,
    )
    .await
    .map_err(|halt| halt.reason().to_string())
}

/// The lines printed and the exit code of a judgement.
fn said(judged: Option<&Judgement>) -> (String, u8) {
    let Some(judged) = judged else {
        return (
            "closure: no contracts/ in this tree — not under the architecture".to_string(),
            0,
        );
    };
    let mut lines: Vec<String> = judged.own.clone();
    lines.extend(
        judged
            .before
            .iter()
            .map(|violation| format!("(before this round, not yours) {violation}")),
    );
    if judged.own.is_empty() {
        lines.insert(0, "closure: ok".to_string());
        return (lines.join("\n"), 0);
    }
    (lines.join("\n"), 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finding_of_the_round_fails_and_an_older_one_is_said() {
        let clean = Judgement::default();
        assert_eq!(said(Some(&clean)), ("closure: ok".to_string(), 0));
        let older = Judgement {
            own: Vec::new(),
            before: vec!["mock/pages.json: stale".to_string()],
        };
        assert_eq!(
            said(Some(&older)),
            (
                "closure: ok\n(before this round, not yours) mock/pages.json: stale".to_string(),
                0
            )
        );
        let mine = Judgement {
            own: vec!["crates/x/y.rs: outside the closure".to_string()],
            before: Vec::new(),
        };
        assert_eq!(
            said(Some(&mine)),
            ("crates/x/y.rs: outside the closure".to_string(), 1)
        );
        assert_eq!(said(None).1, 0);
    }
}
