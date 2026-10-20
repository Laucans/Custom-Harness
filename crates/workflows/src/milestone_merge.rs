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
//!
//! One pull request sits on no board: the rules' task's, on `rules/<n>`,
//! judged by its own reader, who merges it. When the harness decided to
//! merge it as it is while its checks still ran, that reader is gone and
//! the decision is a review on the PR ([`review::decided_to_merge`]): the
//! train takes it once green ([`rules_ready_to_merge`]) and closes the
//! task — which is what frees the development the rules hold.

use harness_core::domain::{Issue, Outcome, Pr};
use harness_core::ports::shell::github::GitHub;

use crate::common::{delivery, labels, review};
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
            // Red past its repairs: the harness decides (repair once more,
            // merge as it is, or cut), as for a review or a conflict that
            // the repairs did not settle.
            if review::repairs(&comments) >= review::MAX_FIXES
                && !gh.pr_failing_checks(&pr.num).await?.is_empty()
                && escalate(gh, task, pr).await?
            {
                escalated.push((task.number, pr.reference()));
            }
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

/// The branch a rules' task's pull request comes from.
#[must_use]
pub fn rules_branch(issue: u64) -> String {
    format!("rules/{issue}")
}

/// The open rules' tasks whose pull request is open on [`rules_branch`],
/// with that PR, lowest number first.
#[must_use]
pub fn rules_candidates<'a>(rules: &'a [Issue], open: &'a [Pr]) -> Vec<(&'a Issue, &'a Pr)> {
    let mut waiting: Vec<&Issue> = rules.iter().filter(|issue| issue.is_open()).collect();
    waiting.sort_by_key(|issue| issue.number);
    waiting
        .into_iter()
        .filter_map(|issue| {
            let branch = rules_branch(issue.number);
            open.iter()
                .find(|pr| pr.head == branch)
                .map(|pr| (issue, pr))
        })
        .collect()
}

/// The rules' task whose PR the train merges into the integration branch.
///
/// The first of [`rules_candidates`] whose PR the harness decided to merge
/// as it is ([`review::decided_to_merge`]), which merges and is green now.
/// A rules' PR is never agent-reviewed: its own reader judges it and
/// merges it, so a clean review alone is not enough — the decision is. One
/// whose checks still run, or whose branch conflicts since, waits.
///
/// # Errors
/// [`harness_core::domain::Halt`] from any read of a PR's comments, checks
/// or mergeability.
pub async fn rules_ready_to_merge<'a>(
    gh: &dyn GitHub,
    rules: &'a [Issue],
    open: &'a [Pr],
) -> Outcome<Option<(&'a Issue, &'a Pr)>> {
    for (task, pr) in rules_candidates(rules, open) {
        if !review::decided_to_merge(&gh.pr_comments(&pr.num).await?) {
            continue;
        }
        if gh.pr_mergeable(&pr.num).await? == Some(false) {
            continue;
        }
        if !gh.pr_checks_green(&pr.num).await? {
            continue;
        }
        return Ok(Some((task, pr)));
    }
    Ok(None)
}

/// What one merge attempt came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attempt {
    /// Merged, the task closed.
    Merged,
    /// A check is still running: nothing was done. The merge train takes
    /// the PR again once it is green.
    Pending,
    /// A check concluded in failure: nothing was done. The repair route's,
    /// not a merge's — the lines are what the checks said.
    Red(Vec<String>),
}

/// Merges `pr` into its milestone branch and closes `task`:
/// `review-pending` and `needs-decision` off, a comment naming the PR, then
/// the close — what unblocks the tasks waiting on it.
///
/// **Never on red or pending checks**, whoever calls it: the router reads
/// the checks before routing here, but a decision to merge "as it is" and
/// a check that turned red between the read and the merge both reach this
/// one line, so the guard sits here, on the merge itself.
///
/// # Errors
/// [`harness_core::domain::Halt`] from the checks read, the merge or any
/// label write — GitHub refusing the merge is not retried.
pub async fn run(gh: &dyn GitHub, task: &Issue, pr: &Pr) -> Outcome<Attempt> {
    if !gh.pr_checks_green(&pr.num).await? {
        let failing = gh.pr_failing_checks(&pr.num).await?;
        return Ok(if failing.is_empty() {
            Attempt::Pending
        } else {
            Attempt::Red(failing)
        });
    }
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
    gh.close_issue(task.number).await?;
    Ok(Attempt::Merged)
}

/// How many times `decide` is asked about one task's PR before a human is.
const REVIEW_DECISIONS: u32 = 2;

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
    if task.has(labels::NEEDS_DECISION) || task.has(labels::HUMAN) {
        return Ok(false);
    }
    let why = format!(
        "PR {} still does not merge after {} repair attempts — its review blocks \
         it, or its branch conflicts with the milestone's.",
        pr.reference(),
        crate::common::review::MAX_FIXES
    );
    // The question is written under `## Decision` as the stop handler
    // writes its own: an answer the owner gave to an earlier question (a
    // contract, a port) would otherwise stand for this one, and `decide`
    // would defer to it at every tick, deciding nothing (ForgeDesMondes'
    // #157, 2026-10-20: three ticks of "the owner already decided" on PR
    // #279, blocked after two repairs, while the owner's answer was about
    // the Session aggregate).
    let asked = crate::common::sections::parse(&task.body)
        .get(crate::common::sections::DECISION)
        .and_then(|text| crate::common::decision::Question::parse(text));
    let round = asked
        .filter(|q| q.kind.as_deref() == Some("review"))
        .map_or(1, |q| q.round.saturating_add(1));
    // Each decision that says "repair" buys two more repairs: past
    // `REVIEW_DECISIONS` of them, the PR is a human's, not a third paid
    // cycle (ForgeDesMondes' #312, 2026-10-20: repair, decide, repair, decide).
    if round > REVIEW_DECISIONS {
        gh.post_issue_comment(
            task.number,
            &format!(
                "{why} The harness already decided {REVIEW_DECISIONS} times to repair it; \
                 it stops spending on it. A human decides: repair it by hand, merge it \
                 as it is, or cut the task — then remove `{}`.",
                labels::HUMAN
            ),
        )
        .await?;
        gh.add_label(task.number, labels::HUMAN).await?;
        return Ok(true);
    }
    let question = crate::common::decision::Question {
        kind: Some("review".to_string()),
        blocks: why.clone(),
        round,
        ..crate::common::decision::Question::default()
    };
    let body = crate::common::sections::replace(
        &task.body,
        &[(crate::common::sections::DECISION, &question.render())],
    );
    gh.set_body(task.number, &body).await?;
    gh.post_issue_comment(
        task.number,
        &format!(
            "{why} The harness decides at the next tick — repair it once more, merge it \
             as it is, or cut the task — and says what it decided; answer under \
             `## Decision` first to decide it yourself."
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
    async fn an_exhausted_pr_asks_its_own_question_over_an_earlier_owner_answer() {
        // ForgeDesMondes #157: the owner had answered a contract question;
        // decide deferred to it on the PR's repairs for three ticks.
        let mut stale = task(157, &[labels::AGENT, labels::READY], "feat/create-session");
        stale.body.push_str(
            "\n## Decision\n\nkind: contract\nblocks: contract Session — an invariant\nround: 0\ndecided-by: owner\nverdict: answer — evolve it in this task\n",
        );
        let gh = FakeGitHub::default();
        assert!(
            escalate(&gh, &stale, &pr("279", "feat/create-session"))
                .await
                .expect("escalates")
        );
        let writes = gh.writes();
        let body = writes
            .iter()
            .find_map(|w| match w {
                Wrote::Body(157, body) => Some(body.clone()),
                _ => None,
            })
            .expect("the body is rewritten");
        let decision = body
            .split("## Decision")
            .nth(1)
            .expect("a Decision section");
        assert!(decision.contains("kind: review"), "{decision}");
        assert!(
            decision.contains("PR #279 still does not merge"),
            "{decision}"
        );
        assert!(
            !decision.contains("decided-by: owner"),
            "the stale answer is gone: {decision}"
        );
        assert!(decision.contains("round: 1"), "{decision}");
        assert!(writes.contains(&Wrote::Label(157, labels::NEEDS_DECISION.to_string())));
        // Asked again after another two repairs: round 2.
        let mut again = stale.clone();
        again.body = body;
        let gh = FakeGitHub::default();
        assert!(
            escalate(&gh, &again, &pr("279", "feat/create-session"))
                .await
                .expect("escalates")
        );
        let body = gh
            .writes()
            .iter()
            .find_map(|w| match w {
                Wrote::Body(157, body) => Some(body.clone()),
                _ => None,
            })
            .expect("rewritten");
        assert!(body.contains("round: 2"), "{body}");
        // Asked a third time: a human's, and no decision is asked.
        let mut third = again.clone();
        third.body = body;
        let gh = FakeGitHub::default();
        assert!(
            escalate(&gh, &third, &pr("279", "feat/create-session"))
                .await
                .expect("hands over")
        );
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::Label(157, labels::HUMAN.to_string())));
        assert!(!writes.contains(&Wrote::Label(157, labels::NEEDS_DECISION.to_string())));
        // Once a human's, never asked again.
        let mut held = third.clone();
        held.labels.push(labels::HUMAN.to_string());
        let gh = FakeGitHub::default();
        assert!(
            !escalate(&gh, &held, &pr("279", "feat/create-session"))
                .await
                .expect("reads")
        );
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn a_red_pr_past_its_repairs_goes_to_a_decision() {
        use crate::common::review;
        let board = vec![task(
            272,
            &[labels::AGENT, labels::REVIEW_PENDING],
            "feat/upload",
        )];
        let open = [pr("90", "feat/upload")];
        let fixes = format!("{}\n", review::FIX_MARKER).repeat(review::MAX_FIXES);
        // Two repairs, then a clean review: the review is satisfied, the CI
        // still red (ForgeDesMondes #304).
        let comments = format!("{fixes}{}\n{}\n", review::MARKER, review::CLEAN);
        let gh = FakeGitHub {
            pr_comment_bodies: vec![("90".to_string(), comments)],
            pr_checks: HashMap::from([("90".to_string(), false)]),
            failing_checks: HashMap::from([("90".to_string(), vec!["rust — FAILURE".to_string()])]),
            ..FakeGitHub::default()
        };
        let (found, escalated) = ready_to_merge(&gh, &board, &open).await.expect("read");
        assert!(found.is_none());
        assert_eq!(escalated, [(272, "#90".to_string())]);
        // Pending (red not yet), or repairs left: no decision.
        let gh = FakeGitHub {
            pr_comment_bodies: vec![(
                "90".to_string(),
                format!("{}\n{}\n", review::MARKER, review::CLEAN),
            )],
            pr_checks: HashMap::from([("90".to_string(), false)]),
            failing_checks: HashMap::from([("90".to_string(), vec!["rust — FAILURE".to_string()])]),
            ..FakeGitHub::default()
        };
        let (_, escalated) = ready_to_merge(&gh, &board, &open).await.expect("read");
        assert!(escalated.is_empty(), "repairs left: the repair route's");
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
        let gh = FakeGitHub {
            pr_checks: HashMap::from([("77".to_string(), true)]),
            ..FakeGitHub::default()
        };
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

    fn waiting() -> Issue {
        task(
            64,
            &[
                labels::AGENT,
                labels::REVIEW_PENDING,
                labels::NEEDS_DECISION,
            ],
            "chore/rust",
        )
    }

    #[tokio::test]
    async fn a_merge_marks_the_task_delivered() {
        let gh = FakeGitHub {
            pr_checks: HashMap::from([("68".to_string(), true)]),
            ..FakeGitHub::default()
        };
        let came_to = run(&gh, &waiting(), &pr("68", "chore/rust"))
            .await
            .expect("merged");
        assert_eq!(came_to, Attempt::Merged);
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::MergedPr("68".to_string())));
        assert!(writes.contains(&Wrote::Unlabelled(64, labels::REVIEW_PENDING.to_string())));
        assert!(writes.contains(&Wrote::Unlabelled(64, labels::NEEDS_DECISION.to_string())));
        assert!(writes.contains(&Wrote::Closed(64)));
    }

    fn rules_task(number: u64, state: &str) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            labels: vec![labels::RULES.to_string(), labels::AI_DECIDED.to_string()],
            ..Issue::default()
        }
    }

    fn rules_pr(num: &str, issue: u64) -> Pr {
        Pr {
            num: num.to_string(),
            base: "main_agent".to_string(),
            head: rules_branch(issue),
            state: "OPEN".to_string(),
            ..Pr::default()
        }
    }

    #[test]
    fn the_rules_candidates_are_the_open_rules_tasks_with_their_pr_on_rules_n() {
        let rules = vec![
            rules_task(9, "open"),
            rules_task(4, "open"),
            rules_task(2, "closed"),
        ];
        let open = [rules_pr("231", 9), rules_pr("200", 2), rules_pr("40", 4)];
        let found: Vec<(u64, &str)> = rules_candidates(&rules, &open)
            .into_iter()
            .map(|(t, p)| (t.number, p.num.as_str()))
            .collect();
        assert_eq!(found, [(4, "40"), (9, "231")], "#2 is closed");
        assert!(rules_candidates(&rules, &[rules_pr("7", 11)]).is_empty());
        assert_eq!(rules_branch(9), "rules/9");
    }

    #[tokio::test]
    async fn a_rules_pr_the_harness_decided_to_merge_is_taken_once_green_and_only_then() {
        // The live failure's own PR (#231, a rules contest) was decided
        // while its checks ran: the decision sat on a PR no board holds.
        let rules = vec![rules_task(9, "open")];
        let open = [rules_pr("231", 9)];
        let decided = format!(
            "{}\n{}\n{}\nThe harness decided to merge this as it is",
            review::MARKER,
            review::CLEAN,
            review::DECIDED_MERGE
        );
        let green = FakeGitHub {
            pr_comment_bodies: vec![("231".to_string(), decided.clone())],
            pr_checks: HashMap::from([("231".to_string(), true)]),
            ..FakeGitHub::default()
        };
        let found = rules_ready_to_merge(&green, &rules, &open)
            .await
            .expect("read");
        assert_eq!(
            found.map(|(t, p)| (t.number, p.num.as_str())),
            Some((9, "231"))
        );
        let pending = FakeGitHub {
            pr_comment_bodies: vec![("231".to_string(), decided.clone())],
            pr_checks: HashMap::from([("231".to_string(), false)]),
            ..FakeGitHub::default()
        };
        assert!(
            rules_ready_to_merge(&pending, &rules, &open)
                .await
                .expect("read")
                .is_none(),
            "its checks still run"
        );
        let conflicting = FakeGitHub {
            pr_comment_bodies: vec![("231".to_string(), decided)],
            pr_checks: HashMap::from([("231".to_string(), true)]),
            pr_mergeable: HashMap::from([("231".to_string(), Some(false))]),
            ..FakeGitHub::default()
        };
        assert!(
            rules_ready_to_merge(&conflicting, &rules, &open)
                .await
                .expect("read")
                .is_none(),
            "its branch conflicts since the decision"
        );
        // Its own reader judges it: no decision, no merge by the train —
        // even reviewed clean by someone.
        let judged = FakeGitHub {
            pr_comment_bodies: vec![(
                "231".to_string(),
                format!("{}\n{}", review::MARKER, review::CLEAN),
            )],
            pr_checks: HashMap::from([("231".to_string(), true)]),
            ..FakeGitHub::default()
        };
        assert!(
            rules_ready_to_merge(&judged, &rules, &open)
                .await
                .expect("read")
                .is_none()
        );
        assert!(
            judged.writes().is_empty(),
            "the train's read writes nothing"
        );
    }

    #[tokio::test]
    async fn nothing_merges_on_a_red_check_nor_on_one_still_running() {
        // The live failure: PR #231 merged with 7 red checks because the
        // caller never looked at them. The merge itself looks now.
        let red = FakeGitHub {
            pr_checks: HashMap::from([("68".to_string(), false)]),
            failing_checks: HashMap::from([(
                "68".to_string(),
                vec!["ci — FAILURE — https://ci/1".to_string()],
            )]),
            ..FakeGitHub::default()
        };
        let came_to = run(&red, &waiting(), &pr("68", "chore/rust"))
            .await
            .expect("refused, not failed");
        assert_eq!(
            came_to,
            Attempt::Red(vec!["ci — FAILURE — https://ci/1".to_string()])
        );
        assert_eq!(red.writes(), [] as [Wrote; 0], "nothing written on red");
        let pending = FakeGitHub {
            pr_checks: HashMap::from([("68".to_string(), false)]),
            failing_checks: HashMap::from([("68".to_string(), Vec::new())]),
            ..FakeGitHub::default()
        };
        let came_to = run(&pending, &waiting(), &pr("68", "chore/rust"))
            .await
            .expect("waits, not failed");
        assert_eq!(came_to, Attempt::Pending);
        assert_eq!(
            pending.writes(),
            [] as [Wrote; 0],
            "nothing written while pending"
        );
    }
}
