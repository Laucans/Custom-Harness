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
//! in [`ports::shell::github`](harness_core::ports::shell::github).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use crate::shell::process;
use harness_core::domain::{Halt, Issue, Outcome, Pr};
use harness_core::ports::shell::github::{Activity, GitHub, ISSUE_BODY_MAX, Kept};
use harness_core::ports::shell::process::Ran;

/// The called binary.
const BINARY: &str = "gh";

/// How long a call GitHub throttled waits before its second try.
const THROTTLE_PAUSE: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether a label removal found no such label on the issue — done
/// already, as far as the caller cares.
#[must_use]
fn label_absent(ran: &Ran) -> bool {
    !ran.ok() && ran.stderr.contains("Label does not exist")
}

/// Whether `gh` was refused for a burst: GitHub's secondary rate limit
/// (or its primary one, which the same pause cannot help, but a tick later
/// is as good a time to find out), as `gh` prints it.
#[must_use]
fn throttled(ran: &Ran) -> bool {
    !ran.ok()
        && (ran.stderr.contains("rate limit exceeded")
            || ran.stderr.contains("secondary rate limit")
            || ran.stderr.contains("abuse detection"))
}

/// A write GitHub took but whose answer `gh` lost: its body came back empty
/// or cut, and `gh` failed to parse it. The write itself may well have
/// landed (#125's comment, `ForgeDesMondes` 2026-10-20 14:13Z).
fn answer_lost(ran: &Ran) -> bool {
    !ran.ok() && ran.stderr.contains(ANSWER_LOST)
}

/// What `gh` prints when the answer it parses is empty or cut.
const ANSWER_LOST: &str = "unexpected end of JSON input";

/// The halt a refused write ends on.
fn write_refused(what: &str, method: &str, url: &str, ran: &Ran) -> Halt {
    Halt::Halted(format!(
        "GitHub refused to {what} ({method} {url}): {}. Nothing is retried \
         and nothing is undone here — a partially applied change is easier \
         to finish by hand than to guess.",
        refusal(&ran.why(), ran.out())
    ))
}

/// Whether a write may be sent a second time without doing it twice.
fn idempotent(method: &str) -> bool {
    matches!(method, "PATCH" | "PUT" | "DELETE")
}

/// Whether a write is sent again: its answer was lost and repeating it
/// cannot do it twice.
fn repeats(ran: &Ran, method: &str) -> bool {
    answer_lost(ran) && idempotent(method)
}

/// A comment's post, confirmed when `gh` lost GitHub's answer: the issue's
/// comments are read once (`latest`), and the post stands if the comment is
/// there. Any other outcome is the post's own.
async fn confirmed<F, Fut>(posted: Outcome<()>, body: &str, latest: F) -> Outcome<()>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Outcome<Vec<String>>>,
{
    match posted {
        Err(Halt::Halted(why)) if why.contains(ANSWER_LOST) => {
            if comment_landed(&latest().await?, body) {
                Ok(())
            } else {
                Err(Halt::Halted(why))
            }
        }
        other => other,
    }
}

/// Whether the comment just posted is the issue's latest — the proof a
/// post whose answer was lost landed after all.
fn comment_landed(bodies: &[String], body: &str) -> bool {
    bodies.last().is_some_and(|last| last.trim() == body.trim())
}

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

/// Why GitHub refused a write: `gh`'s last line (`Validation Failed (HTTP
/// 422)`) and, when the response body says more, GitHub's own reasons
/// (`body is too long (maximum is 65536 characters)`).
fn refusal(why: &str, response: &str) -> String {
    let Ok(payload) = serde_json::from_str::<Value>(response) else {
        return why.to_string();
    };
    let reasons: Vec<String> = payload
        .get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| match e {
            Value::String(text) => Some(text.clone()),
            _ => e
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    let field = e.get("field").and_then(Value::as_str)?;
                    let code = e.get("code").and_then(Value::as_str).unwrap_or("invalid");
                    Some(format!("{field}: {code}"))
                }),
        })
        .collect();
    if reasons.is_empty() {
        why.to_string()
    } else {
        format!("{why} — {}", reasons.join("; "))
    }
}

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

/// `gh pr view --json mergeable`: `MERGEABLE`, `CONFLICTING`, or `UNKNOWN`
/// while GitHub still computes it.
fn mergeable_from(payload: &Value) -> Option<bool> {
    match payload.get("mergeable").and_then(Value::as_str) {
        Some("MERGEABLE") => Some(true),
        Some("CONFLICTING") => Some(false),
        _ => None,
    }
}

/// The check rows `gh pr checks --json` printed, as the two CI reads parse
/// them.
///
/// `gh` prints nothing and says "no checks reported" on a PR GitHub runs
/// no check for — a PR in conflict with its base is one (no `pull_request`
/// run is created for it): that is no check at all, not a failed read, and
/// `ForgeDesMondes`' #251 made every tick fail on it while it waited for the
/// repair that would have resolved the conflict. Nothing printed for any
/// other reason is still a read that failed.
fn ci_rows(pr_ref: &str, ran: &Ran) -> Outcome<Vec<Value>> {
    if ran.out().is_empty() {
        if ran.why().contains("no checks reported") {
            return Ok(Vec::new());
        }
        return Err(Halt::Halted(format!(
            "could not read CI status for {pr_ref} — {}",
            ran.why()
        )));
    }
    serde_json::from_str(ran.out())
        .map_err(|e| Halt::Halted(format!("unreadable CI status for {pr_ref}: {e}")))
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

/// What `gh run list --json headSha,status,conclusion,name` says of a
/// branch: the runs of its newest head sha — the first row's, since `gh`
/// lists the newest run first — read as [`GitHub::branch_checks`] answers.
///
/// A failed run is final for that sha, so one is `Some(false)` whatever
/// still runs beside it; then a run not completed is `None`; then green is
/// what [`checks_are_green`] calls green — at least one `success`, nothing
/// but `success`, `skipped` or `neutral`. A sha whose runs were all
/// cancelled is `None` too: it proved nothing either way.
fn branch_checks_from(rows: &[Value]) -> Option<bool> {
    let text = |row: &Value, field: &str| {
        row.get(field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let newest = text(rows.first()?, "headSha");
    let runs: Vec<(String, String)> = rows
        .iter()
        .filter(|row| text(row, "headSha") == newest)
        .map(|row| (text(row, "status"), text(row, "conclusion")))
        .collect();
    if runs.iter().any(|(_, conclusion)| {
        matches!(
            conclusion.as_str(),
            "failure" | "timed_out" | "startup_failure" | "action_required"
        )
    }) {
        return Some(false);
    }
    if runs.iter().any(|(status, _)| status != "completed") {
        return None;
    }
    let green = runs.iter().any(|(_, conclusion)| conclusion == "success")
        && runs.iter().all(|(_, conclusion)| {
            matches!(conclusion.as_str(), "success" | "skipped" | "neutral")
        });
    green.then_some(true)
}

/// What one `gh run list` call answered, read as [`GitHub::branch_checks`]
/// answers: a call that failed is an error, never a quiet `None` — a base
/// whose runs cannot be listed stops the tick, it does not lift the hold.
fn branch_checks_of(ran: &Ran, branch: &str) -> Outcome<Option<bool>> {
    if !ran.ok() {
        return Err(Halt::Failed(format!(
            "the workflow runs of {branch} cannot be listed — {}",
            ran.why()
        )));
    }
    let payload: Value = serde_json::from_str(ran.out())
        .map_err(|e| Halt::Failed(format!("unreadable workflow runs of {branch}: {e}")))?;
    Ok(branch_checks_from(rows_of(
        &payload,
        &format!("the workflow runs of {branch}"),
    )?))
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
    /// The open PRs' CI, mergeability and update time, read in one request
    /// and kept for [`OPEN_PRS_FRESH`]; dropped by any write. A `Mutex` for
    /// the same reason `name` is a `OnceLock`, never held across an await.
    open_prs: Mutex<Option<OpenPrs>>,
    /// Each PR's comments, kept with the update time they were read at:
    /// read again only once the PR moved.
    comments: Mutex<HashMap<u64, (String, String)>>,
    /// The board's issue reads (`issues/…` lists: labelled issues,
    /// sub-issues, blockers), by path and query, kept while
    /// [`GitHub::activity`] says nothing moved: no issue's or pull
    /// request's update time changed, so none of these lists did. `None`
    /// until a first activity read: a lane that never asks it keeps
    /// nothing. Dropped by any write and after [`BOARD_KEPT_AT_MOST`].
    board: Mutex<Option<KeptBoard>>,
}

/// The board's issue reads kept between two ticks of an unchanged board.
struct KeptBoard {
    /// When the board was last seen moving (or first read).
    since: Instant,
    /// By `path?query`: the payload as read.
    reads: HashMap<String, Value>,
}

/// The longest a kept board read answers, however quiet the board: the
/// valve for a change the activity read cannot see (a dependency added by
/// hand).
const BOARD_KEPT_AT_MOST: Duration = Duration::from_secs(600);

/// One read of every open PR: what the watch asked of each, one request at
/// a time, before (`ForgeDesMondes`, 2026-10-20: the token's 5 000 calls an
/// hour spent by 40–50 calls a tick, one CI, one mergeability and one
/// comments read per open PR).
struct OpenPrs {
    /// When it was read.
    at: Instant,
    /// By PR number: its check rows as `gh pr checks` gives them, its
    /// mergeability, its update time.
    prs: HashMap<u64, OpenPr>,
}

/// What the one read says of a PR.
struct OpenPr {
    checks: Vec<Value>,
    mergeable: Value,
    updated_at: String,
    /// The commit those checks ran on: a merge decided on them merges it
    /// and nothing pushed since.
    head: String,
}

/// How long one read of the open PRs answers for: longer than a tick's
/// reads, shorter than anything a check run changes meanwhile.
const OPEN_PRS_FRESH: Duration = Duration::from_secs(45);

/// The fields the one read asks for.
const OPEN_PR_FIELDS: &str = "number,updatedAt,mergeable,headRefOid,statusCheckRollup";

/// A check of `statusCheckRollup` as a row of `gh pr checks --json
/// name,state,link`: a check run's conclusion once completed (`PENDING`
/// before), a status context's state.
fn check_row(rollup: &Value) -> Value {
    let text = |key: &str| rollup.get(key).and_then(Value::as_str).unwrap_or("");
    let (name, state, link) = if rollup.get("context").is_some() {
        (
            text("context"),
            text("state").to_string(),
            text("targetUrl"),
        )
    } else {
        let state = if text("status") == "COMPLETED" {
            text("conclusion").to_uppercase()
        } else {
            "PENDING".to_string()
        };
        (text("name"), state, text("detailsUrl"))
    };
    serde_json::json!({ "name": name, "state": state, "link": link })
}

/// The one read's rows by PR number; `None` when it is no list.
fn open_prs_from(payload: &Value) -> Option<HashMap<u64, OpenPr>> {
    let rows = payload.as_array()?;
    Some(
        rows.iter()
            .filter_map(|row| {
                let number = row.get("number")?.as_u64()?;
                let checks = row
                    .get("statusCheckRollup")
                    .and_then(Value::as_array)
                    .map(|checks| checks.iter().map(check_row).collect())
                    .unwrap_or_default();
                Some((
                    number,
                    OpenPr {
                        checks,
                        mergeable: serde_json::json!({ "mergeable": row.get("mergeable").cloned().unwrap_or(Value::Null) }),
                        updated_at: row
                            .get("updatedAt")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        head: row
                            .get("headRefOid")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    },
                ))
            })
            .collect(),
    )
}

/// A PR reference as its number: `123` or `#123`; `None` for a branch.
fn pr_number(pr_ref: &str) -> Option<u64> {
    pr_ref.trim_start_matches('#').parse().ok()
}

/// Whether `gh` with these arguments changes something on GitHub — what
/// makes a read kept before it stale.
fn writes(args: &[String]) -> bool {
    match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (Some("api"), _) => args.windows(2).any(|w| w[0] == "-X" && w[1] != "GET"),
        (Some("pr"), Some(verb)) => !matches!(verb, "view" | "list" | "checks" | "diff" | "status"),
        (Some("issue"), Some(verb)) => !matches!(verb, "view" | "list" | "status"),
        _ => false,
    }
}

impl GhCli {
    /// The head commit the kept read of the open PRs saw for PR `number`,
    /// without reading anything again.
    fn kept_head(&self, number: u64) -> Option<String> {
        let kept = self.open_prs.lock().ok()?;
        kept.as_ref()?.prs.get(&number).map(|pr| pr.head.clone())
    }

    /// Drops the kept read of the open PRs: something was just written.
    /// What an activity read leaves of the caches: all of them while
    /// nothing moved, none once something did.
    ///
    /// The open PRs' list goes too: another process's write (a decision
    /// posted on a PR by a `decide` lane) moves the board but not this
    /// process's list, whose stale update times kept the PR's old comments
    /// — and #280 was escalated a second time ten seconds after the first
    /// decision, on a repair count that decision had reset
    /// (`ForgeDesMondes`, 2026-10-20 15:13Z).
    fn kept_through(&self, activity: &Activity) {
        let Ok(mut board) = self.board.lock() else {
            return;
        };
        let quiet = matches!(activity, Activity::Unchanged)
            && board
                .as_ref()
                .is_some_and(|b| b.since.elapsed() < BOARD_KEPT_AT_MOST);
        if quiet {
            return;
        }
        *board = Some(KeptBoard {
            since: Instant::now(),
            reads: HashMap::new(),
        });
        drop(board);
        if let Ok(mut kept) = self.open_prs.lock() {
            *kept = None;
        }
    }

    fn forget_open_prs(&self) {
        if let Ok(mut kept) = self.open_prs.lock() {
            *kept = None;
        }
        if let Ok(mut board) = self.board.lock()
            && let Some(board) = board.as_mut()
        {
            board.reads.clear();
        }
    }

    /// A kept board read for `key`, when the board has not moved since.
    fn kept_board_read(&self, key: &str) -> Option<Value> {
        self.board.lock().ok()?.as_ref().and_then(|board| {
            (board.since.elapsed() < BOARD_KEPT_AT_MOST)
                .then(|| board.reads.get(key).cloned())
                .flatten()
        })
    }

    /// Keeps a board read for `key`, when an activity read has been seen.
    fn keep_board_read(&self, key: String, payload: &Value) {
        if let Ok(mut board) = self.board.lock()
            && let Some(board) = board.as_mut()
        {
            board.reads.insert(key, payload.clone());
        }
    }

    /// What the one read of the open PRs says of PR `number`, read again
    /// when older than [`OPEN_PRS_FRESH`]; `None` when the PR is not open,
    /// or the read failed (the caller then asks GitHub as before).
    async fn open_pr<T>(&self, number: u64, take: impl Fn(&OpenPr) -> T) -> Option<T> {
        let fresh = |kept: &Option<OpenPrs>| {
            kept.as_ref()
                .is_some_and(|k| k.at.elapsed() < OPEN_PRS_FRESH)
        };
        let stale = self.open_prs.lock().map_or(true, |k| !fresh(&k));
        if stale {
            let ran = self
                .gh(&[
                    "pr".to_string(),
                    "list".to_string(),
                    "--state".to_string(),
                    "open".to_string(),
                    "--limit".to_string(),
                    "100".to_string(),
                    "--json".to_string(),
                    OPEN_PR_FIELDS.to_string(),
                ])
                .await
                .ok()?;
            if !ran.ok() {
                return None;
            }
            let prs = open_prs_from(&serde_json::from_str(ran.out()).ok()?)?;
            if let Ok(mut kept) = self.open_prs.lock() {
                *kept = Some(OpenPrs {
                    at: Instant::now(),
                    prs,
                });
            }
        }
        let kept = self.open_prs.lock().ok()?;
        kept.as_ref()?.prs.get(&number).map(take)
    }

    /// `gh`, on this repo.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            name: OnceLock::new(),
            open_prs: Mutex::new(None),
            comments: Mutex::new(HashMap::new()),
            board: Mutex::new(None),
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
    pub fn for_slug(slug: &harness_core::domain::Slug) -> Self {
        Self {
            root: PathBuf::from("."),
            name: OnceLock::from(slug.to_string()),
            open_prs: Mutex::new(None),
            comments: Mutex::new(HashMap::new()),
            board: Mutex::new(None),
        }
    }

    async fn gh(&self, args: &[String]) -> Outcome<Ran> {
        // `gh pr …` infers the repo from the cwd, and a slug-only adapter has
        // none that means anything (it would hit the harness's own repo), so
        // name the target explicitly. `api` calls already carry the slug.
        let named;
        let args = if args.first().is_some_and(|a| a == "pr" || a == "run")
            && let Some(name) = self.name.get()
        {
            let mut with_repo = args.to_vec();
            with_repo.push("-R".to_string());
            with_repo.push(name.clone());
            named = with_repo;
            named.as_slice()
        } else {
            args
        };
        // GitHub refuses a burst with a 403 "rate limit exceeded" while the
        // hour's quota is far from spent (its secondary limit): a minute
        // later the same call goes through. One pause, two tries — a round
        // or a tick is not lost on a comment GitHub would take next minute
        // (ForgeDesMondes, 2026-10-20 04:25Z: a code round failed on its
        // own comment, a tick on the board's read).
        if writes(args) {
            self.forget_open_prs();
        }
        let mut ran = process::run(BINARY, args, &self.root).await?;
        if throttled(&ran) {
            tokio::time::sleep(THROTTLE_PAUSE).await;
            ran = process::run(BINARY, args, &self.root).await?;
        }
        Ok(ran)
    }

    /// An API read. `path` is relative to the repo.
    ///
    /// The repo is resolved here rather than by the caller: it is the only way
    /// an unreadable repo name is a failure propagated like any other.
    async fn read(&self, what: &str, path: &str, query: &[(&str, String)]) -> Outcome<Value> {
        // An issue read is kept while the board does not move: every issue
        // or pull request changed bumps the update time the activity read
        // watches (see `board`).
        let key = path.starts_with("issues").then(|| {
            query
                .iter()
                .fold(path.to_string(), |key, (k, v)| key + "&" + k + "=" + v)
        });
        if let Some(key) = &key
            && let Some(kept) = self.kept_board_read(key)
        {
            return Ok(kept);
        }
        let value = self.read_fresh(what, path, query).await?;
        if let Some(key) = key {
            self.keep_board_read(key, &value);
        }
        Ok(value)
    }

    /// [`Self::read`] without the kept board.
    async fn read_fresh(&self, what: &str, path: &str, query: &[(&str, String)]) -> Outcome<Value> {
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
        let mut ran = self.gh(&args).await?;
        // A lost answer to a write that is safe to repeat: repeated once.
        if repeats(&ran, method) {
            ran = self.gh(&args).await?;
        }
        if ran.ok() {
            return Ok(());
        }
        Err(write_refused(what, method, &url, &ran))
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
                refusal(&ran.why(), ran.out())
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
            refusal(&ran.why(), ran.out())
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

/// Standard base64 with padding, as the contents API wants a file: a few
/// lines rather than a dependency for one call.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                let index = usize::try_from((word >> (18 - 6 * i)) & 0x3f).unwrap_or(0);
                out.push(char::from(TABLE[index]));
            } else {
                out.push('=');
            }
        }
    }
    out
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

    async fn activity(&self, etag: Option<&str>) -> Outcome<Activity> {
        let repo = self.repo().await?;
        // `-i` puts the status line and the headers before the body: the
        // status is what says `304`, and the `Etag` header is what the next
        // read hands back. One page of one item: the newest is all we ask.
        let mut args = vec![
            "api".to_string(),
            "-i".to_string(),
            format!("repos/{repo}/issues"),
            "-X".to_string(),
            "GET".to_string(),
        ];
        for (key, value) in [
            ("state", "all"),
            ("sort", "updated"),
            ("direction", "desc"),
            ("per_page", "1"),
        ] {
            args.push("-f".to_string());
            args.push(format!("{key}={value}"));
        }
        if let Some(etag) = etag {
            args.push("-H".to_string());
            args.push(format!("If-None-Match: {etag}"));
        }
        let activity = activity_from(&self.gh(&args).await?)?;
        self.kept_through(&activity);
        Ok(activity)
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
        // Removing a label the issue does not carry is nothing to do, not a
        // failure: GitHub answers 404 "Label does not exist", and a decide
        // that ended on it was paid again at every tick (ForgeDesMondes
        // #213, 2026-10-20 06:22Z; #214 the day before).
        let repo = self.repo().await?;
        let args = vec![
            "api".to_string(),
            "-X".to_string(),
            "DELETE".to_string(),
            format!("repos/{repo}/issues/{number}/labels/{label}"),
        ];
        let ran = self.gh(&args).await?;
        if ran.ok() || label_absent(&ran) {
            return Ok(());
        }
        Err(Halt::Failed(format!(
            "GitHub refused to remove {label} from #{number} (DELETE repos/{repo}/issues/{number}/labels/{label}): {}",
            ran.why()
        )))
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
        let posted = self
            .write(
                &format!("comment on #{number}"),
                "POST",
                &format!("issues/{number}/comments"),
                &[("body", body.to_string())],
            )
            .await;
        // A POST is not repeated — a second comment is a second answer — but
        // a lost answer is checked: the comment is often there.
        confirmed(posted, body, || self.issue_comments(number)).await
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

    async fn pr_files(&self, pr_ref: &str) -> Outcome<Vec<String>> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "view".to_string(),
                pr_ref.to_string(),
                "--json".to_string(),
                "files".to_string(),
                "--jq".to_string(),
                ".files[].path".to_string(),
            ])
            .await?;
        if !ran.ok() {
            return Err(Halt::Failed(format!(
                "cannot read the files of PR {pr_ref} — {}",
                ran.why()
            )));
        }
        Ok(ran.lines())
    }

    async fn pr_comments(&self, num: &str) -> Outcome<String> {
        let number = pr_number(num);
        let updated = if let Some(n) = number {
            self.open_pr(n, |pr| pr.updated_at.clone()).await
        } else {
            None
        };
        if let (Some(n), Some(updated)) = (number, updated.as_ref())
            && let Ok(kept) = self.comments.lock()
            && let Some((at, text)) = kept.get(&n)
            && at == updated
        {
            return Ok(text.clone());
        }
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
        if let (Some(n), Some(updated)) = (number, updated)
            && let Ok(mut kept) = self.comments.lock()
        {
            kept.insert(n, (updated, ran.stdout.clone()));
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

    async fn branches(&self, prefix: &str) -> Outcome<Vec<String>> {
        let what = format!("branches named {prefix}*");
        let value = self
            .read(
                &what,
                &format!("git/matching-refs/heads/{prefix}"),
                &[("per_page", "100".to_string())],
            )
            .await?;
        let refs = value
            .as_array()
            .ok_or_else(|| unreadable(&what, "not a list of refs"))?;
        Ok(refs
            .iter()
            .filter_map(|r| r.get("ref").and_then(Value::as_str))
            .filter_map(|r| r.strip_prefix("refs/heads/"))
            .map(ToString::to_string)
            .collect())
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

    async fn merge_branch(&self, base: &str, head: &str) -> Outcome<()> {
        self.write(
            &format!("merge {head} into {base}"),
            "POST",
            "merges",
            &[
                ("base", base.to_string()),
                ("head", head.to_string()),
                ("commit_message", format!("Merge {head} into {base}")),
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

    async fn require_squash_merges(&self) -> Outcome<()> {
        // `-F` types `true`/`false` as booleans, which the string-only
        // `write` cannot send.
        let repo = self.repo().await?;
        let url = format!("repos/{repo}");
        let mut args = vec![
            "api".to_string(),
            "-X".to_string(),
            "PATCH".to_string(),
            url.clone(),
        ];
        for field in [
            "allow_squash_merge=true",
            "allow_merge_commit=false",
            "allow_rebase_merge=false",
            "squash_merge_commit_title=PR_TITLE",
            "squash_merge_commit_message=PR_BODY",
        ] {
            args.push("-F".to_string());
            args.push(field.to_string());
        }
        let ran = self.gh(&args).await?;
        if ran.ok() {
            return Ok(());
        }
        Err(Halt::Halted(format!(
            "GitHub refused to require squash merges on {repo} (PATCH {url}): {}",
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

    async fn put_file(
        &self,
        path: &str,
        branch: &str,
        content: &str,
        message: &str,
    ) -> Outcome<()> {
        // The contents API replaces a file only against its current blob:
        // read the sha first, absent when the file is new.
        let what = format!("{path}@{branch}");
        let repo = self.repo().await?;
        let read = vec![
            "api".to_string(),
            format!("repos/{repo}/contents/{path}"),
            "-X".to_string(),
            "GET".to_string(),
            "-f".to_string(),
            format!("ref={branch}"),
        ];
        let sha = match self.read_optional(&what, &read).await? {
            Some(ran) => serde_json::from_str::<Value>(&ran.stdout)
                .ok()
                .and_then(|v| {
                    v.get("sha")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                }),
            None => None,
        };
        let mut fields = vec![
            ("message", message.to_string()),
            ("content", base64(content.as_bytes())),
            ("branch", branch.to_string()),
        ];
        if let Some(sha) = sha {
            fields.push(("sha", sha));
        }
        self.write(
            &format!("write {path} on {branch}"),
            "PUT",
            &format!("contents/{path}"),
            &fields,
        )
        .await
    }

    async fn create_issue(&self, title: &str, body: &str, labels: &[&str]) -> Outcome<u64> {
        let length = body.chars().count();
        if length > ISSUE_BODY_MAX {
            return Err(Halt::Halted(format!(
                "the issue `{title}` was not sent: its body is {length} characters, GitHub \
                 takes at most {ISSUE_BODY_MAX}"
            )));
        }
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
        // Pinned to the commit whose checks the decision read: a push since
        // makes GitHub refuse, rather than merge what no check has judged.
        let head = pr_number(pr_ref).and_then(|n| self.kept_head(n));
        let mut args = vec![
            "pr".to_string(),
            "merge".to_string(),
            pr_ref.to_string(),
            "--squash".to_string(),
        ];
        if let Some(head) = head.filter(|h| !h.is_empty()) {
            args.push("--match-head-commit".to_string());
            args.push(head);
        }
        let ran = self.gh(&args).await?;
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
        if let Some(number) = pr_number(pr_ref)
            && let Some(green) = self
                .open_pr(number, |pr| checks_are_green(&pr.checks))
                .await
        {
            return Ok(green);
        }
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
        Ok(checks_are_green(&ci_rows(pr_ref, &ran)?))
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

    async fn pr_mergeable(&self, pr_ref: &str) -> Outcome<Option<bool>> {
        // GitHub computes mergeability lazily: an UNKNOWN in the list is
        // asked again of the PR itself, which starts the computation.
        if let Some(number) = pr_number(pr_ref)
            && let Some(Some(known)) = self
                .open_pr(number, |pr| mergeable_from(&pr.mergeable))
                .await
        {
            return Ok(Some(known));
        }
        let ran = self
            .gh(&[
                "pr".to_string(),
                "view".to_string(),
                pr_ref.to_string(),
                "--json".to_string(),
                "mergeable".to_string(),
            ])
            .await?;
        if !ran.ok() {
            return Err(Halt::Failed(format!(
                "cannot read whether PR {pr_ref} merges — {}",
                ran.why()
            )));
        }
        let payload: Value = serde_json::from_str(ran.out())
            .map_err(|e| Halt::Failed(format!("unreadable mergeability of PR {pr_ref}: {e}")))?;
        Ok(mergeable_from(&payload))
    }

    async fn branch_checks(&self, branch: &str) -> Outcome<Option<bool>> {
        // Twenty runs: enough to hold every workflow of the newest push and
        // the ones of the push before, which `branch_checks_from` drops.
        let ran = self
            .gh(&[
                "run".to_string(),
                "list".to_string(),
                "--branch".to_string(),
                branch.to_string(),
                "--limit".to_string(),
                "20".to_string(),
                "--json".to_string(),
                "headSha,status,conclusion,name".to_string(),
            ])
            .await?;
        branch_checks_of(&ran, branch)
    }

    async fn pr_failing_checks(&self, pr_ref: &str) -> Outcome<Vec<String>> {
        if let Some(number) = pr_number(pr_ref)
            && let Some(failing) = self.open_pr(number, |pr| failing_checks(&pr.checks)).await
        {
            return Ok(failing);
        }
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
        Ok(failing_checks(&ci_rows(pr_ref, &ran)?))
    }

    async fn artifact_file(
        &self,
        workflow: &str,
        branch: &str,
        artifact: &str,
        file: &str,
    ) -> Outcome<Option<Kept>> {
        let listed = self
            .gh(&[
                "run".to_string(),
                "list".to_string(),
                "--workflow".to_string(),
                workflow.to_string(),
                "--branch".to_string(),
                branch.to_string(),
                "--status".to_string(),
                "completed".to_string(),
                "--limit".to_string(),
                "1".to_string(),
                "--json".to_string(),
                "databaseId,url,updatedAt".to_string(),
            ])
            .await?;
        if !listed.ok() {
            return Err(Halt::Failed(format!(
                "the runs of {workflow} on {branch} cannot be listed — {}",
                listed.why()
            )));
        }
        let Some((id, run_url, at)) = latest_run(listed.out()) else {
            return Ok(None);
        };
        // `gh run download` writes the artifact into a folder: one of its
        // own, read, then removed.
        let dir = std::env::temp_dir().join(format!(
            "harness-artifact-{}-{id}-{artifact}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let downloaded = self
            .gh(&[
                "run".to_string(),
                "download".to_string(),
                id.to_string(),
                "--name".to_string(),
                artifact.to_string(),
                "--dir".to_string(),
                dir.to_string_lossy().to_string(),
            ])
            .await?;
        let text = downloaded
            .ok()
            .then(|| std::fs::read_to_string(dir.join(file)).ok())
            .flatten();
        let _ = std::fs::remove_dir_all(&dir);
        Ok(text.map(|text| Kept { text, run_url, at }))
    }
}

/// The latest run `gh run list --json databaseId,url,updatedAt` names: its
/// id, its page, when it last moved.
fn latest_run(listed: &str) -> Option<(u64, String, String)> {
    let rows: Vec<Value> = serde_json::from_str(listed).ok()?;
    let row = rows.first()?;
    Some((
        row.get("databaseId")?.as_u64()?,
        row.get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        row.get("updatedAt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    ))
}

/// Reads `gh api -i` output as an [`Activity`].
///
/// `gh` exits non-zero on a `304`, as it does on any status outside `2xx`,
/// so the status line is read before the exit code: a `304` is the answer
/// we asked for, anything else non-zero is a failed read.
fn activity_from(ran: &Ran) -> Outcome<Activity> {
    const WHAT: &str = "the board's activity";
    let status = ran
        .stdout
        .lines()
        .chain(ran.stderr.lines())
        .find_map(|line| {
            let rest = line.strip_prefix("HTTP/")?;
            rest.split_whitespace().nth(1)
        });
    if status == Some("304") {
        return Ok(Activity::Unchanged);
    }
    if !ran.ok() {
        return Err(unreadable(WHAT, &ran.why()));
    }
    // Headers, a blank line, then the body.
    let (headers, body) = ran
        .stdout
        .split_once("\n\n")
        .or_else(|| ran.stdout.split_once("\r\n\r\n"))
        .unwrap_or((ran.stdout.as_str(), ""));
    let etag = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("etag")
            .then(|| value.trim().to_string())
    });
    let moved_at = if body.trim().is_empty() {
        None
    } else {
        let payload: Value =
            serde_json::from_str(body.trim()).map_err(|e| unreadable(WHAT, &e.to_string()))?;
        payload
            .get(0)
            .and_then(|first| first.get("updated_at"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
    };
    Ok(Activity::Changed { etag, moved_at })
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_label_the_issue_does_not_carry_is_removed_already() {
        let refused = Ran {
            code: Some(1),
            stdout: String::new(),
            stderr: "gh: Label does not exist (HTTP 404)".to_string(),
        };
        assert!(label_absent(&refused));
        let other = Ran {
            code: Some(1),
            stdout: String::new(),
            stderr: "gh: Not Found (HTTP 404)".to_string(),
        };
        assert!(!label_absent(&other), "another 404 is still a refusal");
        let done = Ran {
            code: Some(0),
            stdout: String::new(),
            stderr: "Label does not exist".to_string(),
        };
        assert!(
            !label_absent(&done),
            "a call that went through needs no excuse"
        );
    }

    #[test]
    fn one_read_of_the_open_prs_answers_as_gh_pr_checks_and_view_would() {
        let payload = json!([
            {"number": 268, "updatedAt": "2026-10-20T01:00:00Z", "mergeable": "MERGEABLE", "headRefOid": "abc123",
             "statusCheckRollup": [
                {"__typename": "CheckRun", "name": "rust", "status": "COMPLETED", "conclusion": "SUCCESS", "detailsUrl": "u1"},
                {"__typename": "CheckRun", "name": "gates", "status": "COMPLETED", "conclusion": "SKIPPED", "detailsUrl": "u2"}]},
            {"number": 279, "updatedAt": "2026-10-20T02:00:00Z", "mergeable": "CONFLICTING",
             "statusCheckRollup": [
                {"__typename": "CheckRun", "name": "rust", "status": "COMPLETED", "conclusion": "FAILURE", "detailsUrl": "u3"},
                {"__typename": "StatusContext", "context": "deploy", "state": "PENDING", "targetUrl": "u4"}]},
            {"number": 280, "updatedAt": "2026-10-20T03:00:00Z", "mergeable": "UNKNOWN",
             "statusCheckRollup": [
                {"__typename": "CheckRun", "name": "rust", "status": "IN_PROGRESS", "conclusion": "", "detailsUrl": "u5"}]},
            {"number": 281, "updatedAt": "2026-10-20T04:00:00Z", "mergeable": "MERGEABLE", "statusCheckRollup": null}
        ]);
        let prs = open_prs_from(&payload).expect("a list");
        assert!(checks_are_green(&prs[&268].checks));
        assert_eq!(mergeable_from(&prs[&268].mergeable), Some(true));
        assert!(!checks_are_green(&prs[&279].checks));
        assert_eq!(failing_checks(&prs[&279].checks), ["rust — FAILURE — u3"]);
        assert_eq!(mergeable_from(&prs[&279].mergeable), Some(false));
        assert!(
            !checks_are_green(&prs[&280].checks),
            "a run in progress is pending"
        );
        assert!(failing_checks(&prs[&280].checks).is_empty());
        assert_eq!(
            mergeable_from(&prs[&280].mergeable),
            None,
            "UNKNOWN is asked again"
        );
        assert!(
            !checks_are_green(&prs[&281].checks),
            "no checks is not green"
        );
        assert_eq!(prs[&281].updated_at, "2026-10-20T04:00:00Z");
        assert_eq!(prs[&268].head, "abc123");
        assert_eq!(prs[&279].head, "", "no head read, no pin");
        assert!(open_prs_from(&json!({"message": "x"})).is_none());
    }

    #[test]
    fn a_moved_board_drops_the_open_prs_and_a_quiet_one_keeps_them() {
        let gh = GhCli::new(Path::new("."));
        let kept = || OpenPrs {
            at: Instant::now(),
            prs: HashMap::new(),
        };
        let moved = Activity::Changed {
            etag: None,
            moved_at: None,
        };
        *gh.open_prs.lock().expect("lock") = Some(kept());
        gh.kept_through(&moved);
        assert!(
            gh.open_prs.lock().expect("lock").is_none(),
            "moved: re-read"
        );
        assert!(
            gh.board.lock().expect("lock").is_some(),
            "a board is kept from now"
        );
        *gh.open_prs.lock().expect("lock") = Some(kept());
        gh.keep_board_read("issues/4/sub_issues".to_string(), &json!([1]));
        gh.kept_through(&Activity::Unchanged);
        assert!(gh.open_prs.lock().expect("lock").is_some(), "quiet: kept");
        assert_eq!(gh.kept_board_read("issues/4/sub_issues"), Some(json!([1])));
        gh.kept_through(&moved);
        assert!(gh.kept_board_read("issues/4/sub_issues").is_none());
    }

    #[test]
    fn a_board_read_is_kept_only_after_an_activity_read_and_until_a_write() {
        let gh = GhCli::new(Path::new("."));
        gh.keep_board_read("issues/4/sub_issues".to_string(), &json!([1]));
        assert!(
            gh.kept_board_read("issues/4/sub_issues").is_none(),
            "no activity read yet: a lane keeps nothing"
        );
        *gh.board.lock().expect("lock") = Some(KeptBoard {
            since: Instant::now(),
            reads: HashMap::new(),
        });
        gh.keep_board_read("issues/4/sub_issues".to_string(), &json!([1]));
        assert_eq!(gh.kept_board_read("issues/4/sub_issues"), Some(json!([1])));
        gh.forget_open_prs();
        assert!(
            gh.kept_board_read("issues/4/sub_issues").is_none(),
            "a write drops it"
        );
        *gh.board.lock().expect("lock") = Some(KeptBoard {
            since: Instant::now()
                .checked_sub(BOARD_KEPT_AT_MOST + Duration::from_secs(1))
                .expect("an instant"),
            reads: HashMap::from([("issues/4/sub_issues".to_string(), json!([1]))]),
        });
        assert!(
            gh.kept_board_read("issues/4/sub_issues").is_none(),
            "past the valve"
        );
    }

    #[test]
    fn a_write_is_told_from_a_read() {
        let a = |v: &[&str]| v.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert!(writes(&a(&[
            "api",
            "-X",
            "POST",
            "repos/x/issues/1/comments"
        ])));
        assert!(writes(&a(&[
            "api",
            "-X",
            "DELETE",
            "repos/x/issues/1/labels/l"
        ])));
        assert!(!writes(&a(&["api", "repos/x/issues/1", "-X", "GET"])));
        assert!(!writes(&a(&["api", "rate_limit"])));
        assert!(writes(&a(&["pr", "merge", "12", "--squash"])));
        assert!(writes(&a(&["pr", "create", "--title", "t"])));
        assert!(!writes(&a(&["pr", "checks", "12"])));
        assert!(!writes(&a(&["pr", "list", "--state", "open"])));
        assert!(writes(&a(&["issue", "edit", "12"])));
        assert!(!writes(&a(&["run", "list"])));
        assert_eq!(pr_number("#279"), Some(279));
        assert_eq!(pr_number("279"), Some(279));
        assert_eq!(pr_number("feat/x"), None);
    }

    #[test]
    fn a_burst_github_refused_is_throttled_and_nothing_else_is() {
        let refused = |stderr: &str| Ran {
            code: Some(1),
            stdout: String::new(),
            stderr: stderr.to_string(),
        };
        assert!(throttled(&refused(
            "gh: API rate limit exceeded for user ID 97902739. If you reach out to GitHub Support"
        )));
        assert!(throttled(&refused(
            "You have exceeded a secondary rate limit"
        )));
        assert!(!throttled(&refused("gh: Not Found (HTTP 404)")));
        assert!(!throttled(&Ran {
            code: Some(0),
            stdout: "rate limit exceeded is what the body says".to_string(),
            stderr: String::new(),
        }));
        assert!(
            !throttled(&Ran {
                code: Some(0),
                stdout: String::new(),
                stderr: "rate limit exceeded".to_string(),
            }),
            "a call that went through is not throttled, whatever it printed"
        );
    }
    #[test]
    fn a_lost_answer_is_told_apart_and_only_safe_writes_repeat() {
        let ran = |code: i32, stderr: &str| Ran {
            code: Some(code),
            stdout: String::new(),
            stderr: stderr.to_string(),
        };
        assert!(answer_lost(&ran(1, "unexpected end of JSON input")));
        assert!(!answer_lost(&ran(0, "unexpected end of JSON input")));
        assert!(!answer_lost(&ran(1, "gh: Not Found (HTTP 404)")));
        assert!(idempotent("PATCH") && idempotent("PUT") && idempotent("DELETE"));
        assert!(!idempotent("POST"));
        assert!(repeats(&ran(1, ANSWER_LOST), "PATCH"));
        assert!(!repeats(&ran(1, ANSWER_LOST), "POST"));
        assert!(!repeats(&ran(1, "gh: Not Found (HTTP 404)"), "PATCH"));
    }

    #[tokio::test]
    async fn a_lost_comment_answer_stands_only_when_the_comment_is_there() {
        let lost = || {
            Err(Halt::Halted(format!(
                "GitHub refused to comment: {ANSWER_LOST}"
            )))
        };
        let there = || async { Ok(vec!["old".to_string(), "the answer".to_string()]) };
        let absent = || async { Ok(vec!["old".to_string()]) };
        assert!(confirmed(lost(), "the answer", there).await.is_ok());
        assert!(confirmed(lost(), "the answer", absent).await.is_err());
        let refused = Err(Halt::Halted("gh: Not Found (HTTP 404)".to_string()));
        assert!(
            confirmed(refused, "the answer", there).await.is_err(),
            "a refusal is not checked against the thread"
        );
        assert!(confirmed(Ok(()), "the answer", absent).await.is_ok());
    }

    #[test]
    fn a_comment_landed_when_it_is_the_issue_s_latest() {
        let bodies = |all: &[&str]| all.iter().map(|b| (*b).to_string()).collect::<Vec<_>>();
        assert!(comment_landed(
            &bodies(&["old", "the answer\n"]),
            "the answer"
        ));
        assert!(!comment_landed(
            &bodies(&["the answer", "newer"]),
            "the answer"
        ));
        assert!(!comment_landed(&[], "the answer"));
    }

    #[test]
    fn a_refusal_says_github_s_own_reasons_when_the_response_has_them() {
        let why = "gh: Validation Failed (HTTP 422)";
        let long = r#"{"message": "Validation Failed", "errors": [{"resource": "Issue", "code": "custom", "field": "body", "message": "body is too long (maximum is 65536 characters)"}]}"#;
        assert_eq!(
            super::refusal(why, long),
            "gh: Validation Failed (HTTP 422) — body is too long (maximum is 65536 characters)"
        );
        let coded = r#"{"errors": [{"field": "title", "code": "missing_field"}, "a plain one"]}"#;
        assert_eq!(
            super::refusal(why, coded),
            "gh: Validation Failed (HTTP 422) — title: missing_field; a plain one"
        );
        assert_eq!(super::refusal(why, ""), why, "no body, gh's line");
        assert_eq!(super::refusal(why, r#"{"message": "x"}"#), why);
    }

    #[test]
    fn the_contents_api_s_base64_is_the_standard_one() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(super::base64(b"{\"a\": 1}\n"), "eyJhIjogMX0K");
    }

    #[tokio::test]
    async fn the_files_of_a_pr_github_cannot_read_are_an_error_not_an_empty_list() {
        use harness_core::ports::shell::github::GitHub as _;
        // A folder that is there and is no repository: `gh` runs, and fails.
        let gh = super::GhCli::new(&std::env::temp_dir());
        assert!(gh.pr_files("1").await.is_err());
    }

    #[tokio::test]
    async fn a_branch_merge_github_cannot_make_is_an_error_not_a_success() {
        use harness_core::ports::shell::github::GitHub as _;
        let gh = super::GhCli::new(std::path::Path::new("/nonexistent"));
        assert!(
            gh.merge_branch("milestone/1-x", "main_agent")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_issue_body_over_github_s_limit_is_refused_before_it_is_sent() {
        use harness_core::ports::shell::github::GitHub as _;
        // No repository there: anything sent would fail otherwise.
        let gh = super::GhCli::new(std::path::Path::new("/nonexistent"));
        let body = "é".repeat(super::ISSUE_BODY_MAX + 1);
        let Err(super::Halt::Halted(why)) = gh.create_issue("Roadmap", &body, &[]).await else {
            panic!("refused as halted");
        };
        assert!(why.contains("65537 characters"), "{why}");
        assert!(why.contains("at most 65536"), "{why}");
        // At the limit it goes on to GitHub (and fails there, no repository).
        let body = "é".repeat(super::ISSUE_BODY_MAX);
        let sent = gh.create_issue("Roadmap", &body, &[]).await;
        assert!(
            !matches!(&sent, Err(super::Halt::Halted(why)) if why.contains("was not sent")),
            "{sent:?}"
        );
    }

    #[test]
    fn the_latest_run_is_read_from_the_list_and_none_when_it_is_empty() {
        let listed = r#"[{"databaseId": 42, "url": "https://github.com/o/r/actions/runs/42", "updatedAt": "2026-10-14T10:00:00Z"}]"#;
        assert_eq!(
            super::latest_run(listed),
            Some((
                42,
                "https://github.com/o/r/actions/runs/42".to_string(),
                "2026-10-14T10:00:00Z".to_string()
            ))
        );
        assert_eq!(super::latest_run("[]"), None);
        assert_eq!(super::latest_run("not json"), None);
        assert_eq!(
            super::latest_run(r#"[{"url": "x"}]"#),
            None,
            "no id, no run"
        );
    }

    #[test]
    fn a_304_is_unchanged_even_though_gh_exits_one() {
        let ran = super::Ran {
            code: Some(1),
            stdout: "HTTP/2.0 304 Not Modified\nEtag: W/\"abc\"\n\n".to_string(),
            stderr: "gh: HTTP 304".to_string(),
        };
        assert_eq!(
            super::activity_from(&ran).expect("read"),
            super::Activity::Unchanged
        );
    }

    #[test]
    fn a_200_carries_the_tag_and_the_newest_stamp() {
        let ran = super::Ran {
            code: Some(0),
            stdout: "HTTP/2.0 200 OK\nX-Ratelimit-Remaining: 4919\nEtag: W/\"abc\"\n\n[{\"number\": 14, \"updated_at\": \"2026-10-08T20:52:00Z\"}]".to_string(),
            stderr: String::new(),
        };
        assert_eq!(
            super::activity_from(&ran).expect("read"),
            super::Activity::Changed {
                etag: Some("W/\"abc\"".to_string()),
                moved_at: Some("2026-10-08T20:52:00Z".to_string()),
            }
        );
    }

    #[test]
    fn an_empty_repository_has_moved_nowhere_but_is_not_unchanged() {
        let ran = super::Ran {
            code: Some(0),
            stdout: "HTTP/2.0 200 OK\n\n[]".to_string(),
            stderr: String::new(),
        };
        assert_eq!(
            super::activity_from(&ran).expect("read"),
            super::Activity::Changed {
                etag: None,
                moved_at: None,
            }
        );
    }

    #[test]
    fn a_failed_read_is_never_quiet() {
        let ran = super::Ran {
            code: Some(1),
            stdout: String::new(),
            stderr: "gh: HTTP 401 Bad credentials".to_string(),
        };
        assert!(matches!(
            super::activity_from(&ran),
            Err(super::Halt::Unreadable(_))
        ));
    }

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

    // --- mergeable_from --------------------------------------------------------

    #[test]
    fn mergeability_is_read_as_github_spells_it() {
        assert_eq!(
            mergeable_from(&json!({"mergeable": "MERGEABLE"})),
            Some(true)
        );
        assert_eq!(
            mergeable_from(&json!({"mergeable": "CONFLICTING"})),
            Some(false)
        );
        assert_eq!(mergeable_from(&json!({"mergeable": "UNKNOWN"})), None);
        assert_eq!(mergeable_from(&json!({})), None);
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
    fn no_check_reported_reads_as_no_check_and_any_other_silence_as_a_failed_read() {
        let silent = |stderr: &str| Ran {
            code: Some(1),
            stdout: String::new(),
            stderr: stderr.to_string(),
        };
        let none = super::ci_rows("251", &silent("no checks reported on the 'feat/x' branch"))
            .expect("no check is a fact");
        assert!(none.is_empty());
        assert!(!super::checks_are_green(&none), "nothing green yet");
        assert!(
            super::failing_checks(&none).is_empty(),
            "nothing failing either"
        );
        let err = super::ci_rows("251", &silent("could not resolve to a PullRequest"))
            .expect_err("a read that failed");
        assert!(
            err.reason().contains("could not read CI status for 251"),
            "{}",
            err.reason()
        );
        let rows = super::ci_rows(
            "251",
            &Ran {
                code: Some(1),
                stdout: r#"[{"state":"FAILURE","name":"rust"}]"#.to_string(),
                stderr: String::new(),
            },
        )
        .expect("rows");
        assert_eq!(rows.len(), 1);
        assert!(super::ci_rows("251", &silent("")).is_err());
    }

    #[test]
    fn no_checks_at_all_is_not_green() {
        // Proof CI ran is the point, not the absence of a reason to refuse.
        assert!(!checks_are_green(&[]));
    }

    // --- branch_checks_from --------------------------------------------------

    fn run(sha: &str, status: &str, conclusion: &str) -> Value {
        json!({"headSha": sha, "status": status, "conclusion": conclusion, "name": "ci"})
    }

    #[test]
    fn a_branch_whose_newest_runs_all_succeeded_is_green() {
        let rows = vec![
            run("b2", "completed", "success"),
            run("b2", "completed", "skipped"),
            run("a1", "completed", "failure"),
        ];
        assert_eq!(
            branch_checks_from(&rows),
            Some(true),
            "the older sha's red is gone"
        );
    }

    #[test]
    fn one_failed_run_on_the_newest_sha_is_red_even_while_another_runs() {
        let rows = vec![
            run("b2", "in_progress", ""),
            run("b2", "completed", "failure"),
            run("a1", "completed", "success"),
        ];
        assert_eq!(branch_checks_from(&rows), Some(false));
        assert_eq!(
            branch_checks_from(&[run("b2", "completed", "timed_out")]),
            Some(false)
        );
    }

    #[test]
    fn a_run_still_going_or_none_at_all_says_nothing_yet() {
        assert_eq!(branch_checks_from(&[]), None);
        let rows = vec![run("b2", "queued", ""), run("b2", "completed", "success")];
        assert_eq!(branch_checks_from(&rows), None);
        // Cancelled runs proved nothing; a skipped run alone proved nothing.
        assert_eq!(
            branch_checks_from(&[run("b2", "completed", "cancelled")]),
            None
        );
        assert_eq!(
            branch_checks_from(&[run("b2", "completed", "skipped")]),
            None
        );
    }

    #[test]
    fn a_run_list_gh_could_not_make_is_an_error_and_one_it_made_is_read() {
        let refused = Ran {
            code: Some(1),
            stdout: String::new(),
            stderr: "gh: HTTP 401: Bad credentials".to_string(),
        };
        let Err(Halt::Failed(why)) = branch_checks_of(&refused, "main_agent") else {
            panic!("a listing gh refused is an error, not a quiet None");
        };
        assert!(
            why.contains("main_agent") && why.contains("HTTP 401"),
            "{why}"
        );
        let listed = |rows: &str| Ran {
            code: Some(0),
            stdout: rows.to_string(),
            stderr: String::new(),
        };
        assert_eq!(
            branch_checks_of(
                &listed(
                    r#"[{"headSha":"b2","status":"completed","conclusion":"failure","name":"ci"}]"#
                ),
                "main_agent"
            )
            .expect("read"),
            Some(false)
        );
        assert_eq!(
            branch_checks_of(
                &listed(
                    r#"[{"headSha":"b2","status":"completed","conclusion":"success","name":"ci"}]"#
                ),
                "main_agent"
            )
            .expect("read"),
            Some(true)
        );
        assert_eq!(
            branch_checks_of(&listed("[]"), "main_agent").expect("read"),
            None
        );
        assert_eq!(
            branch_checks_of(&listed("null"), "main_agent").expect("read"),
            None
        );
        assert!(branch_checks_of(&listed("not json"), "main_agent").is_err());
        assert!(branch_checks_of(&listed(r#"{"runs": []}"#), "main_agent").is_err());
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
