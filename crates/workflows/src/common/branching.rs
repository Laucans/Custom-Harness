//! The branch name a milestone pushes its tasks' PRs into.
//!
//! Shared because `split` writes it into each task's body (as a `branch:`
//! line naming the *task's own* branch, computed by the session, not this
//! module) while `init-repo`'s audit and `main_agent_merge` both need the
//! *milestone's own* branch name — derived, never asked of a session, so
//! the two sides can never disagree on what it is. And the local half of
//! what keeps that branch up to the integration branch
//! ([`catch_up_locally`], when GitHub refuses the merge): the generated
//! files (`harness_architecture::generated`) regenerated, the head's
//! legacy handler split taken first when the merge meets it
//! (`common::handler_split`), any other conflict left as it is — the route
//! that decides when is `common::catch_up`.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use harness_architecture::generated::{self, Regenerator, Take};
use harness_architecture::mock_handlers;
use harness_core::domain::Outcome;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repo;
use harness_core::ports::shell::process::{Process, Ran};
use harness_core::traces::Logbook;

use crate::common::handler_split;

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

/// The integration branch's commit a catch-up merges, and what its changes
/// stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseHead {
    /// The commit the watch read.
    pub sha: String,
    /// The generators that run whatever conflicts: the base changed what
    /// they read (`generated::staled_by`).
    pub rebuild: Vec<Regenerator>,
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

/// Brings `milestone_branch` up to `base` in `checkout`, as GitHub's merge
/// would, regenerating the generated files that conflict — and those of
/// its `rebuild` even when nothing conflicts.
///
/// `base.sha` is the integration branch's commit the watch read; the generated
/// files are `harness_architecture::generated::GENERATED`. Each conflicted
/// generated file is taken from the base — removed when the base deleted
/// it, `mock/pages.json` only when its digests are all that differ — then
/// its generators run on the merged tree, in their order
/// (`scripts/register.sh`, `scripts/queries.sh`, `mock/build.sh`,
/// `cargo metadata`), and the merge is committed and pushed:
/// [`CaughtUp::Locally`]. `base.rebuild` names the generators the base's own
/// changes stale (`generated::staled_by`): they run too, after a clean
/// merge as well — `mock/build.sh` only in a product that builds bundles
/// (a `builtFrom` in the merged `mock/pages.json`) — and what they rewrite
/// is committed on top of git's merge. A conflict on any other file is
/// [`CaughtUp::Residual`], and a step that fails [`CaughtUp::Failed`]:
/// either aborts the merge, never hand-merges it. A push refused because
/// the branch moved on the remote meanwhile is [`CaughtUp::Deferred`]: the
/// next tick merges again on top of it. A merge that meets the head's
/// legacy handler split (a legacy file the branch changed and the head
/// deleted) splits the branch first and merges again, the handler files
/// both sides added merged three ways on the split merge base
/// (`common::handler_split`), the bundles rebuilt; the splitter refusing
/// the branch is [`CaughtUp::Residual`] on the legacy files. Whatever is
/// not [`CaughtUp::Locally`] leaves the checkout on the branch as the
/// remote has it: no unpushed commit to read as work kept. The merge runs
/// on a detached `HEAD` at `origin/<branch>` and is pushed as the branch,
/// so a run killed midway leaves its commits on no local branch either.
pub async fn catch_up_locally(
    checkout: &Checkout,
    milestone_branch: &str,
    integration: &str,
    base: &BaseHead,
    log: &Logbook,
) -> CaughtUp {
    let merging = Merging {
        branch: milestone_branch,
        integration,
        head: &base.sha,
        rebuild: &base.rebuild,
    };
    let stopped = match regenerate(checkout, merging).await {
        Ok(Merge::Held(held)) => {
            log.say(&held.said(milestone_branch, integration));
            return CaughtUp::Locally(held.regenerated);
        }
        Ok(Merge::Raced) => {
            log.say(&format!(
                "{milestone_branch} moved while it took {integration} — merged again at the next \
                 tick"
            ));
            CaughtUp::Deferred
        }
        Ok(Merge::Residual(files, unsplit)) => {
            if let Some(why) = unsplit {
                log.say(&format!(
                    "{milestone_branch} cannot merge the handler files both sides added three \
                     ways ({why}): they stay a hand's"
                ));
            }
            CaughtUp::Residual(files)
        }
        Ok(Merge::Refused(files, why)) => {
            log.say(&format!(
                "{milestone_branch} cannot take {integration}'s handler split: the splitter \
                 refused its tree ({why})"
            ));
            CaughtUp::Residual(files)
        }
        Err(why) => CaughtUp::Failed(why),
    };
    let repo = &checkout.repo;
    if repo.merge_in_progress().await.unwrap_or(true) {
        let _ = repo.abort_merge().await;
    }
    // Back to the branch as the remote has it. The merge ran on a detached
    // `HEAD`, so no local branch holds what it committed; this also puts
    // back a local branch an older run left ahead, and leaves no detached
    // merge base to read as a checkout left astray.
    let _ = repo.checkout(milestone_branch, true).await;
    let _ = repo.reset_hard(&format!("origin/{milestone_branch}")).await;
    let _ = repo.clean().await;
    stopped
}

/// What the merge of [`catch_up_locally`] came to, short of a failure.
enum Merge {
    /// Committed and pushed.
    Held(Held),
    /// Committed, its push refused: the branch moved on the remote since
    /// the fetch.
    Raced,
    /// Left in progress: these files conflict and no script solves them —
    /// and why the merge base could not be split, when the handler files
    /// both sides added needed it.
    Residual(Vec<String>, Option<String>),
    /// The head split its legacy handler files and the splitter refused
    /// the branch's tree: these legacy files, and why.
    Refused(Vec<String>, String),
}

/// A merge [`catch_up_locally`] committed and pushed.
#[derive(Default)]
struct Held {
    /// The generated files regenerated.
    regenerated: Vec<String>,
    /// The handler files merged three ways on the split merge base.
    three_way: Vec<String>,
    /// Whether the branch took the head's handler split first.
    split: bool,
    /// Why the merge base could not be split while handler files both
    /// sides added needed it: they stay a hand's.
    unsplit: Option<String>,
}

impl Held {
    /// The message of its merge commit.
    fn message(&self, branch: &str, integration: &str, head: &str) -> String {
        let mut message = format!("chore: merge {integration} ({head}) into {branch}");
        if !self.regenerated.is_empty() {
            message = format!("{message}, {} regenerated", self.regenerated.join(", "));
        }
        if !self.three_way.is_empty() {
            message = format!(
                "{message}, {} merged three ways on the split merge base",
                self.three_way.join(", ")
            );
        }
        message
    }

    /// What the journal says of it.
    fn said(&self, branch: &str, integration: &str) -> String {
        let mut parts = Vec::new();
        if self.split {
            parts.push("its legacy handler files split first".to_string());
        }
        if !self.three_way.is_empty() {
            parts.push(format!(
                "{} merged three ways on the split merge base",
                self.three_way.join(" and ")
            ));
        }
        if !self.regenerated.is_empty() {
            parts.push(format!("{} regenerated", self.regenerated.join(" and ")));
        }
        if parts.is_empty() {
            return format!("{branch} holds {integration}");
        }
        format!(
            "{branch} holds {integration}: {}, never merged by hand",
            parts.join(", ")
        )
    }
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
    /// The generators that run whatever conflicts: the base changed what
    /// they read.
    rebuild: &'a [Regenerator],
}

/// A path `status --porcelain` reports unmerged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unmerged {
    /// Its path.
    pub(crate) path: String,
    /// Its two-letter status code: `UU`, `AA`, `UD`, `DU`, `DD`, `AU`,
    /// `UA`.
    pub(crate) code: String,
}

impl Unmerged {
    /// Whether the side merged in has no such file (`UD` deleted by them,
    /// `DD` by both, `AU` added by us alone): taking that side is removing
    /// it, `checkout --theirs` refuses.
    pub(crate) fn theirs_gone(&self) -> bool {
        matches!(self.code.as_str(), "UD" | "DD" | "AU")
    }

    /// Whether both sides added it, no merge base having it (`AA`).
    pub(crate) fn both_added(&self) -> bool {
        self.code == "AA"
    }
}

/// The merge of [`catch_up_locally`], or why it stopped.
///
/// A merge that meets the head's handler split (a legacy handler file the
/// branch changed and the head deleted) takes it first
/// (`handler_split::split_receiving`) and merges again; the handler files
/// both sides then added merge three ways on the split merge base
/// (`handler_split::merge_on_split_base`), and the mock-up's bundles are
/// rebuilt whatever the second merge conflicts on.
async fn regenerate(checkout: &Checkout, merging: Merging<'_>) -> Result<Merge, String> {
    let Merging {
        branch,
        integration,
        head,
        rebuild,
    } = merging;
    let repo = &checkout.repo;
    done("fetch", repo.fetch().await)?;
    // On a detached `HEAD` at the remote's tip, pushed as the branch: a
    // run killed before its push leaves its commits on no local branch,
    // never work kept for every later release to warn of.
    done(
        "checkout",
        repo.checkout(&format!("origin/{branch}"), true).await,
    )?;
    let mut held = Held::default();
    let (merged, conflicted) = match merge_across_the_split(checkout, merging, &mut held).await? {
        Left::Merged(merged, conflicted) => (merged, conflicted),
        Left::Refused(legacy, why) => return Ok(Merge::Refused(legacy, why)),
    };
    let mut rebuild = rebuild.to_vec();
    if held.split && !rebuild.contains(&Regenerator::MockBuild) {
        rebuild.push(Regenerator::MockBuild);
    }
    let rebuild = applicable(checkout, &rebuild);
    if merged.ok() {
        return rebuilt(checkout, merging, &rebuild, held).await;
    }
    // What each conflicted file is taken as, or why it is no script's.
    let mut residual = Vec::new();
    let mut resolved = Vec::new();
    for file in &conflicted {
        match generated::generated(&file.path).map(|kind| kind.take) {
            None => residual.push(file.path.clone()),
            Some(_) if file.theirs_gone() => {}
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
        return Ok(Merge::Residual(residual, held.unsplit));
    }
    for file in &conflicted {
        let path = &file.path;
        if file.theirs_gone() {
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
    let needed = generated::regenerators(&paths);
    let regenerators: Vec<Regenerator> = Regenerator::ALL
        .into_iter()
        .filter(|regenerator| needed.contains(regenerator) || rebuild.contains(regenerator))
        .collect();
    run_all(checkout, &regenerators).await?;
    done("stage", repo.stage_all().await)?;
    held.regenerated = paths;
    done(
        "commit",
        repo.commit(&held.message(branch, integration, head)).await,
    )?;
    pushed(repo.as_ref(), branch, Merge::Held(held)).await
}

/// What a merge of [`regenerate`] left: the merge and its unmerged paths.
enum Left {
    /// `merge`'s run, and the paths it left unmerged (none when it went
    /// through).
    Merged(Ran, Vec<Unmerged>),
    /// The head split its legacy handler files and the splitter refused
    /// the branch's tree: those legacy files, and why.
    Refused(Vec<String>, String),
}

/// Merges the head into the branch checked out — taking the head's handler
/// split first when the merge meets it, then merging the handler files
/// both sides added on the split merge base, as `held` records.
async fn merge_across_the_split(
    checkout: &Checkout,
    merging: Merging<'_>,
    held: &mut Held,
) -> Result<Left, String> {
    let Merging {
        branch,
        integration,
        head,
        ..
    } = merging;
    let repo = checkout.repo.as_ref();
    let merged = merge(repo, head).await?;
    if merged.ok() {
        return Ok(Left::Merged(merged, Vec::new()));
    }
    let conflicted = conflicts(repo, &merged).await?;
    if !handler_split::split_needed(&conflicted) {
        return Ok(Left::Merged(merged, conflicted));
    }
    let base = match handler_split::split_receiving(checkout, branch, integration, head).await? {
        handler_split::Receiving::Split { base } => base,
        handler_split::Receiving::Refused(why) => {
            let legacy = conflicted
                .into_iter()
                .filter(|file| mock_handlers::is_legacy(&file.path))
                .map(|file| file.path)
                .collect();
            return Ok(Left::Refused(legacy, why));
        }
    };
    held.split = true;
    let merged = merge(repo, head).await?;
    if merged.ok() {
        return Ok(Left::Merged(merged, Vec::new()));
    }
    let mut conflicted = conflicts(repo, &merged).await?;
    let adds = handler_split::split_add_adds(&conflicted);
    if !adds.is_empty() {
        match &base {
            Ok(split_base) => {
                held.three_way =
                    handler_split::merge_on_split_base(checkout, split_base, &adds).await?;
                conflicted.retain(|file| !held.three_way.contains(&file.path));
            }
            Err(why) => held.unsplit = Some(why.clone()),
        }
    }
    Ok(Left::Merged(merged, conflicted))
}

/// `merge <head>`, a refusal to run being why.
async fn merge(repo: &dyn Repo, head: &str) -> Result<Ran, String> {
    repo.merge(head)
        .await
        .map_err(|halt| halt.reason().to_string())
}

/// The paths a merge that did not go through left unmerged — none being a
/// failure of the merge itself.
async fn conflicts(repo: &dyn Repo, merged: &Ran) -> Result<Vec<Unmerged>, String> {
    let conflicted = unmerged(
        &repo
            .dirty_files()
            .await
            .map_err(|h| h.reason().to_string())?,
    );
    if conflicted.is_empty() {
        return Err(format!("the merge failed: {}", merged.why()));
    }
    Ok(conflicted)
}

/// The generators of `rebuild` that apply to the merged tree:
/// `mock/build.sh` only where the product builds bundles — its
/// `mock/pages.json` records a `builtFrom` digest.
fn applicable(checkout: &Checkout, rebuild: &[Regenerator]) -> Vec<Regenerator> {
    let builds_bundles = checkout
        .disk
        .read_to_string(&checkout.root.join("mock/pages.json"))
        .is_some_and(|pages| pages.contains("\"builtFrom\""));
    rebuild
        .iter()
        .copied()
        .filter(|regenerator| *regenerator != Regenerator::MockBuild || builds_bundles)
        .collect()
}

/// A merge git made alone, pushed: after `rebuild` ran on the merged tree
/// and what it rewrote was committed on top — nothing rewritten, git's
/// merge as it is.
async fn rebuilt(
    checkout: &Checkout,
    merging: Merging<'_>,
    rebuild: &[Regenerator],
    held: Held,
) -> Result<Merge, String> {
    let Merging {
        branch,
        integration,
        head,
        ..
    } = merging;
    let repo = checkout.repo.as_ref();
    if rebuild.is_empty() {
        return pushed(repo, branch, Merge::Held(held)).await;
    }
    run_all(checkout, rebuild).await?;
    done("stage", repo.stage_all().await)?;
    let rewritten: Vec<String> = repo
        .dirty_files()
        .await
        .map_err(|halt| halt.reason().to_string())?
        .iter()
        .filter_map(|line| line.get(3..))
        .map(|path| path.trim_matches('"').to_string())
        .collect();
    if rewritten.is_empty() {
        return pushed(repo, branch, Merge::Held(held)).await;
    }
    let commands: Vec<String> = rebuild.iter().map(|r| r.command()).collect();
    done(
        "commit",
        repo.commit(&format!(
            "chore: {} after merging {integration} ({head}) into {branch}, {} regenerated",
            commands.join(" then "),
            rewritten.join(", ")
        ))
        .await,
    )?;
    pushed(
        repo,
        branch,
        Merge::Held(Held {
            regenerated: rewritten,
            ..held
        }),
    )
    .await
}

/// Runs `regenerators` in the checkout, in the order given: the first that
/// fails stops the rest.
async fn run_all(checkout: &Checkout, regenerators: &[Regenerator]) -> Result<(), String> {
    for regenerator in regenerators {
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
                    timeout(*regenerator),
                )
                .await,
        )?;
    }
    Ok(())
}

/// Pushes the detached `HEAD` as `branch` — never forced: `held` once it
/// went, [`Merge::Raced`] when the remote refused it as a non-fast-forward
/// (`[rejected]`: a push of a session landed there since the fetch), why
/// on any other refusal.
async fn pushed(repo: &dyn Repo, branch: &str, held: Merge) -> Result<Merge, String> {
    match repo.push_head(branch).await {
        Ok(ran) if !ran.ok() && ran.stderr.contains("[rejected]") => Ok(Merge::Raced),
        ran => done("push", ran).map(|()| held),
    }
}

/// A step that ran and succeeded, or why it did not.
pub(crate) fn done(step: &str, ran: Outcome<Ran>) -> Result<(), String> {
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
                code: code.to_string(),
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
        rebuilding(repos, process, disk, &[]).await
    }

    /// [`locally_on`], these generators run whatever conflicts.
    async fn rebuilding(
        repos: &FakeRepos,
        process: &Rc<FakeProcess>,
        disk: &Rc<FakeDisk>,
        rebuild: &[Regenerator],
    ) -> (CaughtUp, Vec<String>) {
        catching(repos, process, disk, ("milestone/17-ai", "h1"), rebuild).await
    }

    /// Catches `branch` up with `main_agent`'s `head` in a checkout of
    /// `repos`, these generators run whatever conflicts.
    async fn catching(
        repos: &FakeRepos,
        process: &Rc<FakeProcess>,
        disk: &Rc<FakeDisk>,
        (branch, head): (&str, &str),
        rebuild: &[Regenerator],
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
        let caught = catch_up_locally(
            &checkout,
            branch,
            "main_agent",
            &BaseHead {
                sha: head.to_string(),
                rebuild: rebuild.to_vec(),
            },
            &log,
        )
        .await;
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
                "checkout origin/milestone/17-ai --force",
                "merge h1",
                "take_theirs Cargo.lock",
                "take_theirs crates/dataguard/registry/Cargo.toml",
                "stage_all",
                "commit chore: merge main_agent (h1) into milestone/17-ai, Cargo.lock, \
                 crates/dataguard/registry/Cargo.toml regenerated",
                "push HEAD:milestone/17-ai",
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
        assert!(tail[0] == "push HEAD:milestone/17-ai" && tail[1].starts_with("commit "));
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
        assert!(repos.asked("push HEAD:milestone/17-ai") && !repos.asked("commit"));
        assert_eq!(said, ["milestone/17-ai holds main_agent"]);
    }

    /// A merged `mock/pages.json` with a digest: a product that builds
    /// bundles.
    const PAGES_BUILT: &str = "{\n  \"builtFrom\": \"bb\"\n}\n";

    #[tokio::test]
    async fn a_clean_merge_whose_base_staled_the_bundles_rebuilds_them_and_commits_on_top() {
        let rewritten = ["mock/components/Dice.js", "mock/pages.json"];
        let repos = FakeRepos::default().rewriting(&rewritten);
        let process = Rc::new(FakeProcess::default());
        let (caught, said) = rebuilding(
            &repos,
            &process,
            &pages(PAGES_BUILT),
            &[Regenerator::MockBuild],
        )
        .await;
        assert_eq!(
            caught,
            CaughtUp::Locally(rewritten.iter().map(|p| (*p).to_string()).collect())
        );
        assert_eq!(programs(&process), ["sh mock/build.sh"]);
        assert_eq!(
            verbs(&repos),
            [
                "fetch",
                "checkout origin/milestone/17-ai --force",
                "merge h1",
                "stage_all",
                "commit chore: sh mock/build.sh after merging main_agent (h1) into \
                 milestone/17-ai, mock/components/Dice.js, mock/pages.json regenerated",
                "push HEAD:milestone/17-ai",
            ]
        );
        assert_eq!(
            said,
            [
                "milestone/17-ai holds main_agent: mock/components/Dice.js and mock/pages.json \
              regenerated, never merged by hand"
            ]
        );
    }

    #[tokio::test]
    async fn a_rebuild_that_rewrites_nothing_pushes_git_s_merge_as_it_is() {
        let repos = FakeRepos::default();
        let process = Rc::new(FakeProcess::default());
        let (caught, said) = rebuilding(
            &repos,
            &process,
            &pages(PAGES_BUILT),
            &[Regenerator::MockBuild],
        )
        .await;
        assert_eq!(caught, CaughtUp::Locally(Vec::new()));
        assert_eq!(programs(&process), ["sh mock/build.sh"]);
        assert!(repos.asked("stage_all") && !repos.asked("commit"));
        assert!(repos.asked("push HEAD:milestone/17-ai"));
        assert_eq!(said, ["milestone/17-ai holds main_agent"]);
    }

    #[tokio::test]
    async fn a_clean_merge_with_nothing_staled_builds_no_bundle_without_a_split() {
        let repos = FakeRepos::default().rewriting(&["mock/pages.json"]);
        let process = Rc::new(FakeProcess::default());
        let (caught, _) = rebuilding(&repos, &process, &pages(PAGES_BUILT), &[]).await;
        assert_eq!(caught, CaughtUp::Locally(Vec::new()));
        assert!(process.ran.borrow().is_empty(), "no mock/build.sh");
        assert!(!repos.asked("stage_all") && !repos.asked("commit"));
        assert!(repos.asked("push HEAD:milestone/17-ai"));
    }

    #[tokio::test]
    async fn a_product_that_builds_no_bundle_is_not_rebuilt() {
        for disk in [
            Rc::new(FakeDisk::default()),
            pages("{\n  \"pages\": []\n}\n"),
        ] {
            let repos = FakeRepos::default().rewriting(&["x"]);
            let process = Rc::new(FakeProcess::default());
            let (caught, _) = rebuilding(&repos, &process, &disk, &[Regenerator::MockBuild]).await;
            assert_eq!(caught, CaughtUp::Locally(Vec::new()));
            assert!(process.ran.borrow().is_empty(), "no mock/build.sh");
            assert!(!repos.asked("stage_all") && !repos.asked("commit"));
            assert!(repos.asked("push HEAD:milestone/17-ai"));
        }
    }

    #[tokio::test]
    async fn a_rebuild_joins_the_conflict_s_generators_in_their_order() {
        let repos = FakeRepos::conflicting(&["Cargo.lock"], &[]);
        let process = Rc::new(FakeProcess::default());
        let (caught, _) = rebuilding(
            &repos,
            &process,
            &pages(PAGES_BUILT),
            &[Regenerator::MockBuild],
        )
        .await;
        assert_eq!(caught, CaughtUp::Locally(vec!["Cargo.lock".to_string()]));
        assert_eq!(
            programs(&process),
            ["sh mock/build.sh", "cargo metadata --format-version 1"]
        );
    }

    #[tokio::test]
    async fn a_rebuild_that_fails_after_a_clean_merge_pushes_nothing() {
        let repos = FakeRepos::default().rewriting(&["mock/pages.json"]);
        let process = Rc::new(FakeProcess::answering(vec![FakeProcess::ran(
            "",
            "npx: command not found",
            127,
        )]));
        let (caught, _) = rebuilding(
            &repos,
            &process,
            &pages(PAGES_BUILT),
            &[Regenerator::MockBuild],
        )
        .await;
        assert!(
            matches!(&caught, CaughtUp::Failed(why) if why.starts_with("sh mock/build.sh failed")),
            "{caught:?}"
        );
        assert!(!repos.asked("push") && !repos.asked("commit"));
        assert!(!repos.asked("abort_merge"), "git's merge is committed");
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
        for step in ["fetch", "checkout origin/"] {
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
        assert!(repos.asked("commit") && repos.asked("push HEAD:milestone/17-ai"));
        let verbs = verbs(&repos);
        assert_eq!(
            verbs[verbs.len() - 3..],
            [
                "checkout milestone/17-ai --force",
                "reset_hard origin/milestone/17-ai",
                "clean"
            ],
            "the unpushed merge commit goes: no work kept in the checkout"
        );
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

    /// The head that split its legacy handler files (`main_agent` at
    /// `a55bd4d`, #530) and the milestone still on them (milestone/28 at
    /// `7e2a839`), as the 2026-10-24 scratch run met them.
    const SPLIT_HEAD: &str = "a55bd4d";
    const M28: &str = "milestone/28-closing-a-session";

    /// The first merge of `milestone/28` with the split head: the four
    /// generated files, and the legacy files the head deleted.
    const FIRST: [(&str, &str); 6] = [
        ("UU", "Cargo.lock"),
        ("UU", REGISTRY),
        ("UU", "mock/pages.json"),
        ("UU", "queries/registry.json"),
        ("UD", "mock/src/commands.js"),
        ("UD", "mock/src/reads.js"),
    ];

    /// The handler files the split wrote on both sides since the merge base.
    const ADD_ADDS: [&str; 3] = [
        "mock/handlers/_lib/commands.js",
        "mock/handlers/_lib/reads.js",
        "mock/handlers/campaign.saveEntity.js",
    ];

    /// The merge after the split: the same generated files, and the
    /// add/adds.
    fn second() -> Vec<(&'static str, &'static str)> {
        let mut second = FIRST[..4].to_vec();
        second.extend(ADD_ADDS.iter().map(|path| ("AA", *path)));
        second
    }

    /// The blobs the split reads: the head's splitter inputs, each add/add's
    /// two sides, and its text in the split merge base (`abc1234`, the
    /// fake's `HEAD` after that commit) unless `absent_at_base`.
    fn blobs(absent_at_base: &[&str]) -> Vec<(String, String)> {
        let mut blobs = vec![
            (
                format!("{SPLIT_HEAD}:scripts/split-handlers.mjs"),
                "// the splitter\n".to_string(),
            ),
            (
                format!("{SPLIT_HEAD}:mock/src/server.js"),
                "// the server\n".to_string(),
            ),
        ];
        for path in ADD_ADDS {
            blobs.push((format!(":2:{path}"), format!("ours of {path}\n")));
            blobs.push((format!(":3:{path}"), format!("theirs of {path}\n")));
            if !absent_at_base.contains(&path) {
                blobs.push((format!("abc1234:{path}"), format!("base of {path}\n")));
            }
        }
        blobs
    }

    /// milestone/28's checkout: its merges conflicting on `first` then
    /// `second`, the merge base `19262b9`, `blobs` shown.
    fn legacy_branch(
        first: &[(&str, &str)],
        second: &[(&str, &str)],
        blobs: &[(String, String)],
    ) -> FakeRepos {
        let shown: Vec<(&str, &str)> = blobs
            .iter()
            .map(|(spec, text)| (spec.as_str(), text.as_str()))
            .collect();
        FakeRepos::conflicting_then(first, second)
            .with_merge_base("19262b9")
            .showing(&shown)
    }

    /// Catches milestone/28 up with the split head.
    async fn across_the_split(
        repos: &FakeRepos,
        process: &Rc<FakeProcess>,
        disk: &Rc<FakeDisk>,
    ) -> (CaughtUp, Vec<String>) {
        catching(repos, process, disk, (M28, SPLIT_HEAD), &[]).await
    }

    fn at_root(path: &str) -> std::path::PathBuf {
        Path::new(ROOT).join(path)
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // one live case, verb by verb: cut, it reads as two
    async fn a_modify_delete_on_the_legacy_handlers_splits_the_branch_first_then_merges() {
        let repos = legacy_branch(&FIRST, &second(), &blobs(&[]));
        let process = Rc::new(FakeProcess::default());
        let disk = pages(PAGES_DIGESTS);
        let (caught, said) = across_the_split(&repos, &process, &disk).await;
        assert_eq!(
            caught,
            CaughtUp::Locally(
                FIRST[..4]
                    .iter()
                    .map(|(_, path)| (*path).to_string())
                    .collect()
            )
        );
        let scratch = |side: &str| format!("{ROOT}/.git/harness-merge/{side}");
        let mut expected: Vec<String> = [
            "fetch".to_string(),
            format!("checkout origin/{M28} --force"),
            format!("merge {SPLIT_HEAD}"),
            "abort_merge".to_string(),
            format!("merge_base HEAD {SPLIT_HEAD}"),
            "checkout 19262b9 --force".to_string(),
            format!("show {SPLIT_HEAD}:scripts/split-handlers.mjs"),
            format!("show {SPLIT_HEAD}:mock/src/server.js"),
            "stage_all".to_string(),
            "commit split of the merge base 19262b9".to_string(),
            "checkout abc1234 --force".to_string(),
            "clean".to_string(),
            format!("show {SPLIT_HEAD}:scripts/split-handlers.mjs"),
            format!("show {SPLIT_HEAD}:mock/src/server.js"),
            "stage_paths mock/handlers mock/src/commands.js mock/src/reads.js".to_string(),
            format!(
                "commit chore(mock): split the legacy handler files into mock/handlers/<id>.js \
                 before taking main_agent ({SPLIT_HEAD})"
            ),
            "reset_hard HEAD".to_string(),
            "clean".to_string(),
            format!("merge {SPLIT_HEAD}"),
        ]
        .to_vec();
        for path in ADD_ADDS {
            expected.extend([
                format!("show :2:{path}"),
                format!("show :3:{path}"),
                format!("show abc1234:{path}"),
                format!(
                    "merge_file {} {} {}",
                    scratch("ours"),
                    scratch("base"),
                    scratch("theirs")
                ),
            ]);
        }
        expected.extend([
            "take_theirs Cargo.lock".to_string(),
            format!("take_theirs {REGISTRY}"),
            "take_theirs queries/registry.json".to_string(),
            "stage_all".to_string(),
            format!(
                "commit chore: merge main_agent ({SPLIT_HEAD}) into {M28}, Cargo.lock, {REGISTRY}, \
                 mock/pages.json, queries/registry.json regenerated, \
                 mock/handlers/_lib/commands.js, mock/handlers/_lib/reads.js, \
                 mock/handlers/campaign.saveEntity.js merged three ways on the split merge base"
            ),
            format!("push HEAD:{M28}"),
        ]);
        assert_eq!(verbs(&repos), expected);
        assert_eq!(
            programs(&process),
            [
                "node scripts/split-handlers.mjs",
                "node scripts/split-handlers.mjs",
                "bash scripts/register.sh",
                "bash scripts/queries.sh",
                "sh mock/build.sh",
                "cargo metadata --format-version 1"
            ],
            "the merge base split, then the branch, both at the root; the bundles rebuilt"
        );
        // The head's inputs written for each run; the three sides of each
        // add/add under .git/; the merged text into the tree.
        let written = disk.written.borrow();
        assert_eq!(
            written
                .iter()
                .filter(|(path, _)| *path == at_root("mock/src/server.js"))
                .count(),
            2
        );
        assert_eq!(
            disk.written_to(&at_root(".git/harness-merge/base"))
                .as_deref(),
            Some("base of mock/handlers/campaign.saveEntity.js\n")
        );
        for path in ADD_ADDS {
            assert!(
                written.contains(&(
                    at_root(".git/harness-merge/ours"),
                    format!("ours of {path}\n")
                )),
                "{path}"
            );
            assert_eq!(
                disk.written_to(&at_root(path)).as_deref(),
                Some(format!("merged {}", scratch("ours")).as_str()),
                "{path}"
            );
        }
        drop(written);
        assert_eq!(
            said,
            [format!(
                "{M28} holds main_agent: its legacy handler files split first, \
                 mock/handlers/_lib/commands.js and mock/handlers/_lib/reads.js and \
                 mock/handlers/campaign.saveEntity.js merged three ways on the split merge base, \
                 Cargo.lock and {REGISTRY} and mock/pages.json and queries/registry.json \
                 regenerated, never merged by hand"
            )]
        );
    }

    #[tokio::test]
    async fn a_handler_absent_from_the_split_base_merges_on_an_empty_base() {
        let repos = legacy_branch(
            &FIRST,
            &[("AA", "mock/handlers/campaign.saveEntity.js")],
            &blobs(&["mock/handlers/campaign.saveEntity.js"]),
        );
        let disk = Rc::new(FakeDisk::default());
        let (caught, _) = across_the_split(&repos, &Rc::new(FakeProcess::default()), &disk).await;
        assert_eq!(caught, CaughtUp::Locally(Vec::new()));
        assert_eq!(
            disk.written_to(&at_root(".git/harness-merge/base"))
                .as_deref(),
            Some(""),
            "an empty base: both added it"
        );
        assert!(repos.asked("push"));
    }

    #[tokio::test]
    async fn a_handler_both_sides_changed_beyond_three_way_is_residual() {
        let repos = legacy_branch(&FIRST, &second(), &blobs(&[])).merging_files(vec![
            FakeProcess::ran("merged", "", 0),
            FakeProcess::ran("<<<<<<< ours", "", 1),
            FakeProcess::ran("merged", "", 0),
        ]);
        let disk = pages(PAGES_DIGESTS);
        let (caught, said) =
            across_the_split(&repos, &Rc::new(FakeProcess::default()), &disk).await;
        assert_eq!(
            caught,
            CaughtUp::Residual(vec!["mock/handlers/_lib/reads.js".to_string()])
        );
        assert!(said.is_empty(), "the caller says it: {said:?}");
        assert!(!repos.asked("push") && !repos.asked("take_theirs"));
        assert_eq!(
            disk.written_to(&at_root("mock/handlers/_lib/reads.js")),
            None,
            "a conflicted merge is never written"
        );
        let verbs = verbs(&repos);
        assert_eq!(
            verbs[verbs.len() - 4..],
            [
                "abort_merge".to_string(),
                format!("checkout {M28} --force"),
                format!("reset_hard origin/{M28}"),
                "clean".to_string()
            ],
            "the unpushed split commit goes with the merge: no work kept"
        );
    }

    #[tokio::test]
    async fn a_merge_file_that_errors_fails() {
        for code in [-1, 255, 128] {
            let repos = legacy_branch(&FIRST, &second(), &blobs(&[]))
                .merging_files(vec![FakeProcess::ran("", "error: could not open", code)]);
            let (caught, _) = across_the_split(
                &repos,
                &Rc::new(FakeProcess::default()),
                &pages(PAGES_DIGESTS),
            )
            .await;
            assert_eq!(
                caught,
                CaughtUp::Failed(
                    "git merge-file on mock/handlers/_lib/commands.js failed: error: could not open"
                        .to_string()
                ),
                "{code}"
            );
        }
        let repos = legacy_branch(&FIRST, &second(), &blobs(&[])).failing_too(&["merge_file"]);
        let (caught, _) = across_the_split(
            &repos,
            &Rc::new(FakeProcess::default()),
            &pages(PAGES_DIGESTS),
        )
        .await;
        assert!(
            matches!(&caught, CaughtUp::Failed(why) if why.starts_with("git merge-file on mock/handlers/_lib/commands.js failed: fatal: merge_file")),
            "{caught:?}"
        );
    }

    #[tokio::test]
    async fn a_splitter_that_refuses_leaves_the_legacy_files_residual_and_pushes_nothing() {
        let repos = legacy_branch(&FIRST, &second(), &blobs(&[]));
        let process = Rc::new(FakeProcess::answering(vec![
            FakeProcess::ran("split-handlers: split 3 read(s)", "", 0),
            FakeProcess::ran(
                "",
                "split-handlers: refused, nothing written — mock/src/commands.js:12: a key \
                 defined twice",
                1,
            ),
        ]));
        let (caught, said) = across_the_split(&repos, &process, &pages(PAGES_DIGESTS)).await;
        assert_eq!(
            caught,
            CaughtUp::Residual(vec![
                "mock/src/commands.js".to_string(),
                "mock/src/reads.js".to_string()
            ])
        );
        assert_eq!(
            said,
            [format!(
                "{M28} cannot take main_agent's handler split: the splitter refused its tree \
                 (split-handlers: refused, nothing written — mock/src/commands.js:12: a key \
                 defined twice)"
            )]
        );
        assert!(!repos.asked("stage_paths") && !repos.asked("push"));
        assert_eq!(
            verbs(&repos)
                .iter()
                .filter(|v| v.starts_with("merge "))
                .count(),
            1,
            "never merged again"
        );
        assert!(repos.asked(&format!("reset_hard origin/{M28}")));
    }

    #[tokio::test]
    async fn a_splitter_that_cannot_start_or_an_old_node_fails() {
        let repos = legacy_branch(&FIRST, &second(), &blobs(&[]));
        let process = Rc::new(FakeProcess {
            unlaunchable: vec!["node".to_string()],
            ..FakeProcess::default()
        });
        let (caught, _) = across_the_split(&repos, &process, &pages(PAGES_DIGESTS)).await;
        assert_eq!(
            caught,
            CaughtUp::Failed(
                "node scripts/split-handlers.mjs cannot start: node cannot be launched".to_string()
            )
        );
        assert!(!repos.asked("push"));
        let repos = legacy_branch(&FIRST, &second(), &blobs(&[]));
        let process = Rc::new(FakeProcess::answering(vec![FakeProcess::ran(
            "",
            "split-handlers: node 22 or later is needed, this is v18.19.0",
            1,
        )]));
        let (caught, _) = across_the_split(&repos, &process, &pages(PAGES_DIGESTS)).await;
        assert!(
            matches!(&caught, CaughtUp::Failed(why) if why.contains("node 22 or later is needed")),
            "the toolchain's fault, parked — never a session: {caught:?}"
        );
        let repos = legacy_branch(&FIRST, &second(), &blobs(&[]));
        let killed = Rc::new(FakeProcess::answering(vec![Ran {
            code: None,
            stdout: String::new(),
            stderr: "killed".to_string(),
        }]));
        assert!(matches!(
            across_the_split(&repos, &killed, &pages(PAGES_DIGESTS))
                .await
                .0,
            CaughtUp::Failed(_)
        ));
    }

    #[tokio::test]
    async fn a_base_splitter_that_refuses_leaves_the_add_adds_residual() {
        let repos = legacy_branch(&FIRST, &second(), &blobs(&[]));
        let process = Rc::new(FakeProcess::answering(vec![FakeProcess::ran(
            "",
            "split-handlers: refused, nothing written — the self-check could not run",
            1,
        )]));
        let (caught, said) = across_the_split(&repos, &process, &pages(PAGES_DIGESTS)).await;
        assert_eq!(
            caught,
            CaughtUp::Residual(ADD_ADDS.iter().map(|p| (*p).to_string()).collect())
        );
        assert_eq!(
            said,
            [format!(
                "{M28} cannot merge the handler files both sides added three ways (the merge \
                 base cannot be split: split-handlers: refused, nothing written — the \
                 self-check could not run): they stay a hand's"
            )],
            "why the add/adds are residual is said, not only their names"
        );
        assert!(
            !repos.asked("stage_all") || !repos.asked("commit split of the merge base"),
            "nothing committed for a base that did not split"
        );
        assert!(repos.asked("stage_paths"), "the branch split all the same");
        assert!(!repos.asked("merge_file"));
        // A head without the splitter: neither side splits.
        let repos = legacy_branch(&FIRST, &second(), &[]);
        let (caught, said) = across_the_split(
            &repos,
            &Rc::new(FakeProcess::default()),
            &pages(PAGES_DIGESTS),
        )
        .await;
        assert!(matches!(caught, CaughtUp::Residual(_)), "{caught:?}");
        assert!(
            said[0].ends_with(&format!("({SPLIT_HEAD} has no scripts/split-handlers.mjs)")),
            "{said:?}"
        );
        // No merge base: the add/adds stay a hand's.
        let repos = FakeRepos::conflicting_then(&FIRST, &second());
        let shown: Vec<(String, String)> = blobs(&[]);
        let repos = repos.showing(
            &shown
                .iter()
                .map(|(spec, text)| (spec.as_str(), text.as_str()))
                .collect::<Vec<_>>(),
        );
        let (caught, said) = across_the_split(
            &repos,
            &Rc::new(FakeProcess::default()),
            &pages(PAGES_DIGESTS),
        )
        .await;
        assert_eq!(
            caught,
            CaughtUp::Residual(ADD_ADDS.iter().map(|p| (*p).to_string()).collect())
        );
        assert!(
            said.len() == 1
                && said[0].contains(&format!("({M28} and {SPLIT_HEAD} have no merge base)")),
            "{said:?}"
        );
    }

    #[tokio::test]
    async fn a_split_whose_second_merge_is_clean_rebuilds_the_bundles() {
        let repos = legacy_branch(&FIRST[4..], &[], &blobs(&[])).rewriting(&["mock/pages.json"]);
        let process = Rc::new(FakeProcess::default());
        let (caught, said) = across_the_split(&repos, &process, &pages(PAGES_BUILT)).await;
        assert_eq!(
            caught,
            CaughtUp::Locally(vec!["mock/pages.json".to_string()])
        );
        assert_eq!(
            programs(&process),
            [
                "node scripts/split-handlers.mjs",
                "node scripts/split-handlers.mjs",
                "sh mock/build.sh"
            ]
        );
        assert_eq!(
            said,
            [format!(
                "{M28} holds main_agent: its legacy handler files split first, mock/pages.json \
                 regenerated, never merged by hand"
            )]
        );
        assert!(repos.asked(&format!("push HEAD:{M28}")));
        // A product that builds no bundle pushes both commits as they are.
        let repos = legacy_branch(&FIRST[4..], &[], &blobs(&[]));
        let process = Rc::new(FakeProcess::default());
        let (caught, said) =
            across_the_split(&repos, &process, &Rc::new(FakeDisk::default())).await;
        assert_eq!(caught, CaughtUp::Locally(Vec::new()));
        assert_eq!(programs(&process).len(), 2, "the two splits only");
        assert_eq!(
            said,
            [format!(
                "{M28} holds main_agent: its legacy handler files split first, never merged by \
                 hand"
            )]
        );
    }

    #[tokio::test]
    async fn a_modify_delete_elsewhere_or_deleted_by_us_is_no_split() {
        for (code, path) in [
            ("UD", "crates/x/src/lib.rs"),
            ("DU", "mock/src/commands.js"),
        ] {
            let repos = legacy_branch(&[(code, path)], &[], &blobs(&[]));
            let (caught, _) = across_the_split(
                &repos,
                &Rc::new(FakeProcess::default()),
                &pages(PAGES_DIGESTS),
            )
            .await;
            assert_eq!(caught, CaughtUp::Residual(vec![path.to_string()]), "{code}");
            assert!(!repos.asked("merge_base"), "{code} {path}: no split");
        }
    }

    #[tokio::test]
    async fn a_failing_step_of_the_split_fails_and_leaves_the_branch_as_the_remote_has_it() {
        for (step, why) in [
            ("abort_merge", "aborting the merge failed"),
            ("checkout 19262b9", "checking the merge base out failed"),
            ("stage_all", "staging the split merge base failed"),
            ("commit split", "committing the split merge base failed"),
            ("clean", "clean failed"),
            ("stage_paths", "staging the split failed"),
            ("commit chore(mock)", "committing the split failed"),
            (
                "reset_hard HEAD",
                "putting the splitter's inputs back failed",
            ),
        ] {
            let repos = legacy_branch(&FIRST, &second(), &blobs(&[])).failing_too(&[step]);
            let (caught, _) = across_the_split(
                &repos,
                &Rc::new(FakeProcess::default()),
                &pages(PAGES_DIGESTS),
            )
            .await;
            assert!(
                matches!(&caught, CaughtUp::Failed(said) if said.starts_with(why)),
                "{step}: {caught:?}"
            );
            assert!(!repos.asked("push"), "{step}");
            assert!(repos.asked(&format!("reset_hard origin/{M28}")), "{step}");
        }
        // A tree the head's inputs cannot be written into.
        let repos = legacy_branch(&FIRST, &second(), &blobs(&[]));
        let disk = Rc::new(FakeDisk {
            refuses_writes: true,
            ..FakeDisk::default()
        });
        let (caught, _) = across_the_split(&repos, &Rc::new(FakeProcess::default()), &disk).await;
        assert!(
            matches!(&caught, CaughtUp::Failed(why) if why.starts_with("scripts/split-handlers.mjs cannot be written")),
            "{caught:?}"
        );
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
            .map(|file| (file.path.as_str(), file.theirs_gone()))
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

    proptest::proptest! {
        #[test]
        fn any_status_line_reads_without_a_panic(line in "\\PC{0,12}") {
            let _ = unmerged(&[line]);
        }

        #[test]
        fn an_unmerged_code_tells_its_sides_and_any_other_reads_as_nothing(
            code in "[ ADMRCU?!]{2}",
            path in "[a-z/._é]{1,20}",
        ) {
            let read = unmerged(&[format!("{code} {path}")]);
            let conflict = ["DD", "AU", "UD", "UA", "DU", "AA", "UU"].contains(&code.as_str());
            proptest::prop_assert_eq!(read.len(), usize::from(conflict));
            if let Some(file) = read.first() {
                proptest::prop_assert_eq!(&file.code, &code);
                proptest::prop_assert_eq!(&file.path, &path);
                proptest::prop_assert_eq!(file.both_added(), code == "AA");
                proptest::prop_assert_eq!(
                    file.theirs_gone(),
                    ["UD", "DD", "AU"].contains(&code.as_str())
                );
            }
        }
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
