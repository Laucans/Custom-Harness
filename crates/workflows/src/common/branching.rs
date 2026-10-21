//! The branch name a milestone pushes its tasks' PRs into.
//!
//! Shared because `split` writes it into each task's body (as a `branch:`
//! line naming the *task's own* branch, computed by the session, not this
//! module) while `init-repo`'s audit and `main_agent_merge` both need the
//! *milestone's own* branch name — derived, never asked of a session, so
//! the two sides can never disagree on what it is. And what keeps that
//! branch up to the integration branch ([`catch_up`], and
//! [`catch_up_locally`] when GitHub refuses on the generated files only).

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use harness_architecture::tree::REGISTRY;
use harness_core::domain::Outcome;
use harness_core::ports::shell::git::Repo;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::shell::process::{Process, Ran};
use harness_core::traces::Logbook;

/// Brings a milestone's branch up to the integration branch before its
/// tasks are handed out.
///
/// A fix pushed on the integration branch — a file the harness upgraded,
/// a milestone that landed after this one was cut — reaches the tasks of a
/// milestone already open: a milestone cut before the one before it landed
/// had tasks that found no Data layer. A refusal (a conflict) is said and
/// never forced: the milestone's own merge to the integration branch meets
/// it there. Whether the branch holds the integration branch now — a task
/// started on a branch that could not take it stops on what the base has
/// and it lacks (`ForgeDesMondes`, 2026-10-20: five sessions in one tick).
pub async fn catch_up(
    gh: &dyn GitHub,
    milestone_branch: &str,
    integration: &str,
    log: &Logbook,
) -> bool {
    match gh.merge_branch(milestone_branch, integration).await {
        Ok(()) => {
            log.debug(&format!("{milestone_branch} holds {integration}"));
            true
        }
        Err(halt) => {
            log.warn(&format!(
                "{milestone_branch} cannot take {integration}: {}",
                halt.reason()
            ));
            false
        }
    }
}

/// The files a merge regenerates instead of resolving: Cargo's lock and the
/// registry's manifest `scripts/register.sh` writes. Two units added in
/// parallel meet nowhere else.
pub const GENERATED: [&str; 2] = ["Cargo.lock", REGISTRY];

/// How long `scripts/register.sh` may run: it reads a few folders.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `cargo metadata` may run: it may fetch the engine by git.
const METADATA_TIMEOUT: Duration = Duration::from_secs(600);

/// A checkout of the product, the harness's own, where a refused catch-up
/// is tried again.
pub struct Checkout {
    /// Its `git`.
    pub repo: Rc<dyn Repo>,
    /// What runs the product's script and `cargo` in it.
    pub process: Rc<dyn Process>,
    /// Its root.
    pub root: PathBuf,
}

/// Brings `milestone_branch` up to `integration` in `checkout`, as
/// [`catch_up`] does on GitHub, and resolves a conflict limited to the
/// [`GENERATED`] files by regenerating them.
///
/// The base's `Cargo.lock` is taken, `scripts/register.sh` rewrites the
/// registry from the unit folders the merge holds, `cargo metadata` brings
/// the lock up to it, and the merge is committed and pushed. A conflict on
/// any other file, or a step that fails, aborts the merge and answers
/// false, as a refusal of [`catch_up`] does: it is a real conflict, met on
/// the milestone's own pull request.
pub async fn catch_up_locally(
    checkout: &Checkout,
    milestone_branch: &str,
    integration: &str,
    log: &Logbook,
) -> bool {
    match regenerate(checkout, milestone_branch, integration).await {
        Ok(regenerated) if regenerated.is_empty() => {
            log.say(&format!("{milestone_branch} holds {integration}"));
            true
        }
        Ok(regenerated) => {
            log.say(&format!(
                "{milestone_branch} holds {integration}: {} regenerated, never merged by hand",
                regenerated.join(" and ")
            ));
            true
        }
        Err(why) => {
            if checkout.repo.merge_in_progress().await.unwrap_or(true) {
                let _ = checkout.repo.abort_merge().await;
            }
            log.warn(&format!(
                "{milestone_branch} cannot take {integration}: {why}"
            ));
            false
        }
    }
}

/// The merge of [`catch_up_locally`]: the generated files it regenerated,
/// or why it stopped.
async fn regenerate(
    checkout: &Checkout,
    branch: &str,
    integration: &str,
) -> Result<Vec<String>, String> {
    let repo = &checkout.repo;
    done("fetch", repo.fetch().await)?;
    done("checkout", repo.checkout(branch, true).await)?;
    done("reset", repo.reset_hard(&format!("origin/{branch}")).await)?;
    let merged = repo
        .merge(&format!("origin/{integration}"))
        .await
        .map_err(|halt| halt.reason().to_string())?;
    if merged.ok() {
        done("push", repo.push(branch).await)?;
        return Ok(Vec::new());
    }
    let conflicted = unmerged(
        &repo
            .dirty_files()
            .await
            .map_err(|h| h.reason().to_string())?,
    );
    if conflicted.is_empty() {
        return Err(format!("the merge failed: {}", merged.why()));
    }
    let others: Vec<&str> = conflicted
        .iter()
        .map(String::as_str)
        .filter(|path| !GENERATED.contains(path))
        .collect();
    if !others.is_empty() {
        return Err(format!("it conflicts on {}", others.join(", ")));
    }
    if conflicted.iter().any(|path| path == "Cargo.lock") {
        done(
            "the base's Cargo.lock",
            repo.take_theirs("Cargo.lock").await,
        )?;
    }
    let process = &checkout.process;
    let register = ["scripts/register.sh".to_string()];
    done(
        "scripts/register.sh",
        process
            .run("bash", &register, &checkout.root, REGISTER_TIMEOUT)
            .await,
    )?;
    let metadata = ["metadata", "--format-version", "1"].map(String::from);
    done(
        "cargo metadata",
        process
            .run("cargo", &metadata, &checkout.root, METADATA_TIMEOUT)
            .await,
    )?;
    done("stage", repo.stage_all().await)?;
    done(
        "commit",
        repo.commit(&format!(
            "chore: merge {integration} into {branch}, {} regenerated",
            GENERATED.join(" and ")
        ))
        .await,
    )?;
    done("push", repo.push(branch).await)?;
    Ok(conflicted)
}

/// A step that ran and succeeded, or why it did not.
fn done(step: &str, ran: Outcome<Ran>) -> Result<(), String> {
    match ran {
        Ok(ran) if ran.ok() => Ok(()),
        Ok(ran) => Err(format!("{step} failed: {}", ran.why())),
        Err(halt) => Err(format!("{step} failed: {}", halt.reason())),
    }
}

/// The paths `status --porcelain` reports unmerged: `UU`, `AA`, `DU`, ….
fn unmerged(status: &[String]) -> Vec<String> {
    status
        .iter()
        .filter_map(|line| {
            let (code, path) = (line.get(..2)?, line.get(3..)?);
            matches!(code, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU")
                .then(|| path.trim_matches('"').to_string())
        })
        .collect()
}

/// The branch a milestone's tasks merge into: `milestone/<number>-<slug>`.
#[must_use]
pub fn milestone_branch(number: u64, title: &str) -> String {
    format!("milestone/{number}-{}", slugify(title))
}

/// The milestone a branch of [`milestone_branch`]'s shape names, if it is
/// one: `milestone/18-campaigns` is #18's; `main_agent` is nobody's.
#[must_use]
pub fn milestone_number(branch: &str) -> Option<u64> {
    branch
        .strip_prefix("milestone/")?
        .split('-')
        .next()?
        .parse()
        .ok()
}

/// Lowercase, non-alphanumeric runs collapsed to one `-`, trimmed of
/// leading/trailing `-`.
fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_dash = false;
    for ch in text.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::common::fake_process::FakeProcess;
    use harness_core::ports::shell::git::Repos as _;
    use harness_core::traces::{Sink, Verbosity};
    use std::cell::RefCell;
    use std::path::Path;

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    /// Catches `milestone/17-ai` up with `main_agent` in a checkout of
    /// `repos`, the product's commands answered by `process`: whether it
    /// holds the base, and what the journal said.
    async fn locally(repos: &FakeRepos, process: &Rc<FakeProcess>) -> (bool, Vec<String>) {
        let root = Path::new("/w/catch-up");
        let checkout = Checkout {
            repo: repos.at(root),
            process: Rc::clone(process) as Rc<dyn Process>,
            root: root.to_path_buf(),
        };
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        let held = catch_up_locally(&checkout, "milestone/17-ai", "main_agent", &log).await;
        let said = capture.0.borrow().clone();
        (held, said)
    }

    /// The verbs asked, without the root.
    fn verbs(repos: &FakeRepos) -> Vec<String> {
        repos
            .calls()
            .iter()
            .map(|call| {
                call.split_once(": ")
                    .map_or(call.as_str(), |(_, v)| v)
                    .to_string()
            })
            .collect()
    }

    fn programs(process: &FakeProcess) -> Vec<String> {
        process
            .ran
            .borrow()
            .iter()
            .map(|(program, args, root)| {
                assert_eq!(root, Path::new("/w/catch-up"), "run in the checkout");
                format!("{program} {}", args.join(" "))
            })
            .collect()
    }

    #[tokio::test]
    async fn a_conflict_on_the_generated_pair_only_is_regenerated_and_committed() {
        let repos = FakeRepos::conflicting(&["Cargo.lock", REGISTRY], &[]);
        let process = Rc::new(FakeProcess::default());
        let (held, said) = locally(&repos, &process).await;
        assert!(held, "{said:?}");
        assert_eq!(
            verbs(&repos),
            [
                "fetch",
                "checkout milestone/17-ai --force",
                "reset_hard origin/milestone/17-ai",
                "merge origin/main_agent",
                "take_theirs Cargo.lock",
                "stage_all",
                "commit chore: merge main_agent into milestone/17-ai, Cargo.lock and \
                 crates/dataguard/registry/Cargo.toml regenerated",
                "push milestone/17-ai",
            ]
        );
        assert_eq!(
            programs(&process),
            [
                "bash scripts/register.sh",
                "cargo metadata --format-version 1"
            ]
        );
        assert_eq!(
            said,
            ["milestone/17-ai holds main_agent: Cargo.lock and \
              crates/dataguard/registry/Cargo.toml regenerated, never merged by hand"]
        );
    }

    #[tokio::test]
    async fn a_conflict_on_the_registry_alone_keeps_the_lock_the_merge_made() {
        let repos = FakeRepos::conflicting(&[REGISTRY], &[]);
        let process = Rc::new(FakeProcess::default());
        let (held, _) = locally(&repos, &process).await;
        assert!(held);
        assert!(!repos.asked("take_theirs"), "{:?}", repos.calls());
        assert!(repos.asked("commit") && repos.asked("push milestone/17-ai"));
        assert_eq!(programs(&process).len(), 2);
    }

    #[tokio::test]
    async fn a_conflict_on_a_source_file_too_is_refused_and_the_merge_aborted() {
        let repos =
            FakeRepos::conflicting(&["Cargo.lock", REGISTRY, "crates/credit/src/lib.rs"], &[]);
        let process = Rc::new(FakeProcess::default());
        let (held, said) = locally(&repos, &process).await;
        assert!(!held);
        assert!(process.ran.borrow().is_empty(), "nothing regenerated");
        assert!(repos.asked("abort_merge"));
        assert!(!repos.asked("take_theirs") && !repos.asked("commit") && !repos.asked("push"));
        assert_eq!(
            said,
            [
                "warning: milestone/17-ai cannot take main_agent: it conflicts on crates/credit/src/lib.rs"
            ]
        );
    }

    #[tokio::test]
    async fn a_failing_register_script_is_refused_and_the_merge_aborted() {
        let repos = FakeRepos::conflicting(&["Cargo.lock", REGISTRY], &[]);
        let process = Rc::new(FakeProcess::answering(vec![FakeProcess::ran(
            "",
            "::error file=crates/x/Cargo.toml::no [package] name",
            1,
        )]));
        let (held, said) = locally(&repos, &process).await;
        assert!(!held);
        assert_eq!(
            programs(&process),
            ["bash scripts/register.sh"],
            "no metadata after it"
        );
        assert!(repos.asked("abort_merge"));
        assert!(!repos.asked("commit") && !repos.asked("push"));
        assert_eq!(said.len(), 1);
        assert!(
            said[0].starts_with(
                "warning: milestone/17-ai cannot take main_agent: scripts/register.sh failed: ::error"
            ),
            "{said:?}"
        );
    }

    #[tokio::test]
    async fn a_failing_metadata_or_push_is_refused() {
        let repos = FakeRepos::conflicting(&[REGISTRY], &[]);
        let process = Rc::new(FakeProcess::answering(vec![
            FakeProcess::ran("", "", 0),
            FakeProcess::ran("", "error: failed to load manifest", 1),
        ]));
        assert!(!locally(&repos, &process).await.0);
        assert!(!repos.asked("commit"));
        let repos = FakeRepos::conflicting(&[REGISTRY], &["push"]);
        let (held, said) = locally(&repos, &Rc::new(FakeProcess::default())).await;
        assert!(!held);
        assert!(said[0].contains("push failed: "), "{said:?}");
    }

    #[tokio::test]
    async fn a_merge_github_refused_and_git_takes_is_pushed_as_it_is() {
        let repos = FakeRepos::default();
        let process = Rc::new(FakeProcess::default());
        let (held, said) = locally(&repos, &process).await;
        assert!(held);
        assert!(process.ran.borrow().is_empty());
        assert!(repos.asked("push milestone/17-ai") && !repos.asked("commit"));
        assert_eq!(said, ["milestone/17-ai holds main_agent"]);
    }

    #[tokio::test]
    async fn a_merge_that_fails_on_no_file_or_a_branch_that_cannot_be_read_is_refused() {
        let repos = FakeRepos::failing(&["merge"]);
        let (held, said) = locally(&repos, &Rc::new(FakeProcess::default())).await;
        assert!(!held);
        assert!(
            said[0].ends_with("the merge failed: fatal: merge origin/main_agent failed"),
            "{said:?}"
        );
        assert!(!repos.asked("abort_merge"), "no merge in progress");
        for step in ["fetch", "checkout", "reset_hard"] {
            let repos = FakeRepos::failing(&[step]);
            assert!(
                !locally(&repos, &Rc::new(FakeProcess::default())).await.0,
                "{step}"
            );
            assert!(!repos.asked("merge"), "{step}");
        }
    }

    #[test]
    fn the_unmerged_paths_are_read_from_their_status_codes() {
        let status: Vec<String> = [
            "UU Cargo.lock",
            "AA crates/a/Cargo.toml",
            "DU b",
            "UD c",
            "AU d",
            "UA e",
            "DD f",
            "M  staged.rs",
            " M changed.rs",
            "?? new.rs",
            "UU \"with space\"",
            "U",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            unmerged(&status),
            [
                "Cargo.lock",
                "crates/a/Cargo.toml",
                "b",
                "c",
                "d",
                "e",
                "f",
                "with space"
            ]
        );
    }

    #[tokio::test]
    async fn a_milestone_branch_takes_the_integration_branch_in() {
        let gh = FakeGitHub::default();
        assert!(catch_up(&gh, "milestone/17-ai", "main_agent", &Logbook::null()).await);
        assert!(
            gh.wrote.borrow().iter().any(|w| matches!(
                w,
                Wrote::MergedBranch(base, head) if base == "milestone/17-ai" && head == "main_agent"
            )),
            "{:?}",
            gh.wrote.borrow()
        );
    }

    #[tokio::test]
    async fn a_merge_github_refuses_is_said_and_answered_false() {
        let gh = FakeGitHub {
            refused_merges: vec!["milestone/20-sessions".to_string()],
            ..FakeGitHub::default()
        };
        assert!(
            !catch_up(&gh, "milestone/20-sessions", "main_agent", &Logbook::null()).await,
            "a conflict: the branch does not hold the base"
        );
        assert!(gh.wrote.borrow().is_empty(), "{:?}", gh.wrote.borrow());
    }

    #[test]
    fn a_plain_title_becomes_a_lowercase_slug() {
        assert_eq!(
            milestone_branch(4, "Territory tooling"),
            "milestone/4-territory-tooling"
        );
    }

    #[test]
    fn punctuation_collapses_to_a_single_dash() {
        assert_eq!(
            slugify("Schema, seed & first DB-backed page"),
            "schema-seed-first-db-backed-page"
        );
    }

    #[test]
    fn leading_and_trailing_punctuation_is_trimmed() {
        assert_eq!(slugify("  — Territory tooling! —  "), "territory-tooling");
    }

    #[test]
    fn repeated_separators_do_not_repeat_dashes() {
        assert_eq!(slugify("a   b---c"), "a-b-c");
    }

    #[test]
    fn two_different_titles_never_collide_by_accident_in_the_common_case() {
        assert_ne!(
            milestone_branch(4, "Territory tooling"),
            milestone_branch(4, "Character creation")
        );
    }
}
