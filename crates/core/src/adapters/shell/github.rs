//! The `gh` binary, wrapped — the implementation of the [`GitHub`] port.
//!
//! Each call **names the repo** by its working directory, for the same reason
//! as `git.rs`: `gh` resolves a PR from its `cwd`, so a call launched from
//! another checkout would resolve a same-named issue in that one.
//!
//! **Issues go through `gh api`**, not `gh issue`. Sub-issues
//! (`issues/{n}/sub_issues`) and dependencies
//! (`issues/{n}/dependencies/blocked_by`) have no native flags in `gh`: since
//! half the model must pass through the raw API anyway, all of it does, rather
//! than leaving a reader guess which half does what.
//!
//! Nothing here decides, and no read that fails answers `[]` — what the port
//! promises, and what that invariant costs when broken, is documented with it
//! in [`ports::shell::github`](crate::ports::shell::github).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::Value;

use crate::adapters::shell::process;
use crate::domain::{Halt, Issue, Outcome, Pr};
use crate::ports::shell::github::GitHub;
use crate::ports::shell::process::Ran;

/// The called binary.
const BINARY: &str = "gh";

/// Enough for a personal repo.
///
/// And a reason not to depend on `--paginate`: its multi-page output is not a
/// single JSON document in all versions of `gh`, and a parser fooled by it
/// would return an empty array — that is, "nothing left to do".
const PER_PAGE: u32 = 100;

/// The ceiling for what gets paginated.
///
/// Beyond that, the read fails rather than return a truncated list: truncated,
/// it reads as a complete list.
const MAX_PAGES: u32 = 10;

/// A failed read, stated rather than returned empty.
fn unreadable(what: &str, detail: &str) -> Halt {
    Halt::Unreadable(format!(
        "impossible to read: {what} ({detail}) — what remains to do is unknown, \
         and reading that as 'nothing left to do' is what makes the harness open \
         a roadmap item no one asked for. Check `gh auth status` and the repo, \
         then retry."
    ))
}

/// An issue from the API, in the form the domain knows how to read.
///
/// Labels arrive as **objects** at most entry points and as **strings** at
/// others: both are accepted, because rejection here would stop a run for a
/// difference of form that does not matter.
fn issue_from(payload: &Value) -> Outcome<Issue> {
    let number = payload
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| unreadable("an issue", "no usable `number` field"))?;
    let labels = label_names(payload);
    Ok(Issue {
        number,
        title: text_at(payload, "title"),
        state: {
            let state = text_at(payload, "state");
            if state.is_empty() {
                "open".to_string()
            } else {
                state
            }
        },
        labels,
        body: text_at(payload, "body"),
        blocked_by: Vec::new(),
    })
}

/// A text field, or empty. `null` and missing read the same.
fn text_at(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The metadata a review skip rule demands, in one call.
const PR_FIELDS: &str = "number,baseRefName,headRefName,title,url,state,isDraft,labels";

/// The label names of a payload, whether they arrive as objects or as
/// strings — `gh issue` and `gh pr` do not agree on which.
fn label_names(payload: &Value) -> Vec<String> {
    payload
        .get("labels")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|label| match label {
                    Value::String(name) => Some(name.clone()),
                    other => other
                        .get("name")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A PR from the API, in the form the domain knows how to read.
fn pr_from(payload: &Value) -> Pr {
    Pr {
        num: payload
            .get("number")
            .and_then(Value::as_u64)
            .map_or_else(String::new, |n| n.to_string()),
        base: text_at(payload, "baseRefName"),
        head: text_at(payload, "headRefName"),
        title: text_at(payload, "title"),
        url: text_at(payload, "url"),
        state: {
            let state = text_at(payload, "state");
            if state.is_empty() {
                "OPEN".to_string()
            } else {
                state
            }
        },
        draft: payload
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        labels: label_names(payload),
    }
}

/// The issues from a list response, **PRs excluded**.
///
/// `/issues` also returns pull requests — GitHub models them as issues. A PR
/// accidentally bearing the requested label would then enter the response, and
/// a caller looking for its milestone would take it for one; the `pull_request`
/// key is what distinguishes them.
fn issues_from(payload: &Value, what: &str) -> Outcome<Vec<Issue>> {
    rows_of(payload, what)?
        .iter()
        .filter(|row| row.get("pull_request").is_none())
        .map(issue_from)
        .collect()
}

/// The **merged** PRs from the response, in the order received.
///
/// The filter stops there: `merged_at` is an API property, and reading it is
/// this adapter's job. Which PR *counts as proof of delivery* is a workflow
/// convention and is decided with it.
fn merged_from(payload: &Value, what: &str) -> Outcome<Vec<Issue>> {
    rows_of(payload, what)?
        .iter()
        .filter(|row| !matches!(row.get("merged_at"), None | Some(Value::Null)))
        .map(issue_from)
        .collect()
}

/// The check states that mean "this one has broken", as `gh pr checks`
/// reports them.
///
/// Deliberately **not** "everything that is not `SUCCESS`": `PENDING`,
/// `QUEUED` and `IN_PROGRESS` are a run still happening, and `SKIPPED` or
/// `NEUTRAL` are a run that decided it had nothing to say. Treating either
/// as a failure would send a repair at a PR with nothing wrong with it yet.
const FAILED_STATES: [&str; 6] = [
    "FAILURE",
    "ERROR",
    "CANCELLED",
    "TIMED_OUT",
    "STARTUP_FAILURE",
    "ACTION_REQUIRED",
];

/// The rows of a `gh pr checks --json name,state,link` response that have
/// concluded in failure, one readable line each.
fn failing_checks(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .filter(|row| {
            row.get("state")
                .and_then(Value::as_str)
                .is_some_and(|state| FAILED_STATES.contains(&state))
        })
        .map(|row| {
            let name = text_at(row, "name");
            let state = text_at(row, "state");
            let link = text_at(row, "link");
            let named = if name.is_empty() { "a check" } else { &name };
            if link.is_empty() {
                format!("{named} — {state}")
            } else {
                format!("{named} — {state} — {link}")
            }
        })
        .collect()
}

/// Whether a `gh pr checks --json state` response is green: at least one
/// check `SUCCESS`, and every other one `SUCCESS`, `SKIPPED` or `NEUTRAL`.
///
/// An empty list is **not** green — proof that CI ran is the point, not the
/// absence of a reason to refuse. A skipped check is not a refusal either: a
/// gate declared but not implemented yet (`if: false`) reports `SKIPPED`
/// forever, and read as red it would hold every PR of the repository back.
fn checks_are_green(rows: &[Value]) -> bool {
    rows.iter().any(|row| state_of(row) == "SUCCESS")
        && rows
            .iter()
            .all(|row| matches!(state_of(row), "SUCCESS" | "SKIPPED" | "NEUTRAL"))
}

/// A check row's `state`, or empty.
fn state_of(row: &Value) -> &str {
    row.get("state").and_then(Value::as_str).unwrap_or("")
}

/// The array from a response. `null` counts as empty, anything else is unreadable.
fn rows_of<'a>(payload: &'a Value, what: &str) -> Outcome<&'a [Value]> {
    match payload {
        Value::Array(rows) => Ok(rows),
        Value::Null => Ok(&[]),
        other => Err(unreadable(what, &format!("expected an array, got {other}"))),
    }
}

/// `gh`, called from a given repo.
pub struct GhCli {
    root: PathBuf,
    /// `owner/name`, cached after the first request.
    ///
    /// An `OnceLock`, not a `RefCell`: the semantics are exactly this — written
    /// once, read thereafter — and it is `Sync`, so it does not make this
    /// module's futures non-`Send` for memoization. It also removes the `RefCell`
    /// pitfall: there is no borrow to keep open across an `await`.
    name: OnceLock<String>,
}

impl GhCli {
    /// `gh`, on this repo.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            name: OnceLock::new(),
        }
    }

    /// `gh`, with no checkout: `name` is pre-filled, so [`GitHub::repo`]
    /// short-circuits and no `gh repo view` ever runs — no current directory
    /// is consulted for *which repo this is*. `init-repo` has a URL and no
    /// checkout; that is the whole reason this exists.
    ///
    /// `root` is still set, only as the subprocess's spawn directory: every
    /// call below is fully qualified by `repos/{slug}/…` and reads nothing
    /// from `cwd`.
    #[must_use]
    pub fn for_slug(slug: &crate::domain::Slug) -> Self {
        Self {
            root: PathBuf::from("."),
            name: OnceLock::from(slug.to_string()),
        }
    }

    async fn gh(&self, args: &[String]) -> Outcome<Ran> {
        // `gh pr …` infers the repo from the cwd, and a slug-only adapter has
        // none that means anything (it would hit the harness's own repo), so
        // name the target explicitly. `api` calls already carry the slug.
        if args.first().is_some_and(|a| a == "pr")
            && let Some(name) = self.name.get()
        {
            let mut named = args.to_vec();
            named.push("-R".to_string());
            named.push(name.clone());
            return process::run(BINARY, &named, &self.root).await;
        }
        process::run(BINARY, args, &self.root).await
    }

    /// An API read. `path` is relative to the repo.
    ///
    /// The repo is resolved here rather than by the caller: it is the only way
    /// an unreadable repo name is a failure propagated like any other.
    async fn read(&self, what: &str, path: &str, query: &[(&str, String)]) -> Outcome<Value> {
        let repo = self.repo().await?;
        let mut args = vec![
            "api".to_string(),
            format!("repos/{repo}/{path}"),
            "-X".to_string(),
            "GET".to_string(),
        ];
        for (key, value) in query {
            args.push("-f".to_string());
            args.push(format!("{key}={value}"));
        }
        let ran = self.gh(&args).await?;
        if !ran.ok() {
            return Err(unreadable(what, &ran.why()));
        }
        let body = ran.out();
        if body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(body).map_err(|e| unreadable(what, &e.to_string()))
    }

    /// An API write.
    ///
    /// Nothing is retried and nothing is undone: a partially applied change is
    /// easier to finish by hand than to guess.
    async fn write(
        &self,
        what: &str,
        method: &str,
        path: &str,
        fields: &[(&str, String)],
    ) -> Outcome<()> {
        let repo = self.repo().await?;
        let url = if path.is_empty() {
            format!("repos/{repo}")
        } else {
            format!("repos/{repo}/{path}")
        };
        let mut args = vec![
            "api".to_string(),
            "-X".to_string(),
            method.to_string(),
            url.clone(),
        ];
        for (key, value) in fields {
            args.push("-f".to_string());
            args.push(format!("{key}={value}"));
        }
        let ran = self.gh(&args).await?;
        if ran.ok() {
            return Ok(());
        }
        Err(Halt::Halted(format!(
            "GitHub refused to {what} ({method} {url}): {}. Nothing is retried \
             and nothing is undone here — a partially applied change is easier \
             to finish by hand than to guess.",
            ran.why()
        )))
    }

    /// Issues from an entry point that lists them, read as-is.
    async fn issue_list(&self, what: &str, path: &str) -> Outcome<Vec<Issue>> {
        let payload = self
            .read(what, path, &[("per_page", PER_PAGE.to_string())])
            .await?;
        rows_of(&payload, what)?.iter().map(issue_from).collect()
    }

    /// `GET repos/{slug}`, read once per call — shared by [`GitHub::default_branch`]
    /// and [`GitHub::can_push`], which each want one field of it.
    async fn repo_json(&self, what: &str) -> Outcome<Value> {
        let repo = self.repo().await?;
        let ran = self
            .gh(&[
                "api".to_string(),
                format!("repos/{repo}"),
                "-X".to_string(),
                "GET".to_string(),
            ])
            .await?;
        if !ran.ok() {
            return Err(unreadable(what, &ran.why()));
        }
        serde_json::from_str(ran.out()).map_err(|e| unreadable(what, &e.to_string()))
    }

    /// An issue's internal node id — what the sub-issues/dependencies API
    /// actually wants, distinct from the number everything else here uses.
    async fn node_id(&self, number: u64) -> Outcome<u64> {
        let what = format!("internal id of #{number}");
        let payload = self.read(&what, &format!("issues/{number}"), &[]).await?;
        payload
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| unreadable(&what, "no usable `id` field"))
    }

    /// An API write whose response body is the point — unlike [`Self::write`],
    /// which discards it. `-f`: every field here is a plain string.
    async fn create(&self, what: &str, path: &str, fields: &[(&str, String)]) -> Outcome<Value> {
        let repo = self.repo().await?;
        let url = format!("repos/{repo}/{path}");
        let mut args = vec![
            "api".to_string(),
            "-X".to_string(),
            "POST".to_string(),
            url.clone(),
        ];
        for (key, value) in fields {
            args.push("-f".to_string());
            args.push(format!("{key}={value}"));
        }
        let ran = self.gh(&args).await?;
        if !ran.ok() {
            return Err(Halt::Halted(format!(
                "GitHub refused to {what} (POST {url}): {}. Nothing is retried \
                 and nothing is undone here — a partially applied change is \
                 easier to finish by hand than to guess.",
                ran.why()
            )));
        }
        serde_json::from_str(ran.out())
            .map_err(|e| Halt::Halted(format!("{what}: unreadable response: {e}")))
    }

    /// Like [`Self::create`], but with `-F` fields: GitHub's sub-issues and
    /// dependencies endpoints want a typed integer (`sub_issue_id`,
    /// `issue_id`), and `-f` would send it as a JSON string instead.
    async fn link(&self, what: &str, path: &str, fields: &[(&str, String)]) -> Outcome<()> {
        let repo = self.repo().await?;
        let url = format!("repos/{repo}/{path}");
        let mut args = vec![
            "api".to_string(),
            "-X".to_string(),
            "POST".to_string(),
            url.clone(),
        ];
        for (key, value) in fields {
            args.push("-F".to_string());
            args.push(format!("{key}={value}"));
        }
        let ran = self.gh(&args).await?;
        if ran.ok() {
            return Ok(());
        }
        Err(Halt::Halted(format!(
            "GitHub refused to {what} (POST {url}): {}. Nothing is retried \
             and nothing is undone here — a partially applied change is \
             easier to finish by hand than to guess.",
            ran.why()
        )))
    }

    /// Runs a fully-built `gh api` call where absence is a valid answer.
    ///
    /// The delicate part: a 404 and a failed call must not collapse into the
    /// same `None`. Non-zero exits either way, so the split comes from
    /// reading `gh`'s own diagnostic, not its exit code.
    async fn read_optional(&self, what: &str, args: &[String]) -> Outcome<Option<Ran>> {
        let ran = self.gh(args).await?;
        if ran.ok() {
            return Ok(Some(ran));
        }
        if is_404(&ran) {
            return Ok(None);
        }
        Err(unreadable(what, &ran.why()))
    }
}

/// Whether `gh`'s own diagnostic names a 404 — confirmed against a real
/// `gh api` call on an absent resource: `gh` prints `gh: Not Found (HTTP
/// 404)` on its last stderr line and exits 1, with and without a custom
/// `Accept` header.
///
/// Pure, so the 404/failure split is unit-testable without a subprocess.
fn is_404(ran: &Ran) -> bool {
    ran.why().contains("HTTP 404")
}

#[async_trait(?Send)]
impl GitHub for GhCli {
    async fn authenticated(&self) -> Outcome<bool> {
        Ok(self
            .gh(&["auth".to_string(), "status".to_string()])
            .await?
            .ok())
    }

    async fn repo(&self) -> Outcome<String> {
        if let Some(name) = self.name.get() {
            return Ok(name.clone());
        }
        let ran = self
            .gh(&[
                "repo".to_string(),
                "view".to_string(),
                "--json".to_string(),
                "nameWithOwner".to_string(),
                "-q".to_string(),
                ".nameWithOwner".to_string(),
            ])
            .await?;
        if !ran.ok() || ran.out().is_empty() {
            return Err(unreadable("the repo name", &ran.why()));
        }
        let name = ran.out().to_string();
        // `set` fails if another call won the race: the value is the same, so
        // there is nothing to catch up on.
        let _ = self.name.set(name.clone());
        Ok(name)
    }

    async fn labels(&self) -> Outcome<Vec<String>> {
        let what = "the repo's labels";
        let payload = self
            .read(what, "labels", &[("per_page", PER_PAGE.to_string())])
            .await?;
        Ok(rows_of(&payload, what)?
            .iter()
            .filter_map(|row| row.get("name").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect())
    }

    async fn issue(&self, number: u64) -> Outcome<Issue> {
        let what = format!("issue #{number}");
        let payload = self.read(&what, &format!("issues/{number}"), &[]).await?;
        issue_from(&payload)
    }

    async fn issues_labelled(&self, label: &str, state: &str) -> Outcome<Vec<Issue>> {
        let what = format!("issues {label}");
        let payload = self
            .read(
                &what,
                "issues",
                &[
                    ("labels", label.to_string()),
                    ("state", state.to_string()),
                    ("per_page", PER_PAGE.to_string()),
                ],
            )
            .await?;
        issues_from(&payload, &what)
    }

    async fn sub_issues(&self, number: u64) -> Outcome<Vec<Issue>> {
        self.issue_list(
            &format!("sub-issues of #{number}"),
            &format!("issues/{number}/sub_issues"),
        )
        .await
    }

    async fn blocked_by(&self, number: u64) -> Outcome<Vec<Issue>> {
        self.issue_list(
            &format!("what blocks #{number}"),
            &format!("issues/{number}/dependencies/blocked_by"),
        )
        .await
    }

    async fn with_blockers(&self, tasks: Vec<Issue>) -> Outcome<Vec<Issue>> {
        let mut out = Vec::with_capacity(tasks.len());
        for mut task in tasks {
            task.blocked_by = self.blocked_by(task.number).await?;
            out.push(task);
        }
        Ok(out)
    }

    async fn merged_prs(&self, base: &str) -> Outcome<Vec<Issue>> {
        let what = format!("merged pull requests on {base}");
        let payload = self
            .read(
                &what,
                "pulls",
                &[
                    ("state", "closed".to_string()),
                    ("base", base.to_string()),
                    ("sort", "updated".to_string()),
                    ("direction", "desc".to_string()),
                    ("per_page", PER_PAGE.to_string()),
                ],
            )
            .await?;
        merged_from(&payload, &what)
    }

    async fn issue_comments(&self, number: u64) -> Outcome<Vec<String>> {
        let what = format!("comments on issue #{number}");
        let mut bodies = Vec::new();
        // Paginated, unlike other reads: GitHub returns comments oldest to
        // newest, so a first page alone loses the latest — and a round count
        // read from a truncated list goes backward, over work already paid for.
        for page in 1..=MAX_PAGES {
            let payload = self
                .read(
                    &what,
                    &format!("issues/{number}/comments"),
                    &[
                        ("per_page", PER_PAGE.to_string()),
                        ("page", page.to_string()),
                    ],
                )
                .await?;
            let rows = rows_of(&payload, &what)?;
            let count = rows.len();
            bodies.extend(rows.iter().map(|row| text_at(row, "body")));
            if count < PER_PAGE as usize {
                return Ok(bodies);
            }
        }
        Err(unreadable(
            &what,
            &format!("more than {} comments", MAX_PAGES * PER_PAGE),
        ))
    }

    async fn add_label(&self, number: u64, label: &str) -> Outcome<()> {
        self.write(
            &format!("label #{number} with {label}"),
            "POST",
            &format!("issues/{number}/labels"),
            &[("labels[]", label.to_string())],
        )
        .await
    }

    async fn remove_label(&self, number: u64, label: &str) -> Outcome<()> {
        self.write(
            &format!("remove {label} from #{number}"),
            "DELETE",
            &format!("issues/{number}/labels/{label}"),
            &[],
        )
        .await
    }

    async fn set_body(&self, number: u64, body: &str) -> Outcome<()> {
        self.write(
            &format!("rewrite the body of #{number}"),
            "PATCH",
            &format!("issues/{number}"),
            &[("body", body.to_string())],
        )
        .await
    }

    async fn post_issue_comment(&self, number: u64, body: &str) -> Outcome<()> {
        self.write(
            &format!("comment on #{number}"),
            "POST",
            &format!("issues/{number}/comments"),
            &[("body", body.to_string())],
        )
        .await
    }

    async fn close_issue(&self, number: u64) -> Outcome<()> {
        self.write(
            &format!("close #{number}"),
            "PATCH",
            &format!("issues/{number}"),
            &[("state", "closed".to_string())],
        )
        .await
    }

    async fn pr(&self, pr_ref: &str) -> Outcome<Pr> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "view".to_string(),
                pr_ref.to_string(),
                "--json".to_string(),
                PR_FIELDS.to_string(),
            ])
            .await?;
        if !ran.ok() {
            return Err(Halt::Failed(format!(
                "cannot read PR {pr_ref} — {}",
                ran.why()
            )));
        }
        serde_json::from_str::<Value>(ran.out())
            .map_err(|e| Halt::Failed(format!("cannot read PR {pr_ref} — {e}")))
            .map(|value| pr_from(&value))
    }

    async fn pr_comments(&self, num: &str) -> Outcome<String> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "view".to_string(),
                num.to_string(),
                "--json".to_string(),
                "comments".to_string(),
                "-q".to_string(),
                ".comments[].body".to_string(),
            ])
            .await?;
        if !ran.ok() {
            return Err(Halt::Failed(format!(
                "cannot tell whether PR #{num} was already reviewed — {}",
                ran.why()
            )));
        }
        Ok(ran.stdout)
    }

    async fn post_pr_comment(&self, num: &str, body_file: &Path) -> Outcome<()> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "comment".to_string(),
                num.to_string(),
                "--body-file".to_string(),
                body_file.display().to_string(),
            ])
            .await?;
        if ran.ok() {
            return Ok(());
        }
        Err(Halt::Failed(format!(
            "exit {}: {}",
            ran.code.unwrap_or(-1),
            ran.why()
        )))
    }

    async fn create_label(&self, name: &str, color: &str, description: &str) -> Outcome<()> {
        self.write(
            &format!("create label {name}"),
            "POST",
            "labels",
            &[
                ("name", name.to_string()),
                ("color", color.to_string()),
                ("description", description.to_string()),
            ],
        )
        .await
    }

    async fn branch_sha(&self, branch: &str) -> Outcome<Option<String>> {
        let what = format!("sha of {branch}");
        let repo = self.repo().await?;
        let args = vec![
            "api".to_string(),
            format!("repos/{repo}/git/ref/heads/{branch}"),
            "-X".to_string(),
            "GET".to_string(),
        ];
        let Some(ran) = self.read_optional(&what, &args).await? else {
            return Ok(None);
        };
        let value: Value =
            serde_json::from_str(ran.out()).map_err(|e| unreadable(&what, &e.to_string()))?;
        let sha = value
            .get("object")
            .and_then(|o| o.get("sha"))
            .and_then(Value::as_str)
            .ok_or_else(|| unreadable(&what, "no usable object.sha field"))?;
        Ok(Some(sha.to_string()))
    }

    async fn create_branch(&self, branch: &str, sha: &str) -> Outcome<()> {
        self.write(
            &format!("create branch {branch}"),
            "POST",
            "git/refs",
            &[
                ("ref", format!("refs/heads/{branch}")),
                ("sha", sha.to_string()),
            ],
        )
        .await
    }

    async fn default_branch(&self) -> Outcome<String> {
        let what = "the default branch";
        let value = self.repo_json(what).await?;
        value
            .get("default_branch")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| unreadable(what, "no usable default_branch field"))
    }

    async fn set_default_branch(&self, branch: &str) -> Outcome<()> {
        self.write(
            &format!("make {branch} the default branch"),
            "PATCH",
            "",
            &[("default_branch", branch.to_string())],
        )
        .await
    }

    async fn protect_branch(&self, branch: &str) -> Outcome<()> {
        // `-F` types its value (`null`, numbers) and understands the bracket
        // form for nested objects, which the string-only `write` cannot send.
        let repo = self.repo().await?;
        let url = format!("repos/{repo}/branches/{branch}/protection");
        let mut args = vec![
            "api".to_string(),
            "-X".to_string(),
            "PUT".to_string(),
            url.clone(),
        ];
        for field in [
            "required_status_checks=null",
            "enforce_admins=false",
            "restrictions=null",
            "required_pull_request_reviews[required_approving_review_count]=0",
        ] {
            args.push("-F".to_string());
            args.push(field.to_string());
        }
        let ran = self.gh(&args).await?;
        if ran.ok() {
            return Ok(());
        }
        Err(Halt::Halted(format!(
            "GitHub refused to protect {branch} (PUT {url}): {}",
            ran.why()
        )))
    }

    async fn can_push(&self) -> Outcome<bool> {
        let what = "push permission";
        let value = self.repo_json(what).await?;
        value
            .get("permissions")
            .and_then(|p| p.get("push"))
            .and_then(Value::as_bool)
            .ok_or_else(|| unreadable(what, "no usable permissions.push field"))
    }

    async fn file_text(&self, path: &str, git_ref: &str) -> Outcome<Option<String>> {
        let what = format!("{path}@{git_ref}");
        let repo = self.repo().await?;
        let args = vec![
            "api".to_string(),
            format!("repos/{repo}/contents/{path}"),
            "-X".to_string(),
            "GET".to_string(),
            "-f".to_string(),
            format!("ref={git_ref}"),
            "-H".to_string(),
            "Accept: application/vnd.github.raw".to_string(),
        ];
        let Some(ran) = self.read_optional(&what, &args).await? else {
            return Ok(None);
        };
        Ok(Some(ran.stdout))
    }

    async fn create_issue(&self, title: &str, body: &str, labels: &[&str]) -> Outcome<u64> {
        let mut fields = vec![("title", title.to_string()), ("body", body.to_string())];
        for label in labels {
            fields.push(("labels[]", (*label).to_string()));
        }
        let payload = self.create("create an issue", "issues", &fields).await?;
        payload
            .get("number")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                Halt::Halted("created an issue but the response named no number".to_string())
            })
    }

    async fn create_sub_issue_link(&self, parent: u64, child: u64) -> Outcome<()> {
        let child_id = self.node_id(child).await?;
        self.link(
            &format!("link #{child} as a sub-issue of #{parent}"),
            &format!("issues/{parent}/sub_issues"),
            &[("sub_issue_id", child_id.to_string())],
        )
        .await
    }

    async fn add_blocked_by(&self, number: u64, blocker: u64) -> Outcome<()> {
        let blocker_id = self.node_id(blocker).await?;
        self.link(
            &format!("mark #{number} as blocked by #{blocker}"),
            &format!("issues/{number}/dependencies/blocked_by"),
            &[("issue_id", blocker_id.to_string())],
        )
        .await
    }

    async fn create_pr(&self, head: &str, base: &str, title: &str, body: &str) -> Outcome<String> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "create".to_string(),
                "--head".to_string(),
                head.to_string(),
                "--base".to_string(),
                base.to_string(),
                "--title".to_string(),
                title.to_string(),
                "--body".to_string(),
                body.to_string(),
            ])
            .await?;
        if !ran.ok() {
            return Err(Halt::Halted(format!(
                "GitHub refused to open a PR from {head} into {base}: {}. Nothing \
                 is retried and nothing is undone here — a partially applied \
                 change is easier to finish by hand than to guess.",
                ran.why()
            )));
        }
        let url = ran.out().to_string();
        if url.is_empty() {
            return Err(Halt::Halted(format!(
                "opened a PR from {head} into {base} but gh printed nothing to \
                 read its URL back"
            )));
        }
        Ok(url)
    }

    async fn merge_pr(&self, pr_ref: &str) -> Outcome<()> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "merge".to_string(),
                pr_ref.to_string(),
                "--rebase".to_string(),
            ])
            .await?;
        if ran.ok() {
            return Ok(());
        }
        Err(Halt::Halted(format!(
            "GitHub refused to merge {pr_ref}: {}. Nothing is retried and \
             nothing is undone here — a partially applied change is easier to \
             finish by hand than to guess.",
            ran.why()
        )))
    }

    async fn pr_checks_green(&self, pr_ref: &str) -> Outcome<bool> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "checks".to_string(),
                pr_ref.to_string(),
                "--json".to_string(),
                "state".to_string(),
            ])
            .await?;
        // `gh pr checks` exits non-zero whenever a check is pending or
        // failing — a normal answer, not a failed read. Success is read from
        // the JSON body, which is printed either way.
        if ran.out().is_empty() {
            return Err(Halt::Halted(format!(
                "could not read CI status for {pr_ref} — {}",
                ran.why()
            )));
        }
        let rows: Vec<Value> = serde_json::from_str(ran.out())
            .map_err(|e| Halt::Halted(format!("unreadable CI status for {pr_ref}: {e}")))?;
        Ok(checks_are_green(&rows))
    }

    async fn open_prs_labelled(&self, label: &str) -> Outcome<Vec<Pr>> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "list".to_string(),
                "--label".to_string(),
                label.to_string(),
                "--state".to_string(),
                "open".to_string(),
                "--json".to_string(),
                PR_FIELDS.to_string(),
            ])
            .await?;
        if !ran.ok() {
            return Err(Halt::Failed(format!(
                "cannot list the open PRs carrying {label} — {}",
                ran.why()
            )));
        }
        let payload: Value = serde_json::from_str(ran.out()).map_err(|e| {
            Halt::Failed(format!("cannot list the open PRs carrying {label} — {e}"))
        })?;
        Ok(rows_of(&payload, &format!("open PRs carrying {label}"))?
            .iter()
            .map(pr_from)
            .collect())
    }

    async fn pr_failing_checks(&self, pr_ref: &str) -> Outcome<Vec<String>> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "checks".to_string(),
                pr_ref.to_string(),
                "--json".to_string(),
                "name,state,link".to_string(),
            ])
            .await?;
        // Same contract as `pr_checks_green`: a non-zero exit is how `gh`
        // reports "something is pending or failing", which is precisely the
        // answer being asked for. Only an empty body is unreadable.
        if ran.out().is_empty() {
            return Err(Halt::Halted(format!(
                "could not read CI status for {pr_ref} — {}",
                ran.why()
            )));
        }
        let rows: Vec<Value> = serde_json::from_str(ran.out())
            .map_err(|e| Halt::Halted(format!("unreadable CI status for {pr_ref}: {e}")))?;
        Ok(failing_checks(&rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- les étiquettes, en objets ou en chaînes ----------------------------

    #[test]
    fn labels_are_read_whether_they_arrive_as_objects_or_strings() {
        let as_objects = json!({
            "number": 12,
            "labels": [{ "name": "pipeline:agent" }, { "name": "pipeline:ready" }]
        });
        let as_strings = json!({
            "number": 12,
            "labels": ["pipeline:agent", "pipeline:ready"]
        });
        let wanted = vec!["pipeline:agent".to_string(), "pipeline:ready".to_string()];
        assert_eq!(issue_from(&as_objects).expect("objects").labels, wanted);
        assert_eq!(issue_from(&as_strings).expect("strings").labels, wanted);
    }

    #[test]
    fn a_missing_number_is_unreadable_rather_than_a_zero() {
        let err = issue_from(&json!({ "title": "no number" })).expect_err("should fail");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[test]
    fn a_null_body_reads_as_empty_not_as_the_word_null() {
        let issue = issue_from(&json!({ "number": 1, "body": null })).expect("parse");
        assert_eq!(issue.body, "");
    }

    #[test]
    fn a_missing_state_defaults_to_open_so_work_is_not_lost() {
        let issue = issue_from(&json!({ "number": 1 })).expect("parse");
        assert!(issue.is_open());
    }

    // --- invariant: never an empty list on failure --------------------------

    #[test]
    fn a_null_list_is_genuinely_empty() {
        // `gh` returns `null` when there is nothing: harmless and distinct
        // from a failure.
        assert_eq!(
            issues_from(&Value::Null, "issues").expect("null"),
            [] as [Issue; 0]
        );
    }

    #[test]
    fn a_response_that_is_not_a_list_is_unreadable_not_empty() {
        // The failure mode it prevents: an unexpected response read as
        // "nothing left to do", which triggers a /planner.
        let err = issues_from(&json!({ "message": "Bad credentials" }), "issues")
            .expect_err("should fail");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[test]
    fn the_unreadable_message_names_the_gesture_that_unblocks() {
        let Halt::Unreadable(said) = unreadable("issues", "401") else {
            panic!("should be Unreadable");
        };
        assert!(said.contains("gh auth status"));
    }

    // --- pull requests -------------------------------------------------------

    #[test]
    fn pull_requests_are_filtered_out_of_an_issue_listing() {
        // GitHub models PRs as issues: a PR accidentally bearing the requested
        // label would be mistaken for a milestone.
        let payload = json!([
            { "number": 1, "title": "a real issue" },
            { "number": 2, "title": "a PR", "pull_request": { "url": "..." } },
        ]);
        let issues = issues_from(&payload, "issues").expect("parse");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].number, 1);
    }

    #[test]
    fn only_merged_pull_requests_come_back() {
        let payload = json!([
            { "number": 1, "merged_at": "2026-09-30T10:00:00Z" },
            { "number": 2, "merged_at": null },
            { "number": 3 },
        ]);
        let merged = merged_from(&payload, "PRs").expect("parse");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].number, 1);
    }

    #[test]
    fn a_pr_is_read_from_its_own_fields_not_the_issue_shape() {
        let payload = json!({
            "number": 32,
            "baseRefName": "main_agent",
            "headRefName": "feat/cities",
            "title": "feat(db): table cities",
            "url": "https://github.com/o/r/pull/32",
            "state": "MERGED",
            "isDraft": false
        });
        let pr = pr_from(&payload);
        assert_eq!(pr.num, "32");
        assert_eq!(pr.base, "main_agent");
        assert_eq!(pr.state, "MERGED");
        assert!(!pr.draft);
    }

    #[test]
    fn a_missing_state_on_a_pr_defaults_to_open() {
        let pr = pr_from(&json!({ "number": 1 }));
        assert_eq!(pr.state, "OPEN");
    }

    #[test]
    fn an_unreadable_pull_request_list_does_not_read_as_nothing_delivered() {
        // "API is down" and "nothing was delivered" lead to opposite decisions.
        let err =
            merged_from(&json!({ "message": "rate limited" }), "PRs").expect_err("should fail");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    // --- init-repo: a 404 must never collapse into an ordinary failure ------

    fn ran(code: i32, stderr: &str) -> Ran {
        Ran {
            code: Some(code),
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn a_404_is_recognized_from_ghs_own_diagnostic() {
        assert!(is_404(&ran(1, "gh: Not Found (HTTP 404)")));
    }

    #[test]
    fn a_non_404_failure_is_not_mistaken_for_an_absent_resource() {
        // The regression this prevents: an expired token reporting
        // "ci.yml is missing" and sending a human to edit a file that is
        // already correct.
        assert!(!is_404(&ran(1, "gh: Bad credentials (HTTP 401)")));
        assert!(!is_404(&ran(1, "(nothing on stderr)")));
    }

    // --- checks_are_green ----------------------------------------------------

    #[test]
    fn every_check_succeeding_is_green() {
        let rows = vec![json!({"state": "SUCCESS"}), json!({"state": "SUCCESS"})];
        assert!(checks_are_green(&rows));
    }

    #[test]
    fn one_failing_check_is_not_green() {
        let rows = vec![json!({"state": "SUCCESS"}), json!({"state": "FAILURE"})];
        assert!(!checks_are_green(&rows));
    }

    #[test]
    fn one_pending_check_is_not_green() {
        let rows = vec![json!({"state": "SUCCESS"}), json!({"state": "PENDING"})];
        assert!(!checks_are_green(&rows));
    }

    #[test]
    fn a_skipped_gate_does_not_hold_a_green_pr_back_but_is_not_green_alone() {
        let rows = vec![json!({"state": "SUCCESS"}), json!({"state": "SKIPPED"})];
        assert!(checks_are_green(&rows));
        assert!(!checks_are_green(&[json!({"state": "SKIPPED"})]));
    }

    #[test]
    fn no_checks_at_all_is_not_green() {
        // Proof CI ran is the point, not the absence of a reason to refuse.
        assert!(!checks_are_green(&[]));
    }

    // --- failing_checks ------------------------------------------------------

    #[test]
    fn only_the_concluded_failures_are_listed_with_their_link() {
        let rows = vec![
            json!({"name": "ci", "state": "FAILURE", "link": "https://x/1"}),
            json!({"name": "fmt", "state": "SUCCESS", "link": "https://x/2"}),
        ];
        assert_eq!(
            failing_checks(&rows),
            ["ci — FAILURE — https://x/1".to_string()]
        );
    }

    #[test]
    fn a_pending_check_is_not_a_failure_to_repair() {
        // The distinction this function exists for: paying a repair session
        // while CI is still running fixes nothing and costs the same.
        for state in ["PENDING", "QUEUED", "IN_PROGRESS", "SKIPPED", "NEUTRAL"] {
            let rows = vec![json!({"name": "ci", "state": state})];
            assert!(
                failing_checks(&rows).is_empty(),
                "{state} must not read as a failure"
            );
        }
    }

    #[test]
    fn every_way_a_run_can_break_is_a_failure() {
        for state in FAILED_STATES {
            let rows = vec![json!({"name": "ci", "state": state})];
            assert_eq!(failing_checks(&rows).len(), 1, "{state} must read as one");
        }
    }

    #[test]
    fn a_failure_without_a_link_or_a_name_still_says_something() {
        let rows = vec![json!({"state": "FAILURE"})];
        assert_eq!(failing_checks(&rows), ["a check — FAILURE".to_string()]);
    }

    #[test]
    fn no_checks_at_all_means_nothing_to_repair() {
        // Unlike `checks_are_green`, where an empty list refuses: there is
        // no broken run to send a session at.
        assert_eq!(failing_checks(&[]), [] as [std::string::String; 0]);
    }
}
