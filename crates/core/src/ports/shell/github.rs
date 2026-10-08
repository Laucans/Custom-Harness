//! What the harness asks GitHub — the port, not the `gh` binary.
//!
//! Nothing here decides. Reading an issue, its comments, its blockers, adding
//! a label: what a label **means** is the definition of a workflow and lives
//! with it.
//!
//! # The invariant that costs money
//!
//! **A read that does not succeed never returns an empty list.** `[]` reads as
//! "this milestone has no open tasks left", which is exactly the input that
//! triggers a `/planner`: an expired token would cost an opus run. Every failed
//! read thus returns [`Halt::Unreadable`](crate::domain::Halt::Unreadable),
//! which says what we do not know and names the gesture that unblocks.
//!
//! The implementation — `gh api` for everything, so a reader never has to
//! guess which half goes through the raw API — lives in
//! [`adapters::shell::github`](crate::adapters::shell::github).

use std::path::Path;

use async_trait::async_trait;

use crate::domain::{Issue, Outcome, Pr};

/// What the harness asks `gh` for, and nothing more.
#[async_trait(?Send)]
pub trait GitHub {
    /// Is `gh` authenticated?
    ///
    /// # Errors
    /// If `gh` could not be launched.
    async fn authenticated(&self) -> Outcome<bool>;

    /// `owner/name`, requested once then cached.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if `gh` cannot tell which repo this is.
    async fn repo(&self) -> Outcome<String>;

    /// The labels the repo bears, by name.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read does not succeed.
    async fn labels(&self) -> Outcome<Vec<String>>;

    /// This issue.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read does not succeed.
    async fn issue(&self, number: u64) -> Outcome<Issue>;

    /// Issues bearing this label, PRs excluded.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read does not succeed.
    async fn issues_labelled(&self, label: &str, state: &str) -> Outcome<Vec<Issue>>;

    /// The sub-issues of this one.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read does not succeed.
    async fn sub_issues(&self, number: u64) -> Outcome<Vec<Issue>>;

    /// What blocks this issue, with the state of each blocker.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read does not succeed.
    async fn blocked_by(&self, number: u64) -> Outcome<Vec<Issue>>;

    /// The same tasks, each bearing its blockers.
    ///
    /// One request per task: the entry point that lists sub-issues says nothing
    /// of dependencies, and deciding without them would read as "nothing
    /// blocks".
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if a read does not succeed.
    async fn with_blockers(&self, tasks: Vec<Issue>) -> Outcome<Vec<Issue>>;

    /// Merged PRs on `base`, most recently touched first.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read does not succeed — "API is down" and
    /// "nothing was delivered" lead to opposite decisions.
    async fn merged_prs(&self, base: &str) -> Outcome<Vec<Issue>>;

    /// The body of each comment, oldest to newest.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read does not succeed, or if there are more
    /// comments than pagination covers.
    async fn issue_comments(&self, number: u64) -> Outcome<Vec<String>>;

    /// Add a label.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn add_label(&self, number: u64, label: &str) -> Outcome<()>;

    /// Remove a label.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn remove_label(&self, number: u64, label: &str) -> Outcome<()>;

    /// Rewrite the body of an issue — for a task, its SPEC.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn set_body(&self, number: u64, body: &str) -> Outcome<()>;

    /// Post a comment on an issue.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn post_issue_comment(&self, number: u64, body: &str) -> Outcome<()>;

    /// Close an issue.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn close_issue(&self, number: u64) -> Outcome<()>;

    // --- what the PR review demands, and nothing else ----------------------
    //
    // Via `gh pr view`/`gh pr comment`, not via `gh api`: a review does not need
    // sub-issues or dependencies, and these two subcommands already return the
    // right form. Their failures are [`Halt::Failed`](crate::domain::Halt::Failed), not [`Halt::Unreadable`](crate::domain::Halt::Unreadable):
    // there is no empty list here that could read as "nothing left to do" — a
    // PR we cannot read is an ordinary failure, not an ambiguity.

    /// A PR's metadata, by its number or URL.
    ///
    /// # Errors
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if `gh` cannot read it.
    async fn pr(&self, pr_ref: &str) -> Outcome<Pr>;

    /// The body of all comments on a PR, **concatenated as-is**.
    ///
    /// A string, not a list, by design: the only thing done with it is a
    /// substring search (the marker of a review already posted), and that is
    /// exactly what `gh pr view --json comments -q .comments[].body`
    /// returns.
    ///
    /// # Errors
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if `gh` cannot read them.
    async fn pr_comments(&self, num: &str) -> Outcome<String>;

    /// Post a comment on a PR, from a file.
    ///
    /// A file, not a string: the text is already kept on disk before this call,
    /// so it survives if `gh` fails.
    ///
    /// # Errors
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if GitHub refuses.
    async fn post_pr_comment(&self, num: &str, body_file: &Path) -> Outcome<()>;

    // --- what `init-repo` needs, and nothing else ---------------------------
    //
    // Two of these (`branch_sha`, `file_text`) answer `None` on a 404, and only
    // on a 404: a 404 and a failed call must not collapse into the same value,
    // or an expired token would report "ci.yml is missing" and send a human
    // editing a file that is already correct.

    /// Create a label.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn create_label(&self, name: &str, color: &str, description: &str) -> Outcome<()>;

    /// The sha a branch points at, or `None` **only** on a 404.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read fails for any reason other than
    /// "absent".
    async fn branch_sha(&self, branch: &str) -> Outcome<Option<String>>;

    /// Create `refs/heads/{branch}` at this sha.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn create_branch(&self, branch: &str, sha: &str) -> Outcome<()>;

    /// The repo's default branch.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read does not succeed.
    async fn default_branch(&self) -> Outcome<String>;

    /// Make `branch` the repo's default branch.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn set_default_branch(&self, branch: &str) -> Outcome<()>;

    /// Protect `branch`: changes only through a pull request, no force push,
    /// no deletion. The required review count is `0`, so a solo maintainer
    /// can still merge.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses — notably on a private repo of a
    /// free plan, where branch protection is not offered.
    async fn protect_branch(&self, branch: &str) -> Outcome<()>;

    /// Whether the authenticated token can push to this repo.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read does not succeed.
    async fn can_push(&self) -> Outcome<bool>;

    /// The raw text of a file at `git_ref`, or `None` **only** on a 404.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read fails for any reason other than
    /// "absent".
    async fn file_text(&self, path: &str, git_ref: &str) -> Outcome<Option<String>>;

    // --- what the three-tier issue model demands, and nothing else ---------
    //
    // These are writes, so they share `issue`'s writes' failure shape
    // (`Halt::Halted`, "nothing is retried and nothing is undone") rather
    // than the PR-review group's `Halt::Failed` — a refused create/link here
    // is the same kind of event as a refused label or body write.

    /// Create an issue, returning its number.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses, or if it accepts the write but the
    /// response names no number to read back.
    async fn create_issue(&self, title: &str, body: &str, labels: &[&str]) -> Outcome<u64>;

    /// Link `child` as a sub-issue of `parent`.
    ///
    /// Takes issue **numbers** — the node-id GitHub's sub-issues API actually
    /// wants is resolved internally, so no caller has to know that quirk
    /// exists.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn create_sub_issue_link(&self, parent: u64, child: u64) -> Outcome<()>;

    /// Mark `number` as blocked by `blocker`.
    ///
    /// Takes issue **numbers**, same node-id resolution as
    /// [`Self::create_sub_issue_link`].
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn add_blocked_by(&self, number: u64, blocker: u64) -> Outcome<()>;

    /// Open a pull request from `head` into `base`, returning its URL.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses, or if it accepts the write but
    /// prints nothing to read the URL back from.
    async fn create_pr(&self, head: &str, base: &str, title: &str, body: &str) -> Outcome<String>;

    /// Merge a pull request by rebase — the same mechanism `/code` uses by
    /// hand today.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn merge_pr(&self, pr_ref: &str) -> Outcome<()>;

    /// Whether every check on this PR's latest commit succeeded.
    ///
    /// A PR with no checks configured at all reads as **not** green — the
    /// main-agent-merge gate that consumes this wants proof CI ran, not the
    /// absence of a reason to refuse.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if the status cannot be read at all (GitHub's own
    /// non-zero exit for "a check is pending or failing" is a normal
    /// answer, read from the JSON — not this error).
    async fn pr_checks_green(&self, pr_ref: &str) -> Outcome<bool>;

    /// The **open** PRs carrying this label, in the order GitHub returns.
    ///
    /// Distinct from [`GitHub::issues_labelled`] on purpose: that endpoint
    /// answers with issues, and a label on a PR is what a routing decision
    /// about a PR needs to read — head, base and draft included.
    ///
    /// # Errors
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if the list cannot be read.
    async fn open_prs_labelled(&self, label: &str) -> Outcome<Vec<Pr>>;

    /// The checks on this PR that have **concluded in failure**, one
    /// readable line each — empty when none has.
    ///
    /// Not the complement of [`GitHub::pr_checks_green`]: a pending check is
    /// neither green nor failing, and the difference is the whole point here
    /// — a repair is worth paying for once something has actually broken,
    /// never while CI is still running.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if the status cannot be read at all — GitHub's own
    /// non-zero exit for "a check is pending or failing" is a normal answer,
    /// read from the JSON.
    async fn pr_failing_checks(&self, pr_ref: &str) -> Outcome<Vec<String>>;
}
