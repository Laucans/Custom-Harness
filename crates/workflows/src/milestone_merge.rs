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
/// `review-pending`, and the PR is open on the branch its body declares.
/// Lowest task number first.
#[must_use]
pub fn candidate<'a>(board: &'a [Issue], open: &'a [Pr]) -> Option<(&'a Issue, &'a Pr)> {
    let mut waiting: Vec<&Issue> = board
        .iter()
        .filter(|task| task.is_open() && tasks::review_pending(task))
        .collect();
    waiting.sort_by_key(|task| task.number);
    waiting.into_iter().find_map(|task| {
        let branch = tasks::declared_branch(&task.body)?;
        open.iter()
            .find(|pr| pr.head == branch)
            .map(|pr| (task, pr))
    })
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
    gh.remove_label(task.number, labels::REVIEW_PENDING).await?;
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

/// Hands `task` to a human: its PR's review stayed blocking after every
/// repair it gets. `needs-decision` on the task, once, with the PR named.
///
/// # Errors
/// [`harness_core::domain::Halt`] from the label or the comment write.
pub async fn escalate(gh: &dyn GitHub, task: &Issue, pr: &Pr) -> Outcome<()> {
    if task.has(labels::NEEDS_DECISION) {
        return Ok(());
    }
    gh.post_issue_comment(
        task.number,
        &format!(
            "PR {} still does not merge after {} repair attempts — its review blocks \
             it, or its branch conflicts with the milestone's — a human decides: fix \
             it by hand, or merge it as it is.",
            pr.reference(),
            crate::common::review::MAX_FIXES
        ),
    )
    .await?;
    gh.add_label(task.number, labels::NEEDS_DECISION).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};

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
