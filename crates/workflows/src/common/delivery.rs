//! What counts as proof that a task shipped: a PR declaring it closed.
//!
//! **A convention, not a GitHub property.** `Closes #n` on its own line is
//! what the harness writes into a task PR's body and what it reads back; the
//! adapter returns PRs, this module decides which one is proof.
//!
//! Shared because two levels need the same answer: the loop, to know a task
//! is delivered and must not be picked again, and the milestone merge, to
//! know every task's code is actually on the milestone's branch before it
//! proposes that branch upward. One reading of the convention, not two.
//!
//! Pure: no `gh` call lives here.

use harness_core::domain::Issue;

/// Does the PR body declare closing `#number`?
///
/// **Requires the entire line**, whereas GitHub accepts the keyword anywhere
/// in the body. Deliberately stricter, and rightly so: the instructions say
/// to put it on its own line, and reading wider would read an issue as
/// delivered on a sentence merely mentioning it.
///
/// Hand-written rather than regex: three keywords and a number don't warrant
/// a dependency, and line-by-line reading says exactly the rule.
#[must_use]
pub fn closes(body: &str, number: u64) -> bool {
    body.lines()
        .any(|line| closes_on_its_own_line(line, number))
}

fn closes_on_its_own_line(line: &str, number: u64) -> bool {
    let trimmed = line.trim_matches(|c| c == ' ' || c == '\t');
    let lowered = trimmed.to_lowercase();
    for keyword in ["closes", "fixes", "resolves"] {
        let Some(rest) = lowered.strip_prefix(keyword) else {
            continue;
        };
        // At least one space or tab between the keyword and the `#`.
        let rest = rest.trim_start_matches([' ', '\t']);
        if rest.len() == lowered.len() - keyword.len() {
            continue;
        }
        let Some(digits) = rest.strip_prefix('#') else {
            continue;
        };
        if digits.parse::<u64>() == Ok(number) {
            return true;
        }
    }
    false
}

/// The first PR in the list that declares closing `#number`.
#[must_use]
pub fn first_closing(prs: &[Issue], number: u64) -> Option<&Issue> {
    prs.iter().find(|pr| closes(&pr.body, number))
}

/// What is written on the task once its PR landed on the integration branch.
///
/// **One line, on purpose.** GitHub closes nothing on a merge outside the
/// default branch, so the only trace of delivery on the issue was a label; a
/// label says *that* it shipped, not *where*. The human reading the issue sees
/// which branch now carries the code, and which PR put it there, without
/// opening the branch list.
///
/// The branch is back-quoted so a slash in `milestone/16-…` cannot be read as
/// prose.
#[must_use]
pub fn merged_note(branch: &str, pr_ref: &str) -> String {
    format!("Merged in `{branch}` by PR {pr_ref}")
}

/// Does every one of these tasks have a merged PR declaring it closed?
///
/// **The git truth, not a label.** A task stays *open* once delivered — a PR
/// merged into a milestone branch closes nothing, since GitHub's closing
/// keywords only fire on the default branch — so the label that marks it
/// delivered is posed by the harness itself. Asking the merged PRs instead
/// means a task whose PR was opened but never merged (red CI, abandoned)
/// cannot be mistaken for shipped code, however it happens to be labelled.
///
/// `merged` is the merged PRs of the branch the tasks landed on.
///
/// A human's task (`harness:human`) ships no PR: it is done once closed,
/// whatever closed it — a commit on the integration branch, a setting
/// changed, a decision taken.
#[must_use]
pub fn every_task_merged(tasks: &[Issue], merged: &[Issue]) -> bool {
    !tasks.is_empty()
        && tasks.iter().all(|task| {
            if task.has(crate::common::labels::HUMAN) {
                task.is_closed()
            } else {
                first_closing(merged, task.number).is_some()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_human_task_needs_no_merged_pr() {
        let human = Issue {
            number: 67,
            state: "closed".to_string(),
            labels: vec![crate::common::labels::HUMAN.to_string()],
            ..Issue::default()
        };
        let agent = Issue {
            number: 64,
            state: "open".to_string(),
            ..Issue::default()
        };
        let merged = vec![pr(68, "Closes #64")];
        assert!(every_task_merged(&[agent.clone(), human.clone()], &merged));
        let open_human = Issue {
            state: "open".to_string(),
            ..human
        };
        assert!(!every_task_merged(&[agent, open_human], &merged));
    }

    fn pr(number: u64, body: &str) -> Issue {
        Issue {
            number,
            body: body.to_string(),
            ..Issue::default()
        }
    }

    fn task(number: u64) -> Issue {
        Issue {
            number,
            ..Issue::default()
        }
    }

    #[test]
    fn every_task_with_a_merged_pr_is_shipped() {
        let tasks = [task(11), task(12)];
        let merged = [pr(90, "Closes #11"), pr(91, "Closes #12")];
        assert!(every_task_merged(&tasks, &merged));
    }

    #[test]
    fn a_task_whose_pr_never_merged_is_not_shipped() {
        // The failure this guards: a task labelled delivered whose PR is
        // still open would otherwise let the milestone propose a branch
        // that does not carry its code.
        let tasks = [task(11), task(12)];
        let merged = [pr(90, "Closes #11")];
        assert!(!every_task_merged(&tasks, &merged));
    }

    #[test]
    fn no_tasks_at_all_is_not_shipped() {
        assert!(!every_task_merged(&[], &[pr(90, "Closes #11")]));
    }

    #[test]
    fn the_note_names_the_branch_that_now_carries_the_code_and_the_pr() {
        let said = merged_note("milestone/16-combat", "#58");
        assert!(said.starts_with("Merged in `milestone/16-combat`"));
        assert!(said.contains("#58"));
        // One line: the issue timeline is read, not parsed.
        assert_eq!(said.lines().count(), 1);
    }
}
