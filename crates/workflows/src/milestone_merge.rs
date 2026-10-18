//! `harness watch` merging a task's development branch into its milestone.
//!
//! A **deterministic command, not a workflow** — like `main_agent_merge`, one
//! level down: no session, no cost. A task on the write side leaves its pull
//! request open on the milestone branch (`review-pending`); once the agent
//! review has run and every check is green, this merges it and closes the
//! task — which is what unblocks the tasks that wait on it. The CI and the
//! review are what stand for the human review here.
//!
//! The choice of *which* PR is pure ([`candidate`]); the router reads the
//! board and the open PRs, asks this, and checks the CI before routing here.

use harness_core::domain::{Issue, Outcome, Pr};
use harness_core::ports::shell::github::GitHub;

use crate::common::{delivery, labels};
use crate::dev_loop::data::tasks;

/// The first task waiting on its PR, with that PR: the task is open and
/// `review-pending` or on the write side, and the PR is open on the branch
/// its body declares. Lowest task number first.
///
/// The write side counts without the label: the label is placed after the
/// round's `code` stage, and a round that halted past its PR (a post-gate,
/// then the budget) never placed it, and a reviewed, green
/// PR waited for a merge nothing would make.
/// The tasks of the board whose PR is open on its declared branch — the
/// write side's, or any task `review-pending` — with that PR, by number.
///
/// Every one, not the first: `ForgeDesMondes`' #51 (PR #105, red on a rule
/// its sibling #225 makes green) stood first and hid #225 (PR #233, clean
/// and green) for three hours — the train took neither.
#[must_use]
pub fn candidates<'a>(board: &'a [Issue], open: &'a [Pr]) -> Vec<(&'a Issue, &'a Pr)> {
    let mut waiting: Vec<&Issue> = board
        .iter()
        .filter(|task| {
            task.is_open() && (tasks::review_pending(task) || tasks::is_write_side(task))
        })
        .collect();
    waiting.sort_by_key(|task| task.number);
    waiting
        .into_iter()
        .filter_map(|task| {
            let branch = tasks::declared_branch(&task.body)?;
            open.iter()
                .find(|pr| pr.head == branch)
                .map(|pr| (task, pr))
        })
        .collect()
}

/// What the train merges now, and the tasks it escalated on the way, each
/// with its PR's reference.
///
/// The first of [`candidates`] whose last review is clean, whose PR merges
/// and whose checks are green; a task met before it whose repairs are used
/// up is escalated to a decision ([`escalate`]) and passed over. One that is
/// not green does not hide the next: `ForgeDesMondes`' #51
/// (PR #105, red on R4) stood first for three hours over #225 (PR #233,
/// clean and green, the very change that makes R4 pass).
///
/// # Errors
/// [`harness_core::domain::Halt`] from any read of a PR's comments, checks
/// or mergeability, or from the escalation's writes.
pub async fn ready_to_merge<'a>(
    gh: &dyn GitHub,
    board: &'a [Issue],
    open: &'a [Pr],
) -> Outcome<(Option<(&'a Issue, &'a Pr)>, Vec<(u64, String)>)> {
    use crate::common::review;
    let mut escalated = Vec::new();
    for (task, pr) in candidates(board, open) {
        let comments = gh.pr_comments(&pr.num).await?;
        match review::status(&comments) {
            review::Status::Clean => {}
            status if status.exhausted() => {
                if escalate(gh, task, pr).await? {
                    escalated.push((task.number, pr.reference()));
                }
                continue;
            }
            _ => continue,
        }
        if gh.pr_mergeable(&pr.num).await? == Some(false) {
            if review::repairs(&comments) >= review::MAX_FIXES && escalate(gh, task, pr).await? {
                escalated.push((task.number, pr.reference()));
            }
            continue;
        }
        if !gh.pr_checks_green(&pr.num).await? {
            continue;
        }
        return Ok((Some((task, pr)), escalated));
    }
    Ok((None, escalated))
}

/// The first of [`candidates`], when there is one.
#[must_use]
pub fn candidate<'a>(board: &'a [Issue], open: &'a [Pr]) -> Option<(&'a Issue, &'a Pr)> {
    candidates(board, open).into_iter().next()
}

/// Merges `pr` into its milestone branch and closes `task`:
/// `review-pending` and `needs-decision` off, a comment naming the PR, then
/// the close — what unblocks the tasks waiting on it.
///
/// # Errors
/// [`harness_core::domain::Halt`] from the merge or any label write —
/// GitHub refusing the merge is not retried.
pub async fn run(gh: &dyn GitHub, task: &Issue, pr: &Pr) -> Outcome<()> {
    gh.merge_pr(&pr.num).await?;
    if tasks::review_pending(task) {
        gh.remove_label(task.number, labels::REVIEW_PENDING).await?;
    }
    if task.has(labels::NEEDS_DECISION) {
        gh.remove_label(task.number, labels::NEEDS_DECISION).await?;
    }
    gh.post_issue_comment(
        task.number,
        &format!(
            "{} — reviewed and every check green, merged by `milestone_merge`.",
            delivery::merged_note(&pr.base, &pr.reference())
        ),
    )
    .await?;
    gh.close_issue(task.number).await
}

/// Asks for a decision on `task`, once; answers whether it asked now.
///
/// Its PR's review stayed blocking, or its branch conflicting, after every
/// repair it gets: `needs-decision` on the task, with the PR named — the
/// `decide` workflow takes it at the next tick (merge as it is, repair once
/// more, or cut the task) and tells the human what it decided.
///
/// # Errors
/// [`harness_core::domain::Halt`] from the label or the comment write.
pub async fn escalate(gh: &dyn GitHub, task: &Issue, pr: &Pr) -> Outcome<bool> {
    if task.has(labels::NEEDS_DECISION) {
        return Ok(false);
    }
    gh.post_issue_comment(
        task.number,
        &format!(
            "PR {} still does not merge after {} repair attempts — its review blocks \
             it, or its branch conflicts with the milestone's. The harness decides at the \
             next tick — repair it once more, merge it as it is, or cut the task — and says \
             what it decided; answer under `## Decision` first to decide it yourself.",
            pr.reference(),
            crate::common::review::MAX_FIXES
        ),
    )
    .await?;
    gh.add_label(task.number, labels::NEEDS_DECISION).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use std::collections::HashMap;

    fn task(number: u64, labels: &[&str], branch: &str) -> Issue {
        Issue {
            number,
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: format!("## Scope\n\nbranch: {branch}\n"),
            ..Issue::default()
        }
    }

    fn pr(num: &str, head: &str) -> Pr {
        Pr {
            num: num.to_string(),
            base: "milestone/52-socle".to_string(),
            head: head.to_string(),
            state: "OPEN".to_string(),
            ..Pr::default()
        }
    }

    #[test]
    fn the_candidate_is_a_review_pending_task_with_its_pr_open() {
        let board = vec![
            task(65, &[labels::AGENT], "chore/ts"),
            task(64, &[labels::AGENT, labels::REVIEW_PENDING], "chore/rust"),
        ];
        let open = vec![pr("70", "chore/ts"), pr("68", "chore/rust")];
        let (found, its_pr) = candidate(&board, &open).expect("one");
        assert_eq!(found.number, 64);
        assert_eq!(its_pr.num, "68");
        assert!(candidate(&board, &[pr("70", "chore/ts")]).is_none());

        // A write-side task whose round never placed the label still merges.
        let unlabelled = vec![task(39, &[labels::AGENT, labels::WRITE_SIDE], "feat/save")];
        let prs = [pr("77", "feat/save")];
        let (found, _) = candidate(&unlabelled, &prs).expect("write side");
        assert_eq!(found.number, 39);

        // Every waiting task with its PR, by number — the first may be red
        // while the second merges.
        let two = vec![
            task(
                225,
                &[labels::AGENT, labels::REVIEW_PENDING],
                "chore/wiring",
            ),
            task(51, &[labels::AGENT, labels::REVIEW_PENDING], "feat/save"),
            task(52, &[labels::AGENT, labels::WRITE_SIDE], "feat/restore"),
        ];
        let open = [pr("233", "chore/wiring"), pr("105", "feat/save")];
        let all: Vec<(u64, &str)> = candidates(&two, &open)
            .into_iter()
            .map(|(t, p)| (t.number, p.num.as_str()))
            .collect();
        assert_eq!(all, [(51, "105"), (225, "233")], "#52 has no PR open");
        assert_eq!(candidate(&two, &open).map(|(t, _)| t.number), Some(51));
    }

    #[tokio::test]
    async fn the_first_clean_mergeable_green_pr_merges_and_a_red_one_hides_nothing() {
        use crate::common::review;
        let board = vec![
            task(51, &[labels::AGENT, labels::REVIEW_PENDING], "feat/save"),
            task(
                225,
                &[labels::AGENT, labels::REVIEW_PENDING],
                "chore/wiring",
            ),
            task(60, &[labels::AGENT, labels::REVIEW_PENDING], "feat/stuck"),
        ];
        let open = [
            pr("105", "feat/save"),
            pr("233", "chore/wiring"),
            pr("90", "feat/stuck"),
        ];
        let clean = format!("{}\n{}", review::MARKER, review::CLEAN);
        let blocking = format!("{}\n{}", review::MARKER, review::BLOCKING);
        let gh = FakeGitHub {
            pr_comment_bodies: vec![
                ("105".to_string(), clean.clone()),
                ("233".to_string(), clean.clone()),
                ("90".to_string(), blocking),
            ],
            pr_checks: HashMap::from([("105".to_string(), false), ("233".to_string(), true)]),
            ..FakeGitHub::default()
        };
        let (found, escalated) = ready_to_merge(&gh, &board, &open).await.expect("read");
        let (task, its_pr) = found.expect("one merges");
        assert_eq!(
            (task.number, its_pr.num.as_str()),
            (225, "233"),
            "#51 is red: #225 is next"
        );
        assert!(
            escalated.is_empty(),
            "a blocking review with repairs left is the repair route's"
        );
        // Repairs used up on the first: escalated once, then the next merges.
        let exhausted = format!(
            "{}{}\n{}",
            review::FIX_MARKER.repeat(review::MAX_FIXES),
            review::MARKER,
            review::BLOCKING
        );
        let gh = FakeGitHub {
            pr_comment_bodies: vec![
                ("105".to_string(), exhausted),
                ("233".to_string(), clean.clone()),
                ("90".to_string(), clean),
            ],
            pr_checks: HashMap::from([("233".to_string(), true)]),
            pr_mergeable: HashMap::from([("90".to_string(), Some(false))]),
            ..FakeGitHub::default()
        };
        let (found, escalated) = ready_to_merge(&gh, &board, &open).await.expect("read");
        assert_eq!(found.map(|(t, _)| t.number), Some(225));
        assert_eq!(escalated, [(51, "#105".to_string())]);
        assert!(
            gh.writes()
                .contains(&Wrote::Label(51, labels::NEEDS_DECISION.to_string()))
        );
        // Nothing green: none, and nothing escalated for a mere red check.
        let gh = FakeGitHub {
            pr_comment_bodies: vec![(
                "105".to_string(),
                format!("{}\n{}", review::MARKER, review::CLEAN),
            )],
            pr_checks: HashMap::from([("105".to_string(), false)]),
            ..FakeGitHub::default()
        };
        let (found, escalated) = ready_to_merge(&gh, &board[..1], &open[..1])
            .await
            .expect("read");
        assert!(found.is_none() && escalated.is_empty());
    }

    #[tokio::test]
    async fn a_merge_removes_only_the_labels_the_task_carries() {
        let gh = FakeGitHub::default();
        let bare = task(39, &[labels::AGENT, labels::WRITE_SIDE], "feat/save");
        run(&gh, &bare, &pr("77", "feat/save"))
            .await
            .expect("merged");
        let writes = gh.writes();
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::Unlabelled(..))),
            "{writes:?}"
        );
        assert!(writes.contains(&Wrote::Closed(39)));
    }

    #[tokio::test]
    async fn a_review_its_repairs_did_not_clear_goes_to_a_human_once() {
        let gh = FakeGitHub::default();
        let stuck = task(64, &[labels::AGENT, labels::REVIEW_PENDING], "chore/rust");
        escalate(&gh, &stuck, &pr("68", "chore/rust"))
            .await
            .expect("escalated");
        assert!(
            gh.writes()
                .contains(&Wrote::Label(64, labels::NEEDS_DECISION.to_string()))
        );
        let decided = task(64, &[labels::AGENT, labels::NEEDS_DECISION], "chore/rust");
        let again = FakeGitHub::default();
        escalate(&again, &decided, &pr("68", "chore/rust"))
            .await
            .expect("nothing to do");
        assert_eq!(again.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn a_merge_marks_the_task_delivered() {
        let gh = FakeGitHub::default();
        let waiting = task(
            64,
            &[
                labels::AGENT,
                labels::REVIEW_PENDING,
                labels::NEEDS_DECISION,
            ],
            "chore/rust",
        );
        run(&gh, &waiting, &pr("68", "chore/rust"))
            .await
            .expect("merged");
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::MergedPr("68".to_string())));
        assert!(writes.contains(&Wrote::Unlabelled(64, labels::REVIEW_PENDING.to_string())));
        assert!(writes.contains(&Wrote::Unlabelled(64, labels::NEEDS_DECISION.to_string())));
        assert!(writes.contains(&Wrote::Closed(64)));
    }
}
