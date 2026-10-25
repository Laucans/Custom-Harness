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
//! `harness_adapters::shell::github`.

use std::path::Path;

use async_trait::async_trait;

use crate::domain::{Issue, Outcome, Pr};

/// The longest issue body GitHub takes, in characters: a longer one is
/// refused whole (`422 Validation Failed`), so a body that may grow is fit
/// under it before it is sent.
pub const ISSUE_BODY_MAX: usize = 65_536;

/// A file a CI run kept in one of its artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    /// The file, as the run wrote it.
    pub text: String,
    /// The run that kept it, as a human opens it.
    pub run_url: String,
    /// When the run last moved, as GitHub says it.
    pub at: String,
}

/// What a conditional read of the board's activity said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activity {
    /// Nothing moved since the tag handed in — a read that cost no quota.
    Unchanged,
    /// Something moved. `etag` is what to hand in next time, when the server
    /// gave one; `moved_at` is when the newest issue or pull request last
    /// changed, when there is one.
    Changed {
        /// The server's tag for this state of the board.
        etag: Option<String>,
        /// The `updated_at` of the newest issue or pull request.
        moved_at: Option<String>,
    },
}

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

    /// Whether any issue or pull request moved since `etag` — the one read
    /// a poll pays while the board is quiet.
    ///
    /// Conditional on purpose: a `304 Not Modified` is free of quota, so a
    /// watch that polls every thirty seconds costs nothing until something
    /// changes. A CI run finishing moves no issue, which is why the caller
    /// keeps reading in full while a pull request waits on its checks.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read does
    /// not succeed — never `Unchanged`: an expired token must stop the loop,
    /// not quiet it.
    async fn activity(&self, etag: Option<&str>) -> Outcome<Activity>;

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

    /// A PR's metadata, by its number, its URL or its head branch — the
    /// newest PR of that branch, open or not: the caller reads its state.
    ///
    /// # Errors
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if `gh` cannot read it —
    /// a branch with no PR among the reasons.
    async fn pr(&self, pr_ref: &str) -> Outcome<Pr>;

    /// The paths a PR changes, by its number, URL or head branch.
    ///
    /// # Errors
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if `gh` cannot read it.
    async fn pr_files(&self, pr_ref: &str) -> Outcome<Vec<String>>;

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

    /// The branches whose name starts with `prefix` (`milestone/`), by name.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read fails.
    async fn branches(&self, prefix: &str) -> Outcome<Vec<String>>;

    /// Create `refs/heads/{branch}` at this sha.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn create_branch(&self, branch: &str, sha: &str) -> Outcome<()>;

    /// Merges `head` into `base` on GitHub (`POST merges`): nothing when
    /// `base` already holds every commit of `head`.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses — a
    /// conflict among the reasons.
    async fn merge_branch(&self, base: &str, head: &str) -> Outcome<()>;

    /// The paths `head` changed since its merge base with `base` — what a
    /// merge of `head` into `base` brings (GitHub's compare `base...head`),
    /// a renamed file under both names; `None` when GitHub lists only part
    /// of them (it stops at 300 files).
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the read
    /// fails — a branch or commit GitHub does not know among the reasons.
    async fn changed_files(&self, base: &str, head: &str) -> Outcome<Option<Vec<String>>>;

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

    /// Allow only squash merges on the repository: the squash commit is
    /// titled from the pull request's title with `(#n)` appended, its body
    /// from the pull request's body.
    ///
    /// Why: one commit per pull request on the integration branch, and the
    /// `(#n)` in its title keeps the link to the pull request, where the
    /// branch's own commits stay readable — nothing of the history is lost.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses — the
    /// token pushes to the repository but does not administer it.
    async fn require_squash_merges(&self) -> Outcome<()>;

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

    /// Writes `content` at `path` on `branch` as one commit saying
    /// `message` — created, or replaced when the file is there.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn put_file(&self, path: &str, branch: &str, content: &str, message: &str)
    -> Outcome<()>;

    /// Deletes `path` on `branch` as one commit saying `message`, made on
    /// GitHub against the branch's head — no checkout, no push to race.
    /// Answers whether it deleted it: `false` when the file is not there,
    /// so a second call changes nothing.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the file's
    /// blob cannot be read for any reason other than "absent";
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses the
    /// delete.
    async fn delete_file(&self, path: &str, branch: &str, message: &str) -> Outcome<bool>;

    // --- what the three-tier issue model demands, and nothing else ---------
    //
    // These are writes, so they share `issue`'s writes' failure shape
    // (`Halt::Halted`, "nothing is retried and nothing is undone") rather
    // than the PR-review group's `Halt::Failed` — a refused create/link here
    // is the same kind of event as a refused label or body write.

    /// Create an issue, returning its number.
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses — a
    /// body over [`ISSUE_BODY_MAX`] is refused before it is sent — or if it
    /// accepts the write but the response names no number to read back.
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

    /// The native GitHub milestone that mirrors the milestone issue
    /// `issue`, made to exist and to be `open` or closed as asked: its
    /// number.
    ///
    /// Found again by the marker its description carries
    /// (`<!-- harness-milestone: #<issue> -->`), created when absent —
    /// titled `#<issue> <title>`, cut to GitHub's 255 characters — and its
    /// state fixed when it differs. The milestone issue and its sub-issues
    /// stay the harness's own truth: this field is for the humans and
    /// GitHub's own views.
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the
    /// milestones cannot be listed;
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses the
    /// create or the state change.
    async fn ensure_native_milestone(&self, issue: u64, title: &str, open: bool) -> Outcome<u64>;

    /// Put `issue` on the native milestone numbered `milestone` (GitHub's
    /// Milestone field).
    ///
    /// # Errors
    /// [`Halt::Halted`](crate::domain::Halt::Halted) if GitHub refuses.
    async fn set_issue_milestone(&self, issue: u64, milestone: u64) -> Outcome<()>;

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

    /// Merge a pull request by squash — the one method `init-repo` leaves
    /// enabled, and the same `/code` uses by hand.
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

    /// Whether this PR can be merged into its base as it stands:
    /// `Some(false)` when its branch conflicts with the base — another PR
    /// landed there since — and `None` while GitHub is still computing it.
    ///
    /// # Errors
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if the PR cannot be read.
    async fn pr_mergeable(&self, pr_ref: &str) -> Outcome<Option<bool>>;

    /// Whether the workflow runs on this branch's newest head are green:
    /// `Some(true)` when every run on that sha completed successfully,
    /// `Some(false)` when one of them failed, `None` when none ran yet or
    /// one is still running.
    ///
    /// For a branch, not a pull request: the integration branch receives
    /// direct pushes and has no PR whose checks could be read. What the
    /// router holds on a `Some(false)` is everything that would build on
    /// the red base or merge into it.
    ///
    /// # Errors
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if the runs cannot be
    /// listed at all.
    async fn branch_checks(&self, branch: &str) -> Outcome<Option<bool>>;

    /// Whether the workflow runs of `branch` on the commit `sha` are green,
    /// judged as [`Self::branch_checks`] judges the newest head — for this
    /// sha alone: `None` while none ran on it yet (GitHub creates them
    /// seconds after a push, a `[skip ci]` push never) or one still runs.
    ///
    /// What ties a verdict to the head a caller read: the newest run of a
    /// branch just pushed to still belongs to the head before.
    ///
    /// # Errors
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if the runs cannot be
    /// listed at all.
    async fn commit_checks(&self, branch: &str, sha: &str) -> Outcome<Option<bool>>;

    /// When commit `sha` was committed (its committer date), in seconds
    /// since the Unix epoch — `None` when GitHub has no such commit or no
    /// readable date.
    ///
    /// What a wait on that commit's own checks counts from: the second it
    /// landed, not the second a watch first read it (a restart reads every
    /// head anew).
    ///
    /// # Errors
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) when the
    /// commit cannot be read.
    async fn commit_time(&self, sha: &str) -> Outcome<Option<u64>>;

    /// The file `file` of the artifact `artifact` the latest finished run of
    /// the workflow `workflow` on `branch` kept — `None` when no finished run
    /// is there, or it kept no such file. A board with no CI keeps nothing.
    ///
    /// # Errors
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if the runs cannot be
    /// listed at all.
    async fn artifact_file(
        &self,
        workflow: &str,
        branch: &str,
        artifact: &str,
        file: &str,
    ) -> Outcome<Option<Kept>> {
        let _ = (workflow, branch, artifact, file);
        Ok(None)
    }
}
