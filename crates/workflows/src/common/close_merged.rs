//! A task whose pull request was merged into its milestone closes — however
//! the PR was merged.
//!
//! The merge train closes the task it merges (`milestone_merge::run`). A PR
//! merged outside it — by a human, by an older session's own `gh pr merge`
//! — left its task open, and every task waiting on it waited forever
//! (`ForgeDesMondes`' #463 and #464, merged by their sessions, held #465;
//! #165, #207, #362 before them). The router reads each building
//! milestone's merged PRs once a full read and routes a free
//! [`Route::CloseMerged`](crate::common::routing::Route::CloseMerged) per
//! task found ([`due`]); [`close`] is the tail both paths share.
//!
//! **The proof is the PR's `Closes #N` line** (`delivery::closes`), or a
//! title naming `#N` **confirmed by the PR's head being the task's declared
//! branch** — one `pr` read, only for an open task with no closing PR. A
//! title alone proves nothing: `fix: unblock #465 (after #463)` names two
//! tasks it does not deliver. A PR proven by its head gets its `Closes`
//! line written before the task closes, so the milestone's landing reads
//! the same proof (`delivery::every_task_merged`).

use harness_core::domain::{Issue, Outcome};
use harness_core::ports::shell::github::GitHub;

use crate::common::{delivery, labels};
use crate::dev_loop::data::tasks;

/// A task to close, the merged PR that delivered it, and the branch it was
/// merged into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    /// The task's number.
    pub task: u64,
    /// The merged PR's number.
    pub pr: u64,
    /// The milestone branch it was merged into.
    pub branch: String,
}

/// Whether a task may be closed by the sweep: open, an agent's (a person's
/// task ships no PR), and no lane on it — a session may still be on the task
/// when a human merges its PR.
fn sweepable(task: &Issue, running: &[u64]) -> bool {
    task.is_open() && task.has(labels::AGENT) && !running.contains(&task.number)
}

/// The tasks of `board` (a milestone's sub-issues) to close, each with the
/// merged PR of `merged` (the PRs merged into `branch`) that delivered it.
///
/// A PR declaring `Closes #N` proves it; a PR whose title names `#N` proves
/// it once its head reads as the task's declared branch. A task whose
/// comments already carry the merged note for that PR is left — told once,
/// and a task a human reopened is not closed again every tick.
///
/// # Errors
/// [`harness_core::domain::Halt`] from a PR or a comments read: a routing
/// read that fails fails the tick.
pub async fn due(
    gh: &dyn GitHub,
    board: &[Issue],
    merged: &[Issue],
    branch: &str,
    running: &[u64],
) -> Outcome<Vec<Delivered>> {
    let mut found = Vec::new();
    for task in board.iter().filter(|task| sweepable(task, running)) {
        let Some(pr) = delivering(gh, task, merged).await? else {
            continue;
        };
        let note = delivery::merged_note(branch, &format!("#{pr}"));
        if gh
            .issue_comments(task.number)
            .await?
            .iter()
            .any(|comment| comment.contains(&note))
        {
            continue;
        }
        found.push(Delivered {
            task: task.number,
            pr,
            branch: branch.to_string(),
        });
    }
    Ok(found)
}

/// The merged PR that delivered `task`: its `Closes` line, else a title
/// naming it whose head is the task's declared branch.
async fn delivering(gh: &dyn GitHub, task: &Issue, merged: &[Issue]) -> Outcome<Option<u64>> {
    if let Some(pr) = delivery::first_closing(merged, task.number) {
        return Ok(Some(pr.number));
    }
    let Some(branch) = tasks::declared_branch(&task.body) else {
        return Ok(None);
    };
    for named in merged
        .iter()
        .filter(|pr| delivery::title_names(&pr.title, task.number))
    {
        if gh.pr(&named.number.to_string()).await?.head == branch {
            return Ok(Some(named.number));
        }
    }
    Ok(None)
}

/// Closes `task`, delivered by PR `pr_ref` merged into `branch`.
///
/// `review-pending` and `needs-decision` off, the merged note with `how`,
/// then the close — what unblocks the tasks waiting on it. The tail the
/// merge train and the sweep share.
///
/// # Errors
/// [`harness_core::domain::Halt`] from a label, the comment or the close.
pub async fn close(
    gh: &dyn GitHub,
    task: &Issue,
    branch: &str,
    pr_ref: &str,
    how: &str,
) -> Outcome<()> {
    if tasks::review_pending(task) {
        gh.remove_label(task.number, labels::REVIEW_PENDING).await?;
    }
    if task.has(labels::NEEDS_DECISION) {
        gh.remove_label(task.number, labels::NEEDS_DECISION).await?;
    }
    gh.post_issue_comment(
        task.number,
        &format!("{} — {how}", delivery::merged_note(branch, pr_ref)),
    )
    .await?;
    gh.close_issue(task.number).await
}

/// Closes a task the sweep found: its PR's body given the `Closes #N` line
/// first when it lacks it (a PR proven by its head), then [`close`].
///
/// # Errors
/// [`harness_core::domain::Halt`] from a read or a write.
pub async fn close_delivered(gh: &dyn GitHub, delivered: &Delivered) -> Outcome<()> {
    let pr = gh.issue(delivered.pr).await?;
    if let Some(body) = delivery::with_closes(&pr.body, delivered.task) {
        gh.set_body(delivered.pr, &body).await?;
    }
    let task = gh.issue(delivered.task).await?;
    if !task.is_open() {
        return Ok(());
    }
    close(
        gh,
        &task,
        &delivered.branch,
        &format!("#{}", delivered.pr),
        "merged outside the merge train, closed by the harness.",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Pr;

    fn task(number: u64, labels: &[&str], branch: &str) -> Issue {
        Issue {
            number,
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: format!("## Scope\n\nbranch: {branch}\n"),
            ..Issue::default()
        }
    }

    fn merged(number: u64, title: &str, body: &str) -> Issue {
        Issue {
            number,
            state: "closed".to_string(),
            title: title.to_string(),
            body: body.to_string(),
            ..Issue::default()
        }
    }

    fn pr(num: &str, head: &str) -> (String, Pr) {
        (
            num.to_string(),
            Pr {
                num: num.to_string(),
                head: head.to_string(),
                ..Pr::default()
            },
        )
    }

    #[tokio::test]
    async fn an_open_task_with_a_merged_pr_naming_it_is_closed() {
        let gh = FakeGitHub {
            prs: vec![pr("469", "feat/pill"), pr("470", "feat/other")],
            ..FakeGitHub::default()
        };
        let board = [
            task(463, &[labels::AGENT], "feat/list"),
            task(464, &[labels::AGENT], "feat/pill"),
            task(465, &[labels::AGENT], "feat/screen"),
        ];
        let prs = [
            merged(467, "feat: the list", "Closes #463"),
            merged(469, "feat: #464 the pill", "no line"),
            merged(470, "fix: unblock #465 (after #463)", "no line"),
        ];
        let found = due(&gh, &board, &prs, "milestone/27-x", &[])
            .await
            .expect("read");
        let tasks: Vec<(u64, u64)> = found.iter().map(|d| (d.task, d.pr)).collect();
        assert_eq!(
            tasks,
            [(463, 467), (464, 469)],
            "a Closes line, or a title confirmed by the head — never a bare mention"
        );
    }

    #[tokio::test]
    async fn human_closed_running_and_already_told_tasks_are_left() {
        let gh = FakeGitHub {
            issue_comments: vec![format!(
                "{} — merged outside the merge train",
                delivery::merged_note("milestone/27-x", "#90")
            )],
            ..FakeGitHub::default()
        };
        let mut closed = task(12, &[labels::AGENT], "feat/c");
        closed.state = "closed".to_string();
        let board = [
            task(10, &[labels::HUMAN], "feat/a"),
            task(11, &[labels::AGENT], "feat/b"),
            closed,
            task(13, &[labels::AGENT], "feat/d"),
        ];
        let prs = [
            merged(80, "x", "Closes #10"),
            merged(81, "x", "Closes #11"),
            merged(82, "x", "Closes #12"),
            merged(90, "x", "Closes #13"),
        ];
        let found = due(&gh, &board, &prs, "milestone/27-x", &[11])
            .await
            .expect("read");
        assert!(found.is_empty(), "{found:?}");
    }

    #[tokio::test]
    async fn closing_writes_the_missing_line_then_closes_the_task() {
        let mut open = task(464, &[labels::AGENT, labels::REVIEW_PENDING], "feat/pill");
        open.title = "the pill".to_string();
        let gh = FakeGitHub {
            issues: vec![open, merged(469, "feat: #464 the pill", "## Plan\n- a")],
            ..FakeGitHub::default()
        };
        close_delivered(
            &gh,
            &Delivered {
                task: 464,
                pr: 469,
                branch: "milestone/27-x".to_string(),
            },
        )
        .await
        .expect("closed");
        let writes = gh.writes();
        assert_eq!(
            writes.first(),
            Some(&Wrote::Body(
                469,
                "## Plan\n- a\n\nCloses #464\n".to_string()
            ))
        );
        assert!(writes.contains(&Wrote::Unlabelled(464, labels::REVIEW_PENDING.to_string())));
        assert!(writes.iter().any(|w| matches!(
            w,
            Wrote::Comment(464, said) if said.starts_with("Merged in `milestone/27-x` by PR #469")
        )));
        assert_eq!(writes.last(), Some(&Wrote::Closed(464)));
    }

    #[tokio::test]
    async fn a_task_closed_meanwhile_is_not_told_twice() {
        let mut done = task(5, &[labels::AGENT], "feat/a");
        done.state = "closed".to_string();
        let gh = FakeGitHub {
            issues: vec![done, merged(9, "x", "Closes #5")],
            ..FakeGitHub::default()
        };
        close_delivered(
            &gh,
            &Delivered {
                task: 5,
                pr: 9,
                branch: "milestone/1-a".to_string(),
            },
        )
        .await
        .expect("nothing to do");
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    }
}
