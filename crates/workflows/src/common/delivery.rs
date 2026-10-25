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
    closed_on_its_own_line(line) == Some(number)
}

/// The task a PR body declares it closes.
///
/// The first line [`closes`] reads as one, with the same keywords and
/// spacing. What a PR's repair names as
/// its requester (`pr_fix`), so a body saying `Fixes #482` is read as the
/// merge train reads it.
#[must_use]
pub fn closed_task(body: &str) -> Option<u64> {
    body.lines().find_map(closed_on_its_own_line)
}

/// The number a whole line declares closed: `<keyword> #<n>`, the keyword
/// one of `closes`, `fixes`, `resolves` in any case, at least one space or
/// tab before the `#`.
fn closed_on_its_own_line(line: &str) -> Option<u64> {
    let lowered = line.trim_matches(|c| c == ' ' || c == '\t').to_lowercase();
    ["closes", "fixes", "resolves"].iter().find_map(|keyword| {
        let rest = lowered.strip_prefix(keyword)?;
        let spaced = rest.trim_start_matches([' ', '\t']);
        if spaced.len() == rest.len() {
            return None;
        }
        spaced.strip_prefix('#')?.parse::<u64>().ok()
    })
}

/// The body with a `Closes #number` line added at its end, or `None` when
/// [`closes`] already holds: every line of the body is kept, in order.
///
/// What the harness writes on a PR before it merges it, and on a PR a human
/// merged without the line: the merged PR is then the proof
/// [`every_task_merged`] reads (`ForgeDesMondes`' milestone #20 waited
/// `NotReady` on a PR body without its line, 2026-10-24).
#[must_use]
pub fn with_closes(body: &str, number: u64) -> Option<String> {
    if closes(body, number) {
        return None;
    }
    let kept = body.trim_end();
    Some(if kept.is_empty() {
        format!("Closes #{number}\n")
    } else {
        format!("{kept}\n\nCloses #{number}\n")
    })
}

/// Whether a title names `#number` as a whole token — never `#2480` for #248.
///
/// A title alone proves nothing: a PR titled `fix: unblock #465
/// (after #463)` names two tasks it does not deliver. It is a hint the
/// caller confirms by the PR's head branch.
#[must_use]
pub fn title_names(title: &str, number: u64) -> bool {
    let token = format!("#{number}");
    title.match_indices(&token).any(|(at, _)| {
        title
            .get(at.saturating_add(token.len())..)
            .and_then(|rest| rest.chars().next())
            .is_none_or(|next| !next.is_ascii_digit())
    })
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
            // A task closed as not planned was given up or covered by
            // another: no PR of its own will ever land (ForgeDesMondes' #19,
            // 2026-10-20: #117, #119 and #123, covered by #243 under #18,
            // held the milestone's merge at every tick).
            if task.has(crate::common::labels::HUMAN) || task.not_planned {
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
    fn a_task_closed_as_not_planned_needs_no_merged_pr() {
        let covered = Issue {
            number: 117,
            state: "closed".to_string(),
            not_planned: true,
            ..Issue::default()
        };
        let shipped = Issue {
            number: 114,
            state: "closed".to_string(),
            ..Issue::default()
        };
        let merged = vec![pr(145, "Closes #114")];
        assert!(every_task_merged(
            &[shipped.clone(), covered.clone()],
            &merged
        ));
        let open_covered = Issue {
            state: "open".to_string(),
            ..covered
        };
        assert!(
            !every_task_merged(&[shipped.clone(), open_covered], &merged),
            "still open, it is not given up yet"
        );
        let unshipped = Issue {
            number: 119,
            state: "closed".to_string(),
            ..Issue::default()
        };
        assert!(
            !every_task_merged(&[shipped, unshipped], &merged),
            "closed as done, it needs its PR"
        );
    }

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

    proptest::proptest! {
        #[test]
        fn with_closes_makes_closes_hold_and_is_idempotent(
            body in "(?s).{0,200}",
            number in 1u64..100_000,
        ) {
            let written = with_closes(&body, number);
            let after = written.clone().unwrap_or_else(|| body.clone());
            proptest::prop_assert!(closes(&after, number));
            proptest::prop_assert_eq!(with_closes(&after, number), None);
            if let Some(written) = written {
                // Every line of the body is kept, in order, before the line.
                proptest::prop_assert!(written.starts_with(body.trim_end()));
            }
        }

        #[test]
        fn a_title_names_its_task_and_never_a_longer_number(
            number in 1u64..100_000,
            digit in 0u32..10,
            before in "[a-z :(]{0,12}",
            after in "[a-z :)]{0,12}",
        ) {
            let named = format!("{before}#{number}{after}");
            proptest::prop_assert!(title_names(&named, number));
            let longer = format!("{before}#{number}{digit}{after}");
            proptest::prop_assert!(!title_names(&longer, number));
        }

        #[test]
        fn the_closed_task_is_what_closes_reads(
            keyword in proptest::sample::select(vec![
                "Closes", "closes", "FIXES", "Fixes", "resolves", "Resolves",
            ]),
            gap in "[ \t]{1,3}",
            margin in "[ \t]{0,2}",
            number in 1u64..100_000,
            before in "[a-z ]{0,20}",
        ) {
            let body = format!("{before}\n{margin}{keyword}{gap}#{number}{margin}\nmore");
            proptest::prop_assert_eq!(closed_task(&body), Some(number));
            proptest::prop_assert!(closes(&body, number));
            // Glued to its number, or in a sentence: no task, as for closes.
            let glued = format!("{keyword}#{number}");
            proptest::prop_assert_eq!(closed_task(&glued), None);
            proptest::prop_assert!(!closes(&glued, number));
            let said = format!("this {keyword}{gap}#{number}");
            proptest::prop_assert_eq!(closed_task(&said), None);
        }
    }

    #[test]
    fn the_first_closing_line_names_the_task() {
        assert_eq!(closed_task("## Plan\n\nFixes #482\nCloses #9"), Some(482));
        assert_eq!(closed_task("Closes #x\nresolves\t#7"), Some(7));
        assert_eq!(closed_task("no closing line"), None);
    }

    #[test]
    fn a_body_with_its_line_is_left_and_one_without_gets_it_at_the_end() {
        assert_eq!(with_closes("x\nCloses #4\n", 4), None);
        assert_eq!(
            with_closes("## Plan\n- a\n\n", 4).as_deref(),
            Some("## Plan\n- a\n\nCloses #4\n")
        );
        assert_eq!(with_closes("", 4).as_deref(), Some("Closes #4\n"));
        assert_eq!(
            with_closes("Closes #40", 4).as_deref(),
            Some("Closes #40\n\nCloses #4\n")
        );
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
