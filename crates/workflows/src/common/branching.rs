//! The branch name a milestone pushes its tasks' PRs into.
//!
//! Shared because `split` writes it into each task's body (as a `branch:`
//! line naming the *task's own* branch, computed by the session, not this
//! module) while `init-repo`'s audit and `main_agent_merge` both need the
//! *milestone's own* branch name — derived, never asked of a session, so
//! the two sides can never disagree on what it is. And the local half of
//! what keeps that branch up to the integration branch
//! ([`catch_up_locally`], when GitHub refuses the merge): the generated
//! files (`harness_architecture::generated`) regenerated, any other
//! conflict left as it is — the route that decides when is
//! `common::catch_up`.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use harness_architecture::generated::{self, Regenerator, Take};
use harness_core::domain::Outcome;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repo;
use harness_core::ports::shell::process::{Process, Ran};
use harness_core::traces::Logbook;

/// What one catch-up of a milestone branch came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaughtUp {
    /// GitHub merged the integration branch in, or the branch already held
    /// every commit of it.
    OnGitHub,
    /// GitHub refused; the merge was made in the harness's checkout and
    /// pushed, these generated files regenerated (none: git merged what
    /// GitHub would not).
    Locally(Vec<String>),
    /// A conflict no script solves, on these files: the merge was aborted,
    /// the branch is as it was.
    Residual(Vec<String>),
    /// A step failed — the checkout, a generator, the push: why. The merge
    /// was aborted.
    Failed(String),
    /// Nothing done yet, tried again at a later tick: this tick's local
    /// merge is spent, a lane works on the landing PR, the conflict met at
    /// this base still holds on GitHub, or the branch moved on the remote
    /// while the harness merged it (its push refused as a
    /// non-fast-forward: a session's push landed meanwhile).
    Deferred,
}

/// How long `scripts/register.sh` may run: it reads a few folders.
const REGISTER_TIMEOUT: Duration = Duration::from_mins(1);
/// How long `scripts/queries.sh` may run: it hashes the persisted queries.
const QUERIES_TIMEOUT: Duration = Duration::from_mins(2);
/// How long `mock/build.sh` may run: one `npx esbuild` per bundle, the
/// first fetching it.
const MOCK_BUILD_TIMEOUT: Duration = Duration::from_mins(15);
/// How long `cargo metadata` may run: it may fetch the engine by git.
const METADATA_TIMEOUT: Duration = Duration::from_mins(10);

/// How long a generator may run.
const fn timeout(regenerator: Regenerator) -> Duration {
    match regenerator {
        Regenerator::Register => REGISTER_TIMEOUT,
        Regenerator::Queries => QUERIES_TIMEOUT,
        Regenerator::MockBuild => MOCK_BUILD_TIMEOUT,
        Regenerator::CargoMetadata => METADATA_TIMEOUT,
    }
}

/// A checkout of the product, the harness's own, where a refused catch-up
/// is tried again.
pub struct Checkout {
    /// Its `git`.
    pub repo: Rc<dyn Repo>,
    /// What runs the product's scripts and `cargo` in it — with the
    /// toolchain the product pins.
    pub process: Rc<dyn Process>,
    /// What reads and writes its files: a `mock/pages.json` whose digests
    /// alone conflict.
    pub disk: Rc<dyn Disk>,
    /// Its root.
    pub root: PathBuf,
}

/// Brings `milestone_branch` up to `head` in `checkout`, as GitHub's merge
/// would, regenerating the generated files that conflict.
///
/// `head` is the integration branch's commit the watch read; the generated
/// files are `harness_architecture::generated::GENERATED`. Each conflicted generated file is taken from the base — removed when the
/// base deleted it, `mock/pages.json` only when its digests are all that
/// differ — then its generators run on the merged tree, in their order
/// (`scripts/register.sh`, `scripts/queries.sh`, `mock/build.sh`,
/// `cargo metadata`), and the merge is committed and pushed:
/// [`CaughtUp::Locally`]. A conflict on any other file is
/// [`CaughtUp::Residual`], and a step that fails [`CaughtUp::Failed`]:
/// either aborts the merge, never hand-merges it. A push refused because
/// the branch moved on the remote meanwhile is [`CaughtUp::Deferred`]: the
/// next tick merges again on top of it.
pub async fn catch_up_locally(
    checkout: &Checkout,
    milestone_branch: &str,
    integration: &str,
    head: &str,
    log: &Logbook,
) -> CaughtUp {
    let merging = Merging {
        branch: milestone_branch,
        integration,
        head,
    };
    let stopped = match regenerate(checkout, merging).await {
        Ok(Merge::Held(regenerated)) => {
            if regenerated.is_empty() {
                log.say(&format!("{milestone_branch} holds {integration}"));
            } else {
                log.say(&format!(
                    "{milestone_branch} holds {integration}: {} regenerated, never merged by hand",
                    regenerated.join(" and ")
                ));
            }
            return CaughtUp::Locally(regenerated);
        }
        Ok(Merge::Raced) => {
            log.say(&format!(
                "{milestone_branch} moved while it took {integration} — merged again at the next \
                 tick"
            ));
            return CaughtUp::Deferred;
        }
        Ok(Merge::Residual(files)) => CaughtUp::Residual(files),
        Err(why) => CaughtUp::Failed(why),
    };
    if checkout.repo.merge_in_progress().await.unwrap_or(true) {
        let _ = checkout.repo.abort_merge().await;
    }
    stopped
}

/// What the merge of [`catch_up_locally`] came to, short of a failure.
enum Merge {
    /// Committed and pushed, these generated files regenerated.
    Held(Vec<String>),
    /// Committed, its push refused: the branch moved on the remote since
    /// the fetch.
    Raced,
    /// Left in progress: these files conflict and no script solves them.
    Residual(Vec<String>),
}

/// The merge [`catch_up_locally`] makes.
#[derive(Clone, Copy)]
struct Merging<'a> {
    /// The milestone branch.
    branch: &'a str,
    /// The integration branch, as messages name it.
    integration: &'a str,
    /// Its commit merged in.
    head: &'a str,
}

/// A path `status --porcelain` reports unmerged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Unmerged {
    /// Its path.
    path: String,
    /// Whether the side merged in has no such file (`UD` deleted by them,
    /// `DD` by both, `AU` added by us alone): taking that side is removing
    /// it, `checkout --theirs` refuses.
    theirs_gone: bool,
}

/// The merge of [`catch_up_locally`], or why it stopped.
async fn regenerate(checkout: &Checkout, merging: Merging<'_>) -> Result<Merge, String> {
    let Merging {
        branch,
        integration,
        head,
    } = merging;
    let repo = &checkout.repo;
    done("fetch", repo.fetch().await)?;
    done("checkout", repo.checkout(branch, true).await)?;
    done("reset", repo.reset_hard(&format!("origin/{branch}")).await)?;
    let merged = repo
        .merge(head)
        .await
        .map_err(|halt| halt.reason().to_string())?;
    if merged.ok() {
        return pushed(repo.as_ref(), branch, Merge::Held(Vec::new())).await;
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
    // What each conflicted file is taken as, or why it is no script's.
    let mut residual = Vec::new();
    let mut resolved = Vec::new();
    for file in &conflicted {
        match generated::generated(&file.path).map(|kind| kind.take) {
            None => residual.push(file.path.clone()),
            Some(_) if file.theirs_gone => {}
            Some(Take::Theirs) => {}
            Some(Take::DigestsOnly) => match checkout
                .disk
                .read_to_string(&checkout.root.join(&file.path))
                .as_deref()
                .and_then(generated::digests_only)
            {
                Some(text) => resolved.push((file.path.clone(), text)),
                None => residual.push(file.path.clone()),
            },
        }
    }
    if !residual.is_empty() {
        return Ok(Merge::Residual(residual));
    }
    for file in &conflicted {
        let path = &file.path;
        if file.theirs_gone {
            done(&format!("removing {path}"), repo.remove(path).await)?;
        } else if let Some((_, text)) = resolved.iter().find(|(p, _)| p == path) {
            checkout
                .disk
                .write_to_string(&checkout.root.join(path), text)
                .map_err(|halt| format!("{path} cannot be written: {}", halt.reason()))?;
        } else {
            done(&format!("the base's {path}"), repo.take_theirs(path).await)?;
        }
    }
    let paths: Vec<String> = conflicted.into_iter().map(|file| file.path).collect();
    for regenerator in generated::regenerators(&paths) {
        let args: Vec<String> = regenerator
            .args()
            .iter()
            .map(|arg| (*arg).to_string())
            .collect();
        done(
            &regenerator.command(),
            checkout
                .process
                .run(
                    regenerator.program(),
                    &args,
                    &checkout.root,
                    timeout(regenerator),
                )
                .await,
        )?;
    }
    done("stage", repo.stage_all().await)?;
    done(
        "commit",
        repo.commit(&format!(
            "chore: merge {integration} ({head}) into {branch}, {} regenerated",
            paths.join(", ")
        ))
        .await,
    )?;
    pushed(repo.as_ref(), branch, Merge::Held(paths)).await
}

/// Pushes `branch` — never forced: `held` once it went, [`Merge::Raced`]
/// when the remote refused it as a non-fast-forward (`[rejected]`: a push
/// of a session landed there since the fetch), why on any other refusal.
async fn pushed(repo: &dyn Repo, branch: &str, held: Merge) -> Result<Merge, String> {
    match repo.push(branch).await {
        Ok(ran) if !ran.ok() && ran.stderr.contains("[rejected]") => Ok(Merge::Raced),
        ran => done("push", ran).map(|()| held),
    }
}

/// A step that ran and succeeded, or why it did not.
fn done(step: &str, ran: Outcome<Ran>) -> Result<(), String> {
    match ran {
        Ok(ran) if ran.ok() => Ok(()),
        Ok(ran) => Err(format!("{step} failed: {}", ran.why())),
        Err(halt) => Err(format!("{step} failed: {}", halt.reason())),
    }
}

/// The paths `status --porcelain` reports unmerged: `UU`, `AA`, `DU`, …,
/// each with whether the side merged in has it.
fn unmerged(status: &[String]) -> Vec<Unmerged> {
    status
        .iter()
        .filter_map(|line| {
            let (code, path) = (line.get(..2)?, line.get(3..)?);
            matches!(code, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU").then(|| Unmerged {
                path: path.trim_matches('"').to_string(),
                theirs_gone: matches!(code, "UD" | "DD" | "AU"),
            })
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
    use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_process::FakeProcess;
    use harness_architecture::tree::REGISTRY;
    use harness_core::ports::shell::git::Repos as _;
    use harness_core::traces::{Sink, Verbosity};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::Path;

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    const ROOT: &str = "/w/catch-up";

    /// A `mock/pages.json` whose one conflict is a digest.
    const PAGES_DIGESTS: &str = "{\n<<<<<<< HEAD\n  \"builtFrom\": \"aa\"\n=======\n  \"builtFrom\": \"bb\"\n>>>>>>> origin/main_agent\n}\n";

    /// Catches `milestone/17-ai` up with `main_agent` in a checkout of
    /// `repos`, the product's commands answered by `process`, its files by
    /// `disk`: what it came to, and what the journal said.
    async fn locally_on(
        repos: &FakeRepos,
        process: &Rc<FakeProcess>,
        disk: &Rc<FakeDisk>,
    ) -> (CaughtUp, Vec<String>) {
        let root = Path::new(ROOT);
        let checkout = Checkout {
            repo: repos.at(root),
            process: Rc::clone(process) as Rc<dyn Process>,
            disk: Rc::clone(disk) as Rc<dyn Disk>,
            root: root.to_path_buf(),
        };
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        let caught = catch_up_locally(&checkout, "milestone/17-ai", "main_agent", "h1", &log).await;
        let said = capture.0.borrow().clone();
        (caught, said)
    }

    /// [`locally_on`] with an empty disk.
    async fn locally(repos: &FakeRepos, process: &Rc<FakeProcess>) -> (CaughtUp, Vec<String>) {
        locally_on(repos, process, &Rc::new(FakeDisk::default())).await
    }

    /// A disk holding a conflicted `mock/pages.json`.
    fn pages(text: &str) -> Rc<FakeDisk> {
        Rc::new(FakeDisk {
            existing: HashMap::from([(Path::new(ROOT).join("mock/pages.json"), text.to_string())]),
            ..FakeDisk::default()
        })
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
                assert_eq!(root, Path::new(ROOT), "run in the checkout");
                format!("{program} {}", args.join(" "))
            })
            .collect()
    }

    #[tokio::test]
    async fn a_conflict_on_the_generated_pair_only_is_regenerated_and_committed() {
        let repos = FakeRepos::conflicting(&["Cargo.lock", REGISTRY], &[]);
        let process = Rc::new(FakeProcess::default());
        let (caught, said) = locally(&repos, &process).await;
        assert_eq!(
            caught,
            CaughtUp::Locally(vec!["Cargo.lock".to_string(), REGISTRY.to_string()])
        );
        assert_eq!(
            verbs(&repos),
            [
                "fetch",
                "checkout milestone/17-ai --force",
                "reset_hard origin/milestone/17-ai",
                "merge h1",
                "take_theirs Cargo.lock",
                "take_theirs crates/dataguard/registry/Cargo.toml",
                "stage_all",
                "commit chore: merge main_agent (h1) into milestone/17-ai, Cargo.lock, \
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
    async fn every_generated_kind_is_taken_from_the_base_and_regenerated_in_order() {
        let conflicted = [
            "Cargo.lock",
            REGISTRY,
            "queries/registry.json",
            "mock/pages.json",
            "mock/components/Battlemap.js",
        ];
        let repos = FakeRepos::conflicting(&conflicted, &[]);
        let process = Rc::new(FakeProcess::default());
        let disk = pages(PAGES_DIGESTS);
        let (caught, _) = locally_on(&repos, &process, &disk).await;
        assert_eq!(
            caught,
            CaughtUp::Locally(conflicted.iter().map(|p| (*p).to_string()).collect())
        );
        let verbs = verbs(&repos);
        for path in [
            "Cargo.lock",
            REGISTRY,
            "queries/registry.json",
            "mock/components/Battlemap.js",
        ] {
            assert!(verbs.contains(&format!("take_theirs {path}")), "{verbs:?}");
        }
        assert!(
            !verbs.contains(&"take_theirs mock/pages.json".to_string()),
            "the manifest is resolved, never taken whole: {verbs:?}"
        );
        assert_eq!(
            disk.written_to(&Path::new(ROOT).join("mock/pages.json"))
                .as_deref(),
            Some("{\n  \"builtFrom\": \"bb\"\n}\n")
        );
        assert_eq!(
            programs(&process),
            [
                "bash scripts/register.sh",
                "bash scripts/queries.sh",
                "sh mock/build.sh",
                "cargo metadata --format-version 1"
            ]
        );
        let tail: Vec<&str> = verbs.iter().rev().take(3).map(String::as_str).collect();
        assert!(tail[0] == "push milestone/17-ai" && tail[1].starts_with("commit "));
        assert_eq!(tail[2], "stage_all");
    }

    #[tokio::test]
    async fn each_kind_runs_its_own_generators_only() {
        let repos = FakeRepos::conflicting(&["queries/registry.json"], &[]);
        let process = Rc::new(FakeProcess::default());
        assert!(matches!(
            locally(&repos, &process).await.0,
            CaughtUp::Locally(_)
        ));
        assert_eq!(programs(&process), ["bash scripts/queries.sh"]);
        let repos = FakeRepos::conflicting(&["mock/components/A.js"], &[]);
        let process = Rc::new(FakeProcess::default());
        assert!(matches!(
            locally(&repos, &process).await.0,
            CaughtUp::Locally(_)
        ));
        assert_eq!(programs(&process), ["sh mock/build.sh"]);
        let repos = FakeRepos::conflicting(&[REGISTRY], &[]);
        let process = Rc::new(FakeProcess::default());
        assert!(matches!(
            locally(&repos, &process).await.0,
            CaughtUp::Locally(_)
        ));
        assert!(repos.asked("take_theirs crates/dataguard/registry/Cargo.toml"));
        assert_eq!(programs(&process).len(), 2, "the registry, then the lock");
    }

    #[tokio::test]
    async fn a_manifest_conflict_beyond_its_digests_is_residual_and_aborted() {
        let repos = FakeRepos::conflicting(&["mock/pages.json", "Cargo.lock"], &[]);
        let process = Rc::new(FakeProcess::default());
        let disk = pages("<<<<<<< HEAD\n\"name\": \"A\"\n=======\n\"name\": \"B\"\n>>>>>>> m\n");
        let (caught, said) = locally_on(&repos, &process, &disk).await;
        assert_eq!(
            caught,
            CaughtUp::Residual(vec!["mock/pages.json".to_string()])
        );
        assert!(said.is_empty(), "the caller says it: {said:?}");
        assert!(
            process.ran.borrow().is_empty() && disk.written.borrow().is_empty(),
            "nothing expected"
        );
        assert!(repos.asked("abort_merge"));
        assert!(!repos.asked("take_theirs") && !repos.asked("commit") && !repos.asked("push"));
        // A manifest that cannot be read is no digest either.
        let repos = FakeRepos::conflicting(&["mock/pages.json"], &[]);
        let (caught, _) = locally(&repos, &Rc::new(FakeProcess::default())).await;
        assert_eq!(
            caught,
            CaughtUp::Residual(vec!["mock/pages.json".to_string()])
        );
    }

    #[tokio::test]
    async fn a_rust_source_conflict_is_residual_never_merged_by_hand() {
        let repos = FakeRepos::conflicting(
            &[
                "Cargo.lock",
                REGISTRY,
                "crates/credit/src/lib.rs",
                "mock/src/commands.js",
            ],
            &[],
        );
        let process = Rc::new(FakeProcess::default());
        let (caught, said) = locally(&repos, &process).await;
        assert_eq!(
            caught,
            CaughtUp::Residual(vec![
                "crates/credit/src/lib.rs".to_string(),
                "mock/src/commands.js".to_string()
            ])
        );
        assert!(said.is_empty(), "nothing expected");
        assert!(process.ran.borrow().is_empty(), "nothing regenerated");
        assert!(repos.asked("abort_merge"));
        assert!(!repos.asked("take_theirs") && !repos.asked("commit") && !repos.asked("push"));
    }

    #[tokio::test]
    async fn a_failing_generator_fails_and_aborts_the_merge() {
        let repos = FakeRepos::conflicting(&["Cargo.lock", REGISTRY], &[]);
        let process = Rc::new(FakeProcess::answering(vec![FakeProcess::ran(
            "",
            "::error file=crates/x/Cargo.toml::no [package] name",
            1,
        )]));
        let (caught, said) = locally(&repos, &process).await;
        let CaughtUp::Failed(why) = caught else {
            panic!("a failure: {caught:?}");
        };
        assert!(
            why.starts_with("bash scripts/register.sh failed: ::error"),
            "{why}"
        );
        assert_eq!(
            programs(&process),
            ["bash scripts/register.sh"],
            "no metadata after it"
        );
        assert!(repos.asked("abort_merge"));
        assert!(!repos.asked("commit") && !repos.asked("push"));
        assert!(said.is_empty(), "nothing expected");
    }

    #[tokio::test]
    async fn a_failing_metadata_or_push_fails() {
        let repos = FakeRepos::conflicting(&[REGISTRY], &[]);
        let process = Rc::new(FakeProcess::answering(vec![
            FakeProcess::ran("", "", 0),
            FakeProcess::ran("", "error: failed to load manifest", 1),
        ]));
        assert!(matches!(
            locally(&repos, &process).await.0,
            CaughtUp::Failed(_)
        ));
        assert!(!repos.asked("commit"));
        let repos = FakeRepos::conflicting(&[REGISTRY], &["push"]);
        let (caught, _) = locally(&repos, &Rc::new(FakeProcess::default())).await;
        assert!(
            matches!(&caught, CaughtUp::Failed(why) if why.contains("push failed: ")),
            "{caught:?}"
        );
        let repos = FakeRepos::conflicting(&["Cargo.lock"], &["take_theirs"]);
        let (caught, _) = locally(&repos, &Rc::new(FakeProcess::default())).await;
        assert!(
            matches!(&caught, CaughtUp::Failed(why) if why.starts_with("the base's Cargo.lock failed")),
            "{caught:?}"
        );
    }

    #[tokio::test]
    async fn a_merge_github_refused_and_git_takes_is_pushed_as_it_is() {
        let repos = FakeRepos::default();
        let process = Rc::new(FakeProcess::default());
        let (caught, said) = locally(&repos, &process).await;
        assert_eq!(caught, CaughtUp::Locally(Vec::new()));
        assert!(process.ran.borrow().is_empty(), "nothing expected");
        assert!(repos.asked("push milestone/17-ai") && !repos.asked("commit"));
        assert_eq!(said, ["milestone/17-ai holds main_agent"]);
    }

    #[tokio::test]
    async fn a_merge_that_fails_on_no_file_or_a_branch_that_cannot_be_read_fails() {
        let repos = FakeRepos::failing(&["merge"]);
        let (caught, _) = locally(&repos, &Rc::new(FakeProcess::default())).await;
        assert!(
            matches!(&caught, CaughtUp::Failed(why)
                if why.ends_with("the merge failed: fatal: merge h1 failed")),
            "{caught:?}"
        );
        assert!(!repos.asked("abort_merge"), "no merge in progress");
        for step in ["fetch", "checkout", "reset_hard"] {
            let repos = FakeRepos::failing(&[step]);
            assert!(
                matches!(
                    locally(&repos, &Rc::new(FakeProcess::default())).await.0,
                    CaughtUp::Failed(_)
                ),
                "{step}"
            );
            assert!(!repos.asked("merge"), "{step}");
        }
    }

    #[tokio::test]
    async fn a_generated_file_the_base_deleted_is_removed_then_regenerated_around() {
        let repos = FakeRepos::conflicting_as(
            &[
                ("UD", "mock/components/Gone.js"),
                ("DD", "mock/components/Both.js"),
                ("UU", "mock/components/Kept.js"),
            ],
            &[],
        );
        let process = Rc::new(FakeProcess::default());
        let (caught, _) = locally(&repos, &process).await;
        assert!(matches!(caught, CaughtUp::Locally(_)), "{caught:?}");
        assert!(repos.asked("remove mock/components/Gone.js"));
        assert!(repos.asked("remove mock/components/Both.js"));
        assert!(
            !repos.asked("take_theirs mock/components/Gone.js")
                && !repos.asked("take_theirs mock/components/Both.js"),
            "checkout --theirs refuses a file theirs has not"
        );
        assert!(repos.asked("take_theirs mock/components/Kept.js"));
        assert!(!repos.asked("remove mock/components/Kept.js"));
        assert_eq!(programs(&process), ["sh mock/build.sh"]);
        // The manifest the base deleted is removed too, never read for its
        // digests.
        let repos = FakeRepos::conflicting_as(&[("UD", "mock/pages.json")], &[]);
        let (caught, _) = locally(&repos, &Rc::new(FakeProcess::default())).await;
        assert!(matches!(caught, CaughtUp::Locally(_)), "{caught:?}");
        assert!(repos.asked("remove mock/pages.json"));
        // A deleted file that is no generated one is still residual.
        let repos = FakeRepos::conflicting_as(&[("UD", "crates/x/src/lib.rs")], &[]);
        assert_eq!(
            locally(&repos, &Rc::new(FakeProcess::default())).await.0,
            CaughtUp::Residual(vec!["crates/x/src/lib.rs".to_string()])
        );
        assert!(!repos.asked("remove"));
        // A removal git refuses fails the catch-up.
        let repos = FakeRepos::conflicting_as(&[("UD", "Cargo.lock")], &["remove"]);
        let (caught, _) = locally(&repos, &Rc::new(FakeProcess::default())).await;
        assert!(
            matches!(&caught, CaughtUp::Failed(why) if why.starts_with("removing Cargo.lock failed")),
            "{caught:?}"
        );
    }

    #[tokio::test]
    async fn a_push_the_remote_refuses_as_behind_is_deferred_not_failed() {
        let repos = FakeRepos::conflicting(&["Cargo.lock"], &[]).rejecting_push();
        let (caught, said) = locally(&repos, &Rc::new(FakeProcess::default())).await;
        assert_eq!(caught, CaughtUp::Deferred);
        assert_eq!(
            said,
            ["milestone/17-ai moved while it took main_agent — merged again at the next tick"]
        );
        assert!(repos.asked("commit") && repos.asked("push milestone/17-ai"));
        // The same race on a merge git took whole.
        let repos = FakeRepos::default().rejecting_push();
        assert_eq!(
            locally(&repos, &Rc::new(FakeProcess::default())).await.0,
            CaughtUp::Deferred
        );
        // Any other refusal of the push is a failure.
        let repos = FakeRepos::conflicting(&["Cargo.lock"], &["push"]);
        assert!(matches!(
            locally(&repos, &Rc::new(FakeProcess::default())).await.0,
            CaughtUp::Failed(_)
        ));
    }

    #[test]
    fn each_generator_has_its_own_time() {
        assert_eq!(timeout(Regenerator::Register), Duration::from_mins(1));
        assert_eq!(timeout(Regenerator::Queries), Duration::from_mins(2));
        assert_eq!(timeout(Regenerator::MockBuild), Duration::from_mins(15));
        assert_eq!(timeout(Regenerator::CargoMetadata), Duration::from_mins(10));
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
        let read = unmerged(&status);
        let read: Vec<(&str, bool)> = read
            .iter()
            .map(|file| (file.path.as_str(), file.theirs_gone))
            .collect();
        assert_eq!(
            read,
            [
                ("Cargo.lock", false),
                ("crates/a/Cargo.toml", false),
                ("b", false),
                ("c", true),
                ("d", true),
                ("e", false),
                ("f", true),
                ("with space", false)
            ]
        );
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
