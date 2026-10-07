//! The gates that no workflow has: the tools and the branch.
//!
//! They are here and not with a workflow because none is specific to one: `claude`
//! on the `PATH` and at the right version, `gh` authenticated, an integration branch
//! that exists and triggers CI, a clean tree. A workflow will compose the list that
//! concerns it; what is specific to it stays with it.
//!
//! Generic over the workflow state (`S`): none reads it.
//!
//! # The version gate, and why it is strict
//!
//! `claude --version` must return **at least [`MINIMUM`]**. The reason is in
//! `docs/SESSION-CARRIER.md`: since v2.1.277, `total_cost_usd` on a `--resume`
//! call is cumulative for the entire conversation, and only covered the call
//! before. The ledger reads the last turn of a stage; under an older version
//! this value would be the cost of the last turn alone, and the ledger would
//! undercount without showing it.
//!
//! A gate costs a local call. A false `costs.tsv` is not visible — that is
//! the entire trade-off, and that is why it refuses rather than warns.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::process;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repo;
use harness_core::ports::shell::github::GitHub;

/// The version of `claude` below which the ledger would lie.
pub const MINIMUM: Version = Version(2, 1, 277);

/// A version, in three numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The version that this text announces, or `None`.
///
/// `claude --version` returns `2.1.285 (Claude Code)`: we read the first word
/// and ignore the rest — what follows has already changed form once.
///
/// `None` rather than an optimistic zero: a version we cannot read is not an
/// old version, and the two do not merit the same message.
#[must_use]
pub fn parse_version(text: &str) -> Option<Version> {
    let first = text.split_whitespace().next()?;
    let mut parts = first.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    // A prerelease suffix (`277-beta.1`) must not make the version unreadable:
    // we read the leading digits.
    let patch = parts.next().unwrap_or("0");
    let digits: String = patch.chars().take_while(char::is_ascii_digit).collect();
    Some(Version(major, minor, digits.parse().ok()?))
}

/// `claude` is on the `PATH` and recent enough that the ledger is correct.
pub struct ClaudeIsRecentEnough {
    /// The minimum version required.
    pub minimum: Version,
}

impl Default for ClaudeIsRecentEnough {
    fn default() -> Self {
        Self { minimum: MINIMUM }
    }
}

#[async_trait(?Send)]
impl<S> Verification<S> for ClaudeIsRecentEnough {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        let said = process::run(
            "claude",
            &["--version".to_string()],
            std::path::Path::new("."),
        )
        .await
        .map_err(|_| Halt::Halted("claude CLI not on PATH".to_string()))?;
        if !said.ok() {
            return Err(Halt::Halted(format!(
                "`claude --version` failed: {}",
                said.why()
            )));
        }
        let Some(found) = parse_version(said.out()) else {
            return Err(Halt::Halted(format!(
                "cannot read the claude version from {:?} — the harness needs \
                 to know it is at least {}, because `total_cost_usd` changed \
                 meaning there (see docs/SESSION-CARRIER.md)",
                said.out(),
                self.minimum
            )));
        };
        if found < self.minimum {
            return Err(Halt::Halted(format!(
                "claude {found} is older than {} — on a resumed session \
                 `total_cost_usd` only covered the last call before that \
                 version, so costs.tsv would undercount every stage without \
                 showing it. Run: claude update (and check that `command -v \
                 claude` resolves to what you just updated)",
                self.minimum
            )));
        }
        ctx.traces.debug(&format!("claude {found}"));
        Ok(Verdict::Continue)
    }
}

/// `gh` responds and is authenticated.
pub struct GhIsAuthenticated {
    /// The GitHub port.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl<S> Verification<S> for GhIsAuthenticated {
    async fn verify(&self, _ctx: &Context<S>) -> Outcome<Verdict> {
        if self.gh.authenticated().await? {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(
            "gh is not authenticated — run: gh auth login".to_string(),
        ))
    }
}

/// The integration branch exists, is the one in the checkout, and is on
/// `origin`.
///
/// All three in one gate because they depend on each other: asking if `origin`
/// has it does not make sense until it exists locally, and each failure names
/// its own command.
pub struct TheIntegrationBranch {
    /// The repository where the question is asked.
    pub git: Rc<dyn Repo>,
    /// The expected branch.
    pub branch: String,
    /// False when the run works in a clone: the current branch is there set by
    /// the mount, not by a human.
    pub check_current: bool,
    /// False for a dry run, which checks nothing out.
    ///
    /// The local branch and the current branch are then questions about a
    /// gesture the dry run deliberately skipped — `provisioning::mount` returns
    /// before any checkout. What is left to ask is whether `origin` carries the
    /// branch, which is exactly what a real run's mount would create the local
    /// one from. Demanding the local branch anyway made `--dry-run` refuse every
    /// workspace no run had provisioned yet, naming a command the human would
    /// have run inside a clone they did not know about.
    pub checked_out: bool,
}

#[async_trait(?Send)]
impl<S> Verification<S> for TheIntegrationBranch {
    async fn verify(&self, _ctx: &Context<S>) -> Outcome<Verdict> {
        let branch = &self.branch;
        if self.checked_out {
            if !self.git.has_branch(branch).await? {
                return Err(Halt::Halted(format!(
                    "branch {branch} does not exist — run: git branch {branch} \
                     origin/{branch}"
                )));
            }
            if self.check_current {
                let current = self.git.current_branch().await?;
                if current != *branch {
                    return Err(Halt::Halted(format!(
                        "on branch {current} — run: git checkout {branch}"
                    )));
                }
            }
        }
        if !self.git.origin_has_branch(branch).await? {
            return Err(Halt::Halted(format!(
                "origin has no {branch} — run: git push -u origin {branch}"
            )));
        }
        Ok(Verdict::Continue)
    }
}

/// CI triggers on the integration branch.
///
/// Without this, the PRs that stages open carry no `ci` check, and the `gh pr
/// checks` that skills wait for never resolves — the stage runs through its
/// budget and dies without delivering anything.
pub struct CiTriggersOnTheBranch {
    /// How to read the file.
    pub disk: Rc<dyn Disk>,
    /// The CI workflow of the target repository.
    pub path: PathBuf,
    /// The expected branch.
    pub branch: String,
}

#[async_trait(?Send)]
impl<S> Verification<S> for CiTriggersOnTheBranch {
    async fn verify(&self, _ctx: &Context<S>) -> Outcome<Verdict> {
        // No workflow yet: the milestone that creates the CI cannot depend on
        // it already existing. Warn and let the run start.
        if !self.disk.exists(&self.path) {
            eprintln!(
                "warning: {} does not exist yet — PRs carry no `ci` check until it does",
                self.path.display()
            );
            return Ok(Verdict::Continue);
        }
        let text = std::fs::read_to_string(&self.path)
            .map_err(|e| Halt::Unreadable(format!("{} : {e}", self.path.display())))?;
        if triggers_on(&text, &self.branch) {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            ".github/workflows/ci.yml does not trigger on {} — the PRs the \
             stages open would carry no `ci` check, and the `gh pr checks` the \
             skills wait on would never resolve",
            self.branch
        )))
    }
}

/// Whether the workflow text names `branch`, literally or through a glob such
/// as `milestone/**`.
///
/// A glob counts when the text before its first `*` is a prefix of the branch.
fn triggers_on(text: &str, branch: &str) -> bool {
    if text.contains(branch) {
        return true;
    }
    // Entries are read as tokens, so a flow list (`[main, "milestone/**"]`)
    // and a block list (`- milestone/**`) both work.
    text.split(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | ',' | '"' | '\'' | '-'))
        .filter_map(|entry| entry.split_once('*'))
        .any(|(prefix, _)| !prefix.is_empty() && branch.starts_with(prefix))
}

/// The working tree is clean, unless the run asked otherwise.
pub struct WorkingTreeIsClean {
    /// The repository where the question is asked.
    pub git: Rc<dyn Repo>,
    /// `--allow-dirty`.
    pub allowed: bool,
}

#[async_trait(?Send)]
impl<S> Verification<S> for WorkingTreeIsClean {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        if self.allowed {
            return Ok(Verdict::Continue);
        }
        let dirty = self.git.dirty_files().await?;
        if dirty.is_empty() {
            return Ok(Verdict::Continue);
        }
        for line in &dirty {
            ctx.traces.warn(line);
        }
        Err(Halt::Halted(
            "working tree is dirty — commit, stash, or re-run with \
             --allow-dirty"
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::ports::shell::process::Ran;

    #[test]
    fn a_milestone_glob_covers_a_milestone_branch() {
        let yml = "on:\n  push:\n    branches: [main_agent]\n  pull_request:\n    branches:\n      - 'milestone/**'\n";
        assert!(triggers_on(yml, "milestone/14-ci-verte"));
        assert!(!triggers_on(yml, "other/14"));
    }

    #[test]
    fn a_glob_inside_a_flow_list_counts() {
        let yml = "on:\n  pull_request:\n    branches: [main_agent, \"milestone/**\"]\n";
        assert!(triggers_on(yml, "milestone/15-x"));
    }

    #[test]
    fn a_literal_branch_name_still_counts() {
        assert!(triggers_on("branches: [main_agent]", "main_agent"));
    }

    #[test]
    fn the_version_is_read_off_the_first_word_and_the_rest_is_ignored() {
        // What follows the number has already changed form once.
        assert_eq!(
            parse_version("2.1.285 (Claude Code)"),
            Some(Version(2, 1, 285))
        );
        assert_eq!(parse_version("2.1.257"), Some(Version(2, 1, 257)));
    }

    #[test]
    fn a_prerelease_suffix_does_not_make_a_version_unreadable() {
        assert_eq!(parse_version("2.1.277-beta.1"), Some(Version(2, 1, 277)));
    }

    #[test]
    fn a_version_that_cannot_be_read_is_none_not_an_optimistic_zero() {
        // An unreadable version and an old version do not merit the same
        // message.
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("unknown"), None);
        assert_eq!(parse_version("2"), None);
    }

    #[test]
    fn the_ordering_is_by_number_not_by_text() {
        // `"2.1.9" > "2.1.277"` in string comparison, and that is exactly the
        // error three `u32`s make impossible.
        assert!(Version(2, 1, 9) < Version(2, 1, 277));
        assert!(Version(2, 2, 0) > MINIMUM);
        assert!(Version(2, 1, 257) < MINIMUM);
        assert!(MINIMUM >= MINIMUM);
    }

    #[test]
    fn the_minimum_is_the_version_where_total_cost_usd_changed_meaning() {
        assert_eq!(MINIMUM, Version(2, 1, 277));
        assert_eq!(MINIMUM.to_string(), "2.1.277");
    }

    // --- the integration branch -------------------------------------------

    /// A command that did nothing, successfully.
    fn nothing() -> Ran {
        Ran {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    /// A repository that answers only the three questions this gate asks.
    struct Checkout {
        local: bool,
        on: &'static str,
        remote: bool,
    }

    #[async_trait(?Send)]
    impl Repo for Checkout {
        async fn current_branch(&self) -> Outcome<String> {
            Ok(self.on.to_string())
        }
        async fn has_branch(&self, _name: &str) -> Outcome<bool> {
            Ok(self.local)
        }
        async fn origin_has_branch(&self, _name: &str) -> Outcome<bool> {
            Ok(self.remote)
        }
        async fn head_sha(&self) -> Outcome<String> {
            Ok(String::new())
        }
        async fn dirty_files(&self) -> Outcome<Vec<String>> {
            Ok(Vec::new())
        }
        async fn remote_url(&self, _remote: &str) -> Outcome<String> {
            Ok(String::new())
        }
        async fn tracked_files(&self) -> Outcome<Vec<String>> {
            Ok(Vec::new())
        }
        async fn default_branch(&self) -> Outcome<String> {
            Ok(String::new())
        }
        async fn local_branches(&self) -> Outcome<Vec<String>> {
            Ok(Vec::new())
        }
        async fn stashes(&self) -> Outcome<Vec<String>> {
            Ok(Vec::new())
        }
        async fn unpushed(&self) -> Outcome<Vec<String>> {
            Ok(Vec::new())
        }
        async fn branches_at_risk(&self, _upstream: &str) -> Outcome<Vec<String>> {
            Ok(Vec::new())
        }
        async fn create_local_branch(&self, _name: &str, _at: &str) -> Outcome<Ran> {
            Ok(nothing())
        }
        async fn stage_all(&self) -> Outcome<Ran> {
            Ok(nothing())
        }
        async fn commit(&self, _message: &str) -> Outcome<Ran> {
            Ok(nothing())
        }
        async fn push(&self, _branch: &str) -> Outcome<Ran> {
            Ok(nothing())
        }
        async fn clone_repo(&self, _url: &str, _name: &str) -> Outcome<Ran> {
            Ok(nothing())
        }
        async fn fetch(&self) -> Outcome<Ran> {
            Ok(nothing())
        }
        async fn checkout(&self, _branch: &str, _reset: bool) -> Outcome<Ran> {
            Ok(nothing())
        }
        async fn reset_hard(&self, _to: &str) -> Outcome<Ran> {
            Ok(nothing())
        }
        async fn clean(&self) -> Outcome<Ran> {
            Ok(nothing())
        }
        async fn delete_branch(&self, _name: &str) -> Outcome<Ran> {
            Ok(nothing())
        }
    }

    async fn judge(checkout: Checkout, check_current: bool, checked_out: bool) -> Outcome<Verdict> {
        let gate = TheIntegrationBranch {
            git: Rc::new(checkout),
            branch: "main_agent".to_string(),
            check_current,
            checked_out,
        };
        let ctx: Context<()> = Context::new(
            harness_core::execution::Settings {
                dry_run: false,
                stages: String::new(),
            },
            (),
            harness_core::traces::Logbook::null(),
        );
        gate.verify(&ctx).await
    }

    #[tokio::test]
    async fn a_dry_run_is_not_asked_for_a_checkout_it_deliberately_skipped() {
        // The defect: `--dry-run` mounts nothing, so no local `main_agent`
        // existed, and the gate refused every workspace no run had provisioned —
        // naming a command the human would have had to run inside a clone they
        // did not know about.
        let said = judge(
            Checkout {
                local: false,
                on: "feat/something-else",
                remote: true,
            },
            true,
            false,
        )
        .await
        .expect("origin has it, which is the whole question");
        assert!(matches!(said, Verdict::Continue));
    }

    #[tokio::test]
    async fn a_real_run_still_demands_the_local_branch_and_the_right_one() {
        let missing = judge(
            Checkout {
                local: false,
                on: "main_agent",
                remote: true,
            },
            true,
            true,
        )
        .await
        .expect_err("must stop");
        // The command must name the branch to branch *from*, not `origin/main`:
        // creating `main_agent` at `main`'s tip is not the integration branch.
        assert!(
            missing
                .reason()
                .contains("git branch main_agent origin/main_agent"),
            "{}",
            missing.reason()
        );
        let elsewhere = judge(
            Checkout {
                local: true,
                on: "feat/x",
                remote: true,
            },
            true,
            true,
        )
        .await
        .expect_err("must stop");
        assert!(elsewhere.reason().contains("git checkout main_agent"));
    }

    #[tokio::test]
    async fn origin_is_asked_whether_or_not_anything_was_checked_out() {
        // The one question a dry run keeps: it is what a real run's mount would
        // create the local branch from.
        for checked_out in [true, false] {
            let err = judge(
                Checkout {
                    local: true,
                    on: "main_agent",
                    remote: false,
                },
                true,
                checked_out,
            )
            .await
            .expect_err("must stop");
            assert!(err.reason().contains("origin has no main_agent"));
        }
    }
}
