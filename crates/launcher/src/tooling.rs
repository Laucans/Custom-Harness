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
use harness_core::adapters::shell::disk::Disk;
use harness_core::adapters::shell::git::Repo;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::shell::process;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

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
}

#[async_trait(?Send)]
impl<S> Verification<S> for TheIntegrationBranch {
    async fn verify(&self, _ctx: &Context<S>) -> Outcome<Verdict> {
        let branch = &self.branch;
        if !self.git.has_branch(branch).await? {
            return Err(Halt::Halted(format!(
                "branch {branch} does not exist — run: git branch {branch} \
                 origin/main"
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
        let missing = Halt::Halted(format!(
            ".github/workflows/ci.yml does not trigger on {} — the PRs the \
             stages open would carry no `ci` check, and the `gh pr checks` the \
             skills wait on would never resolve",
            self.branch
        ));
        if !self.disk.exists(&self.path) {
            return Err(missing);
        }
        let text = std::fs::read_to_string(&self.path)
            .map_err(|e| Halt::Unreadable(format!("{} : {e}", self.path.display())))?;
        if text.contains(&self.branch) {
            return Ok(Verdict::Continue);
        }
        Err(missing)
    }
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
}
