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
//! train takes it once green ([`rules_prs`], [`RulesPr::Mergeable`]) and
//! closes the task — which is what frees the development the rules hold.

use harness_core::domain::{Issue, Outcome, Pr};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Logbook;

use crate::common::branching::BaseHead;
use crate::common::{catch_up, contest, delivery, labels, review};
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
/// The caller passes `open` without the PRs that wait
/// (`contest::not_waiting`): a waiting PR is neither escalated nor merged.
/// `held` holds the issues and PRs the lanes work on now, by number (the
/// lanes' keys). A PR held is not escalated while its repair runs — its
/// last repair's marker is posted before the session, the checks still red
/// from the push before. A task held — a decision taken on it, which takes
/// `harness:needs-decision` off before its session — is neither merged nor
/// escalated: asked again, the question is decided twice
/// (`ForgeDesMondes`' #542, PR #547, 2026-10-25: two "repair" verdicts
/// three minutes apart).
///
/// `integration` is the integration branch, where an issue a decided wait
/// named lands when it was built under another milestone; `log` journals
/// a PR resumed on its settled waits ([`journal`]).
///
/// # Errors
/// [`harness_core::domain::Halt`] from any read of a PR's comments, checks
/// or mergeability, or from the escalation's writes.
pub async fn ready_to_merge<'a>(
    gh: &dyn GitHub,
    board: &'a [Issue],
    open: &'a [Pr],
    held: &[u64],
    integration: &str,
    log: &Logbook,
) -> Outcome<(Option<(&'a Issue, &'a Pr)>, Vec<(u64, String)>)> {
    let mut escalated = Vec::new();
    for (task, pr) in candidates(board, open) {
        if held.contains(&task.number)
            || pr
                .num
                .parse::<u64>()
                .is_ok_and(|number| held.contains(&number))
        {
            continue;
        }
        let comments = gh.pr_comments(&pr.num).await?;
        match review::status(&comments) {
            review::Status::Clean => {}
            status if status.exhausted() => {
                if journal(
                    log,
                    pr,
                    &escalate(gh, task, pr, &comments, integration).await?,
                ) {
                    escalated.push((task.number, pr.reference()));
                }
                continue;
            }
            _ => continue,
        }
        if gh.pr_mergeable(&pr.num).await? == Some(false) {
            if review::repairs(&comments) >= review::MAX_FIXES
                && journal(
                    log,
                    pr,
                    &escalate(gh, task, pr, &comments, integration).await?,
                )
            {
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
                && journal(
                    log,
                    pr,
                    &escalate(gh, task, pr, &comments, integration).await?,
                )
            {
                escalated.push((task.number, pr.reference()));
            }
            continue;
        }
        return Ok((Some((task, pr)), escalated));
    }
    Ok((None, escalated))
}

/// Escalates the PRs labelled `harness:pr-fix` past their repairs that no
/// other path escalates; answers the (task, PR) pairs asked now.
///
/// The repair cap covers every `harness:pr-fix` PR (`pr_fix::data::candidates::due`),
/// and past it a red or conflicting PR is repaired no more. A task's PR
/// under review is [`ready_to_merge`]'s to escalate, a milestone's landing
/// PR `main_agent_merge`'s, a rules' PR the rules train's (its red stand
/// re-taps the rules' task). The rest — a task's PR with no
/// `harness:to-review` (`ForgeDesMondes`' #498) — is escalated here on the
/// task its body closes (`delivery::closed_task`), [`escalate`] asking once.
/// The caller passes `to_fix` without the PRs a lane repairs now and those
/// that wait (`contest::not_waiting`): neither is escalated. Nor is one
/// whose task a lane holds (`held`, the lanes' keys): a decision taken on
/// it is not asked again ([`ready_to_merge`]). `integration` and `log` as
/// [`ready_to_merge`] takes them.
///
/// # Errors
/// [`harness_core::domain::Halt`] from a read of a PR, its comments or its
/// task, or from the escalation's writes.
pub async fn exhausted_repairs(
    gh: &dyn GitHub,
    to_fix: &[Pr],
    to_review: &[Pr],
    held: &[u64],
    integration: &str,
    log: &Logbook,
) -> Outcome<Vec<(u64, String)>> {
    let mut escalated = Vec::new();
    for pr in to_fix {
        let covered = to_review.iter().any(|other| other.num == pr.num)
            || crate::common::branching::milestone_number(&pr.head).is_some()
            || pr.head.starts_with("rules/");
        let Ok(number) = pr.num.parse::<u64>() else {
            continue;
        };
        if covered {
            continue;
        }
        let comments = gh.pr_comments(&pr.num).await?;
        if review::repairs(&comments) < review::MAX_FIXES {
            continue;
        }
        let red = !gh.pr_failing_checks(&pr.num).await?.is_empty()
            || gh.pr_mergeable(&pr.num).await? == Some(false);
        if !red {
            continue;
        }
        let Some(task) = delivery::closed_task(&gh.issue(number).await?.body) else {
            log.debug(&format!(
                "watch: PR {} is red past its repairs and closes no task — nobody to ask",
                pr.reference()
            ));
            continue;
        };
        if held.contains(&task) {
            continue;
        }
        let owner = gh.issue(task).await?;
        if journal(
            log,
            pr,
            &escalate(gh, &owner, pr, &comments, integration).await?,
        ) {
            escalated.push((task, pr.reference()));
        }
    }
    Ok(escalated)
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

/// Where a rules' task's open PR stands with the train.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulesPr {
    /// Judged faithful (or decided merged), mergeable and green: the train
    /// merges it.
    Mergeable,
    /// Judged faithful (or decided merged), no check failed, not green yet:
    /// its checks still run. The task is neither tapped nor attempted again
    /// meanwhile — a second attempt would judge the same PR faithful again.
    Checking,
    /// Judged faithful (or decided merged), and a check failed or its
    /// branch conflicts with the base: attempted again, its write prompt
    /// checks `rules/<n>` out and answers its review.
    Red,
    /// No verdict yet, or judged unfaithful: its own reader re-taps it.
    Unjudged,
}

/// The open rules' PRs ([`rules_candidates`]), each with where it stands.
///
/// The train merges the first [`RulesPr::Mergeable`]: its fidelity reader
/// judged it faithful ([`review::judged_faithful`]) or the harness decided
/// to merge it as it is ([`review::decided_to_merge`]), and it merges and
/// is green now. A rules' PR is never agent-reviewed: its own reader judges
/// it, and the harness merges it — a session merges no PR — so a clean
/// review alone is not enough: the verdict or the decision is. One whose
/// checks still run, or whose branch conflicts since, waits.
///
/// # Errors
/// [`harness_core::domain::Halt`] from any read of a PR's comments, checks
/// or mergeability.
pub async fn rules_prs<'a>(
    gh: &dyn GitHub,
    rules: &'a [Issue],
    open: &'a [Pr],
) -> Outcome<Vec<(&'a Issue, &'a Pr, RulesPr)>> {
    let mut found = Vec::new();
    for (task, pr) in rules_candidates(rules, open) {
        let comments = gh.pr_comments(&pr.num).await?;
        let stands = if !(review::judged_faithful(&comments) || review::decided_to_merge(&comments))
        {
            RulesPr::Unjudged
        } else if gh.pr_mergeable(&pr.num).await? == Some(false) {
            RulesPr::Red
        } else if gh.pr_checks_green(&pr.num).await? {
            RulesPr::Mergeable
        } else if gh.pr_failing_checks(&pr.num).await?.is_empty() {
            RulesPr::Checking
        } else {
            RulesPr::Red
        };
        found.push((task, pr, stands));
    }
    Ok(found)
}

/// The rules' task whose PR the train merges — the first of [`rules_prs`]
/// that is [`RulesPr::Mergeable`], as the launcher's train takes it; for
/// the tests, which read the train's choice.
#[cfg(test)]
pub(crate) async fn rules_ready_to_merge<'a>(
    gh: &dyn GitHub,
    rules: &'a [Issue],
    open: &'a [Pr],
) -> Outcome<Option<(&'a Issue, &'a Pr)>> {
    Ok(rules_prs(gh, rules, open)
        .await?
        .into_iter()
        .find(|(_, _, stands)| *stands == RulesPr::Mergeable)
        .map(|(task, pr, _)| (task, pr)))
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
    /// Green, but on a base its merge stales the mock-up's bundles of
    /// (`catch_up::staled`): nothing was merged. The PR takes its milestone
    /// at this commit in the harness's checkout, the generators named run
    /// (`branching::catch_up_locally`), and it lands once its checks are
    /// green again — never with a digest of the old file.
    Stale(BaseHead),
    /// Stale the same way, but the PR is asked for a repair
    /// (`harness:pr-fix`): nothing was done — the repair takes the base,
    /// never a checkout every tick for a refresh that already failed.
    Repairing,
}

/// Merges `pr` into its milestone branch and closes `task`:
/// `review-pending` and `needs-decision` off, a comment naming the PR, then
/// the close — what unblocks the tasks waiting on it.
///
/// **Never on red or pending checks**, whoever calls it: the router reads
/// the checks before routing here, but a decision to merge "as it is" and
/// a check that turned red between the read and the merge both reach this
/// one line, so the guard sits here, on the merge itself. **Never on a
/// base that stales its bundles** either ([`Attempt::Stale`]): green
/// checks ran on the base the PR last took, and a PR the catch-up left on
/// it — or skipped, its session running — would land a `builtFrom` digest
/// of the old file. A compare that cannot be read is said and merges: the
/// checks are the guard then, as before.
///
/// # Errors
/// [`harness_core::domain::Halt`] from the checks read, the base's head
/// read, the merge or any label write — GitHub refusing the merge is not
/// retried.
pub async fn run(gh: &dyn GitHub, task: &Issue, pr: &Pr, log: &Logbook) -> Outcome<Attempt> {
    if !gh.pr_checks_green(&pr.num).await? {
        let failing = gh.pr_failing_checks(&pr.num).await?;
        return Ok(if failing.is_empty() {
            Attempt::Pending
        } else {
            Attempt::Red(failing)
        });
    }
    match catch_up::staled(gh, &pr.head, &pr.base).await {
        Ok(rebuild) if !rebuild.is_empty() => {
            if pr.has(labels::PR_FIX) {
                return Ok(Attempt::Repairing);
            }
            if let Some(sha) = gh.branch_sha(&pr.base).await? {
                return Ok(Attempt::Stale(BaseHead { sha, rebuild }));
            }
        }
        Ok(_) => {}
        Err(halt) => log.debug(&format!(
            "{}: what it and {} changed since they parted cannot be read ({}) — merged on its \
             checks",
            pr.reference(),
            pr.base,
            halt.reason()
        )),
    }
    ensure_closes(gh, task, pr, log).await?;
    gh.merge_pr(&pr.num).await?;
    crate::common::close_merged::close(
        gh,
        task,
        &pr.base,
        &pr.reference(),
        "reviewed and every check green, merged by `milestone_merge`.",
    )
    .await?;
    // A landing PR decided on its milestone: the milestone closed, its
    // native milestone with it.
    crate::common::native_milestone::close_if_milestone(gh, task, log).await;
    Ok(Attempt::Merged)
}

/// Writes `Closes #<task>` on the PR's body before the harness merges it,
/// when the body lacks it (`delivery::with_closes`): the merged PR is then
/// the proof the milestone's landing reads (`delivery::every_task_merged`).
/// A body that cannot be read is said and merged as it is — the checks are
/// green, and the sweep (`common::close_merged`) proves it by its head.
///
/// # Errors
/// [`harness_core::domain::Halt`] from the body's write.
async fn ensure_closes(gh: &dyn GitHub, task: &Issue, pr: &Pr, log: &Logbook) -> Outcome<()> {
    let Ok(number) = pr.num.parse::<u64>() else {
        return Ok(());
    };
    match gh.issue(number).await {
        Ok(read) => {
            if let Some(body) = delivery::with_closes(&read.body, task.number) {
                gh.set_body(number, &body).await?;
                log.say(&format!(
                    "{}: its body lacked `Closes #{}` — written before the merge",
                    pr.reference(),
                    task.number
                ));
            }
        }
        Err(halt) => log.debug(&format!(
            "{}: its body cannot be read ({}) — merged without checking its Closes line",
            pr.reference(),
            halt.reason()
        )),
    }
    Ok(())
}

/// How many times `decide` is asked about one task's PR before a human is.
const REVIEW_DECISIONS: u32 = 2;

/// Asks for a decision on the issue that owns `pr`, once; answers whether
/// it asked now.
///
/// `owner` is the task whose PR it is, or the milestone whose landing PR it
/// is (`main_agent_merge`): one escalation for every PR, the landing ones
/// included (`ForgeDesMondes`' #423: a red landing PR repaired 21 to 25
/// times, ~$9, nothing ever escalating it). Its review stayed blocking, a
/// check red, or its branch conflicting, after every repair it gets:
/// `needs-decision` on the owner, with the PR named — the `decide` workflow
/// takes it at the next tick (merge as it is, repair once more, wait, or
/// cut the task; a PR into the integration branch is never merged nor cut
/// by a decision) and tells the human what it decided.
///
/// A decided `wait` is never a round, and never what hands the PR to a
/// human while it can still settle ([`waits_settle`], `ForgeDesMondes`'
/// #569 and #580, 2026-10-25: both sent to a human on the waits' cap
/// minutes before what they waited on landed). Each issue a decided wait
/// named is read against the PR's base (`contest::landed_on_base`, with
/// `integration`, the integration branch): one whose work has not reached
/// it — still open, or landed under another milestone that has not, or
/// whose base has not caught up — asks nothing, the wait stands
/// ([`Escalation::Waits`]); once every one landed there (closed or
/// `harness:waiting-merge`), the PR is resumed instead
/// (`contest::resume_decided`: its repairs counted anew, one repair
/// asked, [`Escalation::Resumed`]), and the question is asked again at the
/// same round only if those repairs fail too. A wait that cannot settle by
/// itself — on an issue a person holds (`contest::wait_stuck`), one cut
/// (`contest::was_cut`: closed as not planned, as a cut decision does, or
/// built under a milestone that was), or one built under a milestone that
/// waits on the PR's own — hands the PR to a human at once, told which.
/// Past `REVIEW_DECISIONS` rounds, or [`contest::MAX_WAITS`] decided waits
/// on the PR (`comments`, its comments as read) every one of which landed,
/// the PR is a human's too, told why and what was decided — the repairs
/// and waits counted from the PR's own markers, and the last decision
/// quoted. **An owner's answer to the question is never asked over**
/// (`ForgeDesMondes`' #157, 2026-10-20: the owner answered, `decide` paid
/// nothing, and "still blocked after 2 repairs" posed the label again nine
/// times in 1 h 40): the PR is the owner's until the answer is taken off.
///
/// Answers what it did ([`Escalation`]): the callers journal a resume
/// ([`journal`]), which happens here and nowhere else.
///
/// # Errors
/// [`harness_core::domain::Halt`] from a read of an issue a wait named or
/// its milestone, or from the label or the comment write.
pub async fn escalate(
    gh: &dyn GitHub,
    owner: &Issue,
    pr: &Pr,
    comments: &str,
    integration: &str,
) -> Outcome<Escalation> {
    let task = owner;
    if task.has(labels::NEEDS_DECISION) || task.has(labels::HUMAN) {
        return Ok(Escalation::Not);
    }
    let why = format!(
        "PR {} still does not merge after {} repair attempts — its review blocks \
         it, a check stays red, or its branch conflicts with {}.",
        pr.reference(),
        crate::common::review::MAX_FIXES,
        if pr.base.is_empty() {
            "its base".to_string()
        } else {
            format!("`{}`", pr.base)
        }
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
        .and_then(|text| crate::common::decision::Question::parse(text))
        .filter(|q| q.kind.as_deref() == Some("review"));
    if asked
        .as_ref()
        .is_some_and(crate::common::decision::Question::decided_by_owner)
    {
        return Ok(Escalation::Not);
    }
    let lost = match waits_settle(gh, pr, comments, integration).await? {
        Settle::Stands(on) => return Ok(Escalation::Waits(on)),
        Settle::Resumed(on) => return Ok(Escalation::Resumed(on)),
        Settle::Lost(because) => Some(because),
        Settle::Nothing => None,
    };
    let last_verdict = asked.as_ref().and_then(|q| q.verdict.clone());
    // A wait decided is not a repair bought (ForgeDesMondes' #486,
    // 2026-10-24: two "wait on #517" verdicts read as two rounds, and a
    // human was told the harness "decided 2 times to repair").
    let waited = last_verdict
        .as_deref()
        .is_some_and(|verdict| verdict.trim_start().starts_with("wait"));
    let round = asked.as_ref().map_or(1, |q| {
        if waited {
            q.round
        } else {
            q.round.saturating_add(1)
        }
    });
    let waits = contest::waits_decided(comments);
    // Each decision that says "repair" buys two more repairs: past
    // `REVIEW_DECISIONS` of them, the PR is a human's, not a third paid
    // cycle (ForgeDesMondes' #312, 2026-10-20: repair, decide, repair,
    // decide) — and so is one the harness keeps deciding to wait on.
    if lost.is_some() || round > REVIEW_DECISIONS || waits >= contest::MAX_WAITS {
        let repairs = comments.matches(review::DECIDED_MARKER).count();
        let because = match lost {
            Some(because) => because,
            None if waits >= contest::MAX_WAITS => "Every issue its waits named has landed, and \
                 the repairs since still fail."
                .to_string(),
            None => "The repairs it decided did not settle it.".to_string(),
        };
        gh.post_issue_comment(
            task.number,
            &format!(
                "{why} {because} The harness decided {repairs} repair(s) and {waits} wait(s) on \
                 it (last decision: {}); it stops spending on it. A human decides: repair it by \
                 hand, merge it as it is, or cut the task — then remove `{}`.",
                last_verdict.as_deref().unwrap_or("none"),
                labels::HUMAN
            ),
        )
        .await?;
        gh.add_label(task.number, labels::HUMAN).await?;
        return Ok(Escalation::Asked);
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
            "{why} The harness decides at the next tick — {} — and says what it decided; \
             answer under `## Decision` first to decide it yourself.",
            if task.has(labels::MILESTONE) {
                "repair it once more, or wait: a landing PR is never merged red nor cut"
            } else {
                "repair it once more, merge it as it is, wait, or cut the task"
            }
        ),
    )
    .await?;
    gh.add_label(task.number, labels::NEEDS_DECISION).await?;
    Ok(Escalation::Asked)
}

/// What [`escalate`] did with a PR past its repairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Escalation {
    /// A decision, or a human, was asked now.
    Asked,
    /// Every issue its decided waits named landed on its base: the PR was
    /// resumed on them now, one repair asked.
    Resumed(Vec<u64>),
    /// A decided wait on this issue still stands: nothing asked.
    Waits(u64),
    /// Nothing asked: asked already, a human's, or the owner's answer
    /// stands.
    Not,
}

impl Escalation {
    /// Whether a decision or a human was asked now.
    #[must_use]
    pub const fn asked(&self) -> bool {
        matches!(self, Self::Asked)
    }

    /// What became of the escalation of a PR owned by `owner`, said for
    /// the journal.
    #[must_use]
    pub fn outcome(&self, owner: u64) -> String {
        match self {
            Self::Asked => format!("#{owner} escalated"),
            Self::Resumed(_) => "resumed on its settled waits".to_string(),
            Self::Waits(on) => format!("its decided wait on #{on} stands"),
            Self::Not => format!("#{owner} asked already"),
        }
    }

    /// The journal line of a resume on `pr`, `None` for anything else.
    #[must_use]
    pub fn resumed_line(&self, pr: &Pr) -> Option<String> {
        let Self::Resumed(on) = self else {
            return None;
        };
        let named = on
            .iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "watch: decided wait on {named} landed -> PR {} resumed, its repairs counted anew",
            pr.reference()
        ))
    }
}

/// Journals what [`escalate`] did on `pr` when it resumed it; answers
/// whether it asked.
#[must_use]
pub fn journal(log: &Logbook, pr: &Pr, escalation: &Escalation) -> bool {
    if let Some(line) = escalation.resumed_line(pr) {
        log.say(&line);
    }
    escalation.asked()
}

/// Where the decided waits on a PR stand, as [`escalate`] reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Settle {
    /// No decided wait left uncleared on the PR.
    Nothing,
    /// The work of this issue a wait named has not reached the PR's base:
    /// the wait stands, nothing is asked.
    Stands(u64),
    /// Every issue the waits named landed on the PR's base: the PR was
    /// resumed on them now.
    Resumed(Vec<u64>),
    /// A wait that cannot settle by itself, said for the human.
    Lost(String),
}

/// Reads every issue a decided wait named on `pr` and no resumed marker
/// cleared (`contest::waits_named`), against the PR's base
/// (`contest::landed_on_base`, everything the integration branch changed
/// as what it touched), and settles what can be.
///
/// One whose work has not reached the base is [`Settle::Stands`]; one a
/// person holds, one cut, or one under a milestone waiting on the PR's
/// own is [`Settle::Lost`] — the first met, said; when every one landed,
/// the PR is resumed on them (`contest::resume_decided`) and
/// [`Settle::Resumed`].
///
/// # Errors
/// [`harness_core::domain::Halt`] from an issue read, the comment or the
/// label.
async fn waits_settle(
    gh: &dyn GitHub,
    pr: &Pr,
    comments: &str,
    integration: &str,
) -> Outcome<Settle> {
    let named = contest::waits_named(comments);
    if named.is_empty() {
        return Ok(Settle::Nothing);
    }
    let mut lost = None;
    for &on in &named {
        let target = gh.issue(on).await?;
        if contest::wait_stuck(&target) {
            lost = lost.or_else(|| {
                Some(format!(
                    "It waits on #{on}, which a person holds (`{}`): that wait cannot settle by \
                     itself.",
                    labels::HUMAN
                ))
            });
            continue;
        }
        let home = contest::home_of(&target);
        match contest::landed_on_base(gh, pr, &target, home, |_| true, integration).await? {
            contest::Landing::Landed => {}
            contest::Landing::NotYet => return Ok(Settle::Stands(on)),
            contest::Landing::Cut(cut) => {
                lost = lost.or_else(|| {
                    Some(if cut == on {
                        format!(
                            "It waited on #{on}, which was cut: what it waited for will not come."
                        )
                    } else {
                        format!(
                            "It waited on #{on}, built under #{cut}, which was cut: what it \
                             waited for will not come."
                        )
                    })
                });
            }
            contest::Landing::Cycle(milestone) => {
                lost = lost.or_else(|| {
                    Some(format!(
                        "It waits on #{on}, built under #{milestone}, which waits on this PR's \
                         own milestone: it cannot land before this PR does."
                    ))
                });
            }
        }
    }
    if let Some(because) = lost {
        return Ok(Settle::Lost(because));
    }
    contest::resume_decided(gh, pr, &named).await?;
    Ok(Settle::Resumed(named))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use std::collections::HashMap;

    /// The integration branch of every board here.
    const INTEGRATION: &str = "main_agent";

    /// [`super::escalate`] on [`INTEGRATION`]: whether it asked.
    async fn escalate(gh: &dyn GitHub, owner: &Issue, pr: &Pr, comments: &str) -> Outcome<bool> {
        super::escalate(gh, owner, pr, comments, INTEGRATION)
            .await
            .map(|done| done.asked())
    }

    /// [`super::ready_to_merge`] on [`INTEGRATION`], journalling nowhere.
    async fn ready_to_merge<'a>(
        gh: &dyn GitHub,
        board: &'a [Issue],
        open: &'a [Pr],
        held: &[u64],
    ) -> Outcome<(Option<(&'a Issue, &'a Pr)>, Vec<(u64, String)>)> {
        super::ready_to_merge(gh, board, open, held, INTEGRATION, &Logbook::null()).await
    }

    /// [`super::exhausted_repairs`] on [`INTEGRATION`].
    async fn exhausted_repairs(
        gh: &dyn GitHub,
        to_fix: &[Pr],
        to_review: &[Pr],
        held: &[u64],
        log: &Logbook,
    ) -> Outcome<Vec<(u64, String)>> {
        super::exhausted_repairs(gh, to_fix, to_review, held, INTEGRATION, log).await
    }

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
            escalate(&gh, &stale, &pr("279", "feat/create-session"), "")
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
            escalate(&gh, &again, &pr("279", "feat/create-session"), "")
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
            escalate(&gh, &third, &pr("279", "feat/create-session"), "")
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
            !escalate(&gh, &held, &pr("279", "feat/create-session"), "")
                .await
                .expect("reads")
        );
        assert!(gh.writes().is_empty(), "gh.writes() should be empty");
    }

    /// Task #486's body once `decide` answered its review question.
    fn decided(round: u32, verdict: &str) -> Issue {
        let mut decided = task(486, &[labels::AGENT, labels::REVIEW_PENDING], "feat/scene");
        decided.body = format!(
            "{}\n## Decision\n\nkind: review\nblocks: PR #516 still does not merge\nround: \
             {round}\ndecided-by: ai\nverdict: {verdict}\n",
            decided.body
        );
        decided
    }

    /// The body `escalate` wrote, if it wrote one.
    fn body_written(gh: &FakeGitHub) -> Option<String> {
        gh.writes().iter().find_map(|w| match w {
            Wrote::Body(486, body) => Some(body.clone()),
            _ => None,
        })
    }

    #[tokio::test]
    async fn a_decided_wait_is_not_a_round() {
        let gh = FakeGitHub::default();
        let waited = decided(1, "wait — #517 settles the test first (4/5)");
        // The wait settled and resumed the PR, and its repairs since failed.
        let comments = format!(
            "{}\n{}",
            contest::waits_on_marker(517),
            contest::resumed_marker(517)
        );
        assert!(
            escalate(&gh, &waited, &pr("516", "feat/scene"), &comments)
                .await
                .expect("asked")
        );
        let body = body_written(&gh).expect("the question asked again");
        assert!(body.contains("round: 1"), "{body}");
        assert!(
            gh.writes()
                .contains(&Wrote::Label(486, labels::NEEDS_DECISION.to_string()))
        );
        assert!(
            !gh.writes()
                .contains(&Wrote::Label(486, labels::HUMAN.to_string()))
        );
        // Waited at round 2 too: still round 2, still a decision.
        let gh = FakeGitHub::default();
        let waited = decided(2, "wait — #520 first (3/5)");
        assert!(
            escalate(&gh, &waited, &pr("516", "feat/scene"), &comments)
                .await
                .expect("asked")
        );
        assert!(body_written(&gh).expect("asked").contains("round: 2"));
        assert!(
            !gh.writes()
                .contains(&Wrote::Label(486, labels::HUMAN.to_string()))
        );
    }

    #[tokio::test]
    async fn the_human_comment_counts_the_repairs_decided_not_the_rounds() {
        // Two waits (one before the deploy wrote no marker), then two
        // repairs decided: round 2 repaired, so a third round is a human's.
        let gh = FakeGitHub::default();
        let repaired = decided(2, "repair — fix the slug mapping (4/5)");
        let comments = format!(
            "{}\n{}\n{}\nrepair\n{}\nrepair\n",
            contest::waits_on_marker(517),
            contest::resumed_marker(517),
            review::DECIDED_MARKER,
            review::DECIDED_MARKER
        );
        assert!(
            escalate(&gh, &repaired, &pr("516", "feat/scene"), &comments)
                .await
                .expect("handed over")
        );
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::Label(486, labels::HUMAN.to_string())));
        let said = writes
            .iter()
            .find_map(|w| match w {
                Wrote::Comment(486, said) => Some(said.clone()),
                _ => None,
            })
            .expect("the human is told");
        assert!(said.contains("decided 2 repair(s) and 1 wait(s)"), "{said}");
        assert!(
            said.contains("The repairs it decided did not settle it."),
            "{said}"
        );
        assert!(
            said.contains("last decision: repair — fix the slug mapping (4/5)"),
            "{said}"
        );
    }

    #[tokio::test]
    async fn waits_past_their_cap_are_a_human_s() {
        let gh = FakeGitHub::default();
        let waited = decided(1, "wait — #520 first (3/5)");
        // Both waits landed and resumed the PR; its repairs since still fail.
        let comments = format!(
            "{}\n{}\n{}\n{}",
            contest::waits_on_marker(517),
            contest::resumed_marker(517),
            contest::waits_on_marker(520),
            contest::resumed_marker(520)
        );
        assert!(
            escalate(&gh, &waited, &pr("516", "feat/scene"), &comments)
                .await
                .expect("handed over")
        );
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::Label(486, labels::HUMAN.to_string())));
        assert!(!writes.contains(&Wrote::Label(486, labels::NEEDS_DECISION.to_string())));
        assert!(writes.iter().any(|w| matches!(
            w,
            Wrote::Comment(486, said) if said.contains("0 repair(s) and 2 wait(s)")
                && said.contains("Every issue its waits named has landed, and the repairs since \
                                  still fail.")
        )));
    }

    /// An issue a decided wait named, in the state the replay reads it.
    fn named(number: u64, state: &str, labels: &[&str]) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    /// PR #569's comments up to the escalation of 14:50 (2026-10-25): its
    /// contract wait on #570 resumed, two repairs, then the decided wait on
    /// #567.
    fn comments_569() -> String {
        format!(
            "{}\n{}\n{}\n{}\n{}\nThe harness decided this PR waits on #567.",
            contest::contract_wait_marker(570),
            contest::resumed_marker(570),
            review::FIX_MARKER,
            review::FIX_MARKER,
            contest::waits_on_marker(567)
        )
    }

    /// Task #568's body once `decide` answered "wait on #567".
    fn task_568() -> Issue {
        let mut decided = task(568, &[labels::AGENT, labels::READY], "feat/scene-board");
        decided.body.push_str(
            "\n## Decision\n\nkind: review\nblocks: PR #569 still does not merge\nround: 1\n\
             decided-by: ai\nverdict: wait, 4/5: PR #569 cannot merge until #567 lands\n",
        );
        decided
    }

    /// The milestone of every PR here (`milestone/52-socle`).
    const OWN: u64 = 52;

    /// [`named`], built under `home` (its native milestone).
    fn built_under(number: u64, state: &str, labels: &[&str], home: u64) -> Issue {
        Issue {
            milestone: Some(home),
            ..named(number, state, labels)
        }
    }

    #[tokio::test]
    async fn the_replay_of_569_resumes_once_567_landed_never_a_human_on_the_waits_cap() {
        let pr_569 = pr("569", "feat/scene-board");
        // 14:50: #567 merged into the PR's base one minute before — the PR
        // resumes, one repair, journalled.
        let gh = FakeGitHub {
            issues: vec![built_under(567, "closed", &[labels::AGENT], OWN)],
            ..FakeGitHub::default()
        };
        let done = super::escalate(&gh, &task_568(), &pr_569, &comments_569(), INTEGRATION)
            .await
            .expect("resumed");
        assert_eq!(done, Escalation::Resumed(vec![567]));
        assert_eq!(
            done.resumed_line(&pr_569).as_deref(),
            Some("watch: decided wait on #567 landed -> PR #569 resumed, its repairs counted anew")
        );
        let writes = gh.writes();
        let said = writes
            .iter()
            .find_map(|w| match w {
                Wrote::Comment(569, said) => Some(said.clone()),
                _ => None,
            })
            .expect("the PR told");
        assert!(said.starts_with(&contest::resumed_marker(567)), "{said}");
        assert!(said.contains("landed on its base (#567)"), "{said}");
        assert_eq!(writes[0], Wrote::Label(569, labels::PR_FIX.to_string()));
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::Label(568, _))),
            "neither a human nor a decision: {writes:?}"
        );
        // Its repairs are counted anew from there.
        let resumed = format!("{}\n{said}", comments_569());
        assert_eq!(review::repairs(&resumed), 0);
        // Those two repairs fail too: now a human's, told why.
        let failed = format!("{resumed}\n{}\n{}", review::FIX_MARKER, review::FIX_MARKER);
        let gh = FakeGitHub {
            issues: vec![built_under(567, "closed", &[labels::AGENT], OWN)],
            ..FakeGitHub::default()
        };
        assert!(
            escalate(&gh, &task_568(), &pr_569, &failed)
                .await
                .expect("handed over")
        );
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::Label(568, labels::HUMAN.to_string())));
        assert!(writes.iter().any(|w| matches!(
            w,
            Wrote::Comment(568, said) if said.contains("Every issue its waits named has landed")
                && said.contains("0 repair(s) and 2 wait(s)")
        )));
        assert!(!writes.iter().any(|w| matches!(w, Wrote::Comment(569, _))));
    }

    #[tokio::test]
    async fn the_replay_of_569_waits_while_567_is_merged_only_under_another_milestone() {
        // #567 built under #27: closed, it is merged into milestone/27,
        // not into PR #569's base — the wait stands, nothing paid.
        let pr_569 = pr("569", "feat/scene-board");
        let board = |milestone_27: &[&str], behind: Vec<String>| FakeGitHub {
            issues: vec![
                built_under(567, "closed", &[labels::AGENT], 27),
                named(27, "open", milestone_27),
            ],
            changed_files: HashMap::from([(
                "milestone/52-socle...main_agent".to_string(),
                Some(behind),
            )]),
            ..FakeGitHub::default()
        };
        let read = |gh: FakeGitHub| async move {
            let done = super::escalate(
                &gh,
                &task_568(),
                &pr("569", "feat/scene-board"),
                &comments_569(),
                INTEGRATION,
            )
            .await
            .expect("reads");
            (done, gh.writes())
        };
        let (done, writes) = read(board(&[labels::MILESTONE], Vec::new())).await;
        assert_eq!(done, Escalation::Waits(567), "#27 still building");
        assert!(writes.is_empty(), "{writes:?}");
        let (done, writes) = read(board(
            &[labels::MILESTONE, labels::WAITING_MERGE],
            vec!["crates/scene/src/lib.rs".to_string()],
        ))
        .await;
        assert_eq!(done, Escalation::Waits(567), "#27 landed, base behind");
        assert!(writes.is_empty(), "{writes:?}");
        let (done, writes) = read(board(
            &[labels::MILESTONE, labels::WAITING_MERGE],
            Vec::new(),
        ))
        .await;
        assert_eq!(
            done,
            Escalation::Resumed(vec![567]),
            "#27 landed, base caught up"
        );
        assert!(writes.contains(&Wrote::Label(569, labels::PR_FIX.to_string())));
        assert_eq!(Escalation::Waits(567).resumed_line(&pr_569), None);
    }

    #[tokio::test]
    async fn a_decided_wait_on_a_landed_milestone_resumes_and_is_not_held() {
        // `wait on #27`: #27 landed on the integration branch
        // (`waiting-merge`, open until promotion) — settled, not a wait
        // that stands until a human promotes `main_agent`.
        let comments = format!("{}\n{}", comments_569(), contest::waits_on_marker(27));
        let comments = comments.replace(&contest::waits_on_marker(567), "");
        let landed_27 = named(27, "open", &[labels::MILESTONE, labels::WAITING_MERGE]);
        let gh = FakeGitHub {
            issues: vec![landed_27],
            ..FakeGitHub::default()
        };
        let pr_569 = pr("569", "feat/scene-board");
        assert!(!contest::held(&gh, &pr_569, &comments).await.expect("read"));
        assert_eq!(
            super::escalate(&gh, &task_568(), &pr_569, &comments, INTEGRATION)
                .await
                .expect("resumed"),
            Escalation::Resumed(vec![27])
        );
        assert!(
            gh.writes()
                .contains(&Wrote::Label(569, labels::PR_FIX.to_string()))
        );
    }

    #[tokio::test]
    async fn a_wait_on_an_open_issue_asks_nothing_and_one_a_person_holds_is_a_human_s() {
        let pr_569 = pr("569", "feat/scene-board");
        // #567 still being built: the wait stands, nothing counted, nothing
        // asked.
        let gh = FakeGitHub {
            issues: vec![built_under(567, "open", &[labels::AGENT], OWN)],
            ..FakeGitHub::default()
        };
        assert_eq!(
            super::escalate(&gh, &task_568(), &pr_569, &comments_569(), INTEGRATION)
                .await
                .expect("reads"),
            Escalation::Waits(567)
        );
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
        // #567 stopped for a person: the wait cannot settle — a human's at
        // once, told which issue holds it.
        let gh = FakeGitHub {
            issues: vec![named(567, "open", &[labels::AGENT, labels::HUMAN])],
            ..FakeGitHub::default()
        };
        assert!(
            escalate(&gh, &task_568(), &pr_569, &comments_569())
                .await
                .expect("handed over")
        );
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::Label(568, labels::HUMAN.to_string())));
        assert!(writes.iter().any(|w| matches!(
            w,
            Wrote::Comment(568, said) if said.contains("It waits on #567, which a person holds")
        )));
        assert!(!writes.contains(&Wrote::Label(569, labels::PR_FIX.to_string())));
        // Two waits, one landed and one still open: the open one stands.
        let two = format!("{}\n{}", comments_569(), contest::waits_on_marker(571));
        let gh = FakeGitHub {
            issues: vec![
                built_under(567, "closed", &[], OWN),
                built_under(571, "open", &[labels::AGENT], OWN),
            ],
            ..FakeGitHub::default()
        };
        assert!(
            !escalate(&gh, &task_568(), &pr_569, &two)
                .await
                .expect("reads")
        );
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    }

    #[tokio::test]
    async fn a_wait_on_a_cut_issue_or_a_cycle_is_a_human_s_never_a_resume() {
        let pr_569 = pr("569", "feat/scene-board");
        let handed = |gh: &FakeGitHub, why: &str| {
            let writes = gh.writes();
            assert!(
                writes.contains(&Wrote::Label(568, labels::HUMAN.to_string())),
                "{writes:?}"
            );
            assert!(
                writes
                    .iter()
                    .any(|w| matches!(w, Wrote::Comment(568, said) if said.contains(why))),
                "{why}: {writes:?}"
            );
            assert!(
                !writes
                    .iter()
                    .any(|w| matches!(w, Wrote::Comment(569, _) | Wrote::Label(569, _))),
                "never resumed: {writes:?}"
            );
        };
        // Closed as not planned, or cut by a decision.
        let mut not_planned = built_under(567, "closed", &[labels::AGENT], OWN);
        not_planned.not_planned = true;
        let mut decided_cut = built_under(567, "closed", &[labels::AGENT], OWN);
        decided_cut.body = "## Decision\n\nkind: review\nblocks: x\nround: 1\ndecided-by: ai\n\
                            verdict: cut — out of scope (4/5)\n"
            .to_string();
        for cut in [not_planned, decided_cut] {
            let gh = FakeGitHub {
                issues: vec![cut],
                ..FakeGitHub::default()
            };
            assert!(
                escalate(&gh, &task_568(), &pr_569, &comments_569())
                    .await
                    .expect("handed over")
            );
            handed(
                &gh,
                "It waited on #567, which was cut: what it waited for will not come.",
            );
        }
        // Built under #27, which was cut.
        let mut cut_27 = named(27, "closed", &[labels::MILESTONE]);
        cut_27.not_planned = true;
        let gh = FakeGitHub {
            issues: vec![built_under(567, "closed", &[], 27), cut_27],
            ..FakeGitHub::default()
        };
        assert!(
            escalate(&gh, &task_568(), &pr_569, &comments_569())
                .await
                .expect("handed over")
        );
        handed(&gh, "It waited on #567, built under #27, which was cut");
        // Built under #27, which waits on #52, the PR's own milestone.
        let mut waits_on_own = named(27, "open", &[labels::MILESTONE]);
        waits_on_own.blocked_by = vec![named(OWN, "open", &[labels::MILESTONE])];
        let gh = FakeGitHub {
            issues: vec![built_under(567, "open", &[], 27), waits_on_own.clone()],
            subs: vec![(12, vec![waits_on_own])],
            ..FakeGitHub::default()
        };
        assert!(
            escalate(&gh, &task_568(), &pr_569, &comments_569())
                .await
                .expect("handed over")
        );
        handed(
            &gh,
            "built under #27, which waits on this PR's own milestone",
        );
    }

    #[test]
    fn only_a_resume_is_journalled_and_only_an_ask_counts_as_asked() {
        /// The journal's lines, kept.
        #[derive(Default)]
        struct Capture(std::cell::RefCell<Vec<String>>);
        impl harness_core::traces::Sink for Capture {
            fn emit(&self, line: &str) {
                self.0.borrow_mut().push(line.to_string());
            }
        }
        let pr_569 = pr("569", "feat/scene-board");
        let capture = std::rc::Rc::new(Capture::default());
        let log = Logbook::new(
            std::rc::Rc::clone(&capture) as std::rc::Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        let lines = &capture.0;
        assert!(journal(&log, &pr_569, &Escalation::Asked));
        assert!(!journal(
            &log,
            &pr_569,
            &Escalation::Resumed(vec![567, 571])
        ));
        assert!(!journal(&log, &pr_569, &Escalation::Waits(567)));
        assert!(!journal(&log, &pr_569, &Escalation::Not));
        let said = lines.borrow().join("\n");
        assert!(
            said.contains("decided wait on #567, #571 landed -> PR #569 resumed"),
            "{said}"
        );
        assert_eq!(said.matches("watch:").count(), 1, "{said}");
        for (done, told) in [
            (Escalation::Asked, "#568 escalated"),
            (
                Escalation::Resumed(vec![567]),
                "resumed on its settled waits",
            ),
            (Escalation::Waits(567), "its decided wait on #567 stands"),
            (Escalation::Not, "#568 asked already"),
        ] {
            assert_eq!(done.outcome(568), told);
        }
    }

    #[tokio::test]
    async fn the_replay_of_580_resumes_once_582_landed() {
        // PR #580: its contract wait on #581 resumed, two repairs, then
        // "wait on #582" — and #582 landed at 16:5x.
        let comments = format!(
            "{}\n{}\n{}\n{}\n{}",
            contest::contract_wait_marker(581),
            contest::resumed_marker(581),
            review::FIX_MARKER,
            review::FIX_MARKER,
            contest::waits_on_marker(582)
        );
        let mut owner = task(
            576,
            &[labels::AGENT, labels::READY],
            "feat/campaign-list-micro-ui",
        );
        owner.body.push_str(
            "\n## Decision\n\nkind: review\nblocks: PR #580\nround: 1\ndecided-by: ai\n\
             verdict: wait for #582 (campaign.listCampaigns v2)\n",
        );
        let gh = FakeGitHub {
            issues: vec![built_under(582, "closed", &[labels::AGENT], OWN)],
            ..FakeGitHub::default()
        };
        assert!(
            !escalate(
                &gh,
                &owner,
                &pr("580", "feat/campaign-list-micro-ui"),
                &comments
            )
            .await
            .expect("resumed")
        );
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::Label(580, labels::PR_FIX.to_string())));
        assert!(writes.iter().any(|w| matches!(
            w,
            Wrote::Comment(580, said) if said.starts_with(&contest::resumed_marker(582))
        )));
        assert!(!writes.contains(&Wrote::Label(576, labels::HUMAN.to_string())));
        // Had #582 been closed as not planned: no resume, no repair burnt —
        // a human's, told it was cut.
        let mut cut = built_under(582, "closed", &[labels::AGENT], OWN);
        cut.not_planned = true;
        let gh = FakeGitHub {
            issues: vec![cut],
            ..FakeGitHub::default()
        };
        assert!(
            escalate(
                &gh,
                &owner,
                &pr("580", "feat/campaign-list-micro-ui"),
                &comments
            )
            .await
            .expect("handed over")
        );
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::Label(576, labels::HUMAN.to_string())));
        assert!(writes.iter().any(|w| matches!(
            w,
            Wrote::Comment(576, said) if said.contains("#582, which was cut")
        )));
        assert!(
            !writes
                .iter()
                .any(|w| matches!(w, Wrote::Label(580, _) | Wrote::Comment(580, _)))
        );
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
        let (found, escalated) = ready_to_merge(&gh, &board, &open, &[]).await.expect("read");
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
        let (_, escalated) = ready_to_merge(&gh, &board, &open, &[]).await.expect("read");
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
        let (found, escalated) = ready_to_merge(&gh, &board, &open, &[]).await.expect("read");
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
        let (found, escalated) = ready_to_merge(&gh, &board, &open, &[]).await.expect("read");
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
        let (found, escalated) = ready_to_merge(&gh, &board[..1], &open[..1], &[])
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
        run(&gh, &bare, &pr("77", "feat/save"), &Logbook::null())
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
        escalate(&gh, &stuck, &pr("68", "chore/rust"), "")
            .await
            .expect("escalated");
        assert!(
            gh.writes()
                .contains(&Wrote::Label(64, labels::NEEDS_DECISION.to_string()))
        );
        let decided = task(64, &[labels::AGENT, labels::NEEDS_DECISION], "chore/rust");
        let again = FakeGitHub::default();
        escalate(&again, &decided, &pr("68", "chore/rust"), "")
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
        let came_to = run(&gh, &waiting(), &pr("68", "chore/rust"), &Logbook::null())
            .await
            .expect("merged");
        assert_eq!(came_to, Attempt::Merged);
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::MergedPr("68".to_string())));
        assert!(writes.contains(&Wrote::Unlabelled(64, labels::REVIEW_PENDING.to_string())));
        assert!(writes.contains(&Wrote::Unlabelled(64, labels::NEEDS_DECISION.to_string())));
        assert!(writes.contains(&Wrote::Closed(64)));
    }

    #[tokio::test]
    async fn a_landing_merged_on_a_decision_closes_its_native_milestone_and_a_task_s_does_not() {
        let gh = FakeGitHub {
            pr_checks: HashMap::from([("68".to_string(), true)]),
            ..FakeGitHub::default()
        };
        run(&gh, &waiting(), &pr("68", "chore/rust"), &Logbook::null())
            .await
            .expect("merged");
        assert!(
            !gh.writes()
                .iter()
                .any(|w| matches!(w, Wrote::NativeMilestone(..))),
            "a task has no native milestone"
        );
        let mut milestone = task(6, &[labels::MILESTONE], "milestone/6-media");
        milestone.title = "Media".to_string();
        run(
            &gh,
            &milestone,
            &pr("68", "milestone/6-media"),
            &Logbook::null(),
        )
        .await
        .expect("merged");
        let writes = gh.writes();
        assert_eq!(
            writes.last(),
            Some(&Wrote::NativeMilestone(6, "Media".to_string(), false)),
            "{writes:?}"
        );
        assert!(writes.contains(&Wrote::Closed(6)));
    }

    #[tokio::test]
    async fn a_green_pr_on_a_base_that_stales_its_bundles_is_refreshed_never_landed() {
        // Its milestone took a new `server.js` since the PR last took it.
        let stale = || FakeGitHub {
            pr_checks: HashMap::from([("68".to_string(), true)]),
            changed_files: HashMap::from([(
                "chore/rust...milestone/52-socle".to_string(),
                Some(vec!["mock/src/server.js".to_string()]),
            )]),
            branch_shas: HashMap::from([(
                "milestone/52-socle".to_string(),
                Some("m1".to_string()),
            )]),
            ..FakeGitHub::default()
        };
        let gh = stale();
        let came_to = run(&gh, &waiting(), &pr("68", "chore/rust"), &Logbook::null())
            .await
            .expect("read");
        assert_eq!(
            came_to,
            Attempt::Stale(BaseHead {
                sha: "m1".to_string(),
                rebuild: vec![harness_architecture::generated::Regenerator::MockBuild],
            })
        );
        assert!(gh.writes().is_empty(), "nothing merged: {:?}", gh.writes());
        // Under repair: the repair takes the base, the train waits.
        let mut repairing = pr("68", "chore/rust");
        repairing.labels = vec![labels::PR_FIX.to_string()];
        let gh = stale();
        let came_to = run(&gh, &waiting(), &repairing, &Logbook::null())
            .await
            .expect("read");
        assert_eq!(came_to, Attempt::Repairing);
        assert!(gh.writes().is_empty(), "nothing merged: {:?}", gh.writes());
        // A base GitHub no longer knows: GitHub's merge answers for it.
        let gone = FakeGitHub {
            branch_shas: HashMap::from([("milestone/52-socle".to_string(), None)]),
            ..stale()
        };
        let came_to = run(&gone, &waiting(), &pr("68", "chore/rust"), &Logbook::null())
            .await
            .expect("merged");
        assert_eq!(came_to, Attempt::Merged);
        // A compare that cannot be read merges on its checks.
        let unread = FakeGitHub {
            broken_compares: vec!["chore/rust...milestone/52-socle".to_string()],
            ..stale()
        };
        let came_to = run(
            &unread,
            &waiting(),
            &pr("68", "chore/rust"),
            &Logbook::null(),
        )
        .await
        .expect("merged");
        assert_eq!(came_to, Attempt::Merged);
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
        assert!(
            rules_candidates(&rules, &[rules_pr("7", 11)]).is_empty(),
            "rules_candidates(&rules, &[rules_pr('7', 11)]) should be empty"
        );
        assert_eq!(rules_branch(9), "rules/9");
    }

    #[tokio::test]
    async fn a_rules_pr_its_reader_judged_faithful_is_taken_once_green() {
        // ForgeDesMondes #501: the reader judged the test faithful but could
        // no longer merge it (a session's `gh pr merge` is denied); the
        // train takes it on the verdict.
        let rules = vec![rules_task(9, "open")];
        let open = [rules_pr("501", 9)];
        let verdict = |word: &str| format!("Every test faithful.\n{}{word} -->", review::FIDELITY);
        let faithful = FakeGitHub {
            pr_comment_bodies: vec![("501".to_string(), verdict("faithful"))],
            pr_checks: HashMap::from([("501".to_string(), true)]),
            ..FakeGitHub::default()
        };
        assert_eq!(
            rules_ready_to_merge(&faithful, &rules, &open)
                .await
                .expect("read")
                .map(|(t, p)| (t.number, p.num.as_str())),
            Some((9, "501"))
        );
        let unfaithful = FakeGitHub {
            pr_comment_bodies: vec![("501".to_string(), verdict("unfaithful"))],
            pr_checks: HashMap::from([("501".to_string(), true)]),
            ..FakeGitHub::default()
        };
        assert!(
            rules_ready_to_merge(&unfaithful, &rules, &open)
                .await
                .expect("read")
                .is_none(),
            "an unfaithful verdict merges nothing"
        );
        let pending = FakeGitHub {
            pr_comment_bodies: vec![("501".to_string(), verdict("faithful"))],
            pr_checks: HashMap::from([("501".to_string(), false)]),
            failing_checks: HashMap::from([("501".to_string(), Vec::new())]),
            ..FakeGitHub::default()
        };
        assert!(
            rules_ready_to_merge(&pending, &rules, &open)
                .await
                .expect("read")
                .is_none(),
            "faithful but its checks still run"
        );
    }

    #[tokio::test]
    async fn a_faithful_rules_pr_with_no_check_at_all_is_held_within_a_bound() {
        // No check row at all (the workflow never triggered): not green,
        // nothing failed — it reads as checking, which the watch holds
        // within a bound (`routing::RULES_HELD_READS`), never forever.
        let rules = vec![rules_task(9, "open")];
        let open = [rules_pr("501", 9)];
        let gh = FakeGitHub {
            pr_comment_bodies: vec![(
                "501".to_string(),
                format!("{}faithful -->", review::FIDELITY),
            )],
            pr_checks: HashMap::from([("501".to_string(), false)]),
            failing_checks: HashMap::from([("501".to_string(), Vec::new())]),
            ..FakeGitHub::default()
        };
        let stands: Vec<(u64, RulesPr)> = rules_prs(&gh, &rules, &open)
            .await
            .expect("read")
            .into_iter()
            .map(|(task, _, stands)| (task.number, stands))
            .collect();
        assert_eq!(
            stands,
            [(9, RulesPr::Checking)],
            "the bound is the watch's: routing::bound_rules_on_pr"
        );
    }

    #[tokio::test]
    async fn a_faithful_rules_pr_with_checks_running_is_checking_and_one_red_is_red() {
        let rules = vec![
            rules_task(9, "open"),
            rules_task(10, "open"),
            rules_task(11, "open"),
            rules_task(12, "open"),
            rules_task(13, "open"),
        ];
        let open = [
            rules_pr("501", 9),
            rules_pr("502", 10),
            rules_pr("503", 11),
            rules_pr("504", 12),
            rules_pr("505", 13),
        ];
        let faithful = format!("{}faithful -->", review::FIDELITY);
        let gh = FakeGitHub {
            pr_comment_bodies: vec![
                ("501".to_string(), faithful.clone()),
                ("502".to_string(), faithful.clone()),
                ("503".to_string(), faithful.clone()),
                ("504".to_string(), faithful),
                (
                    "505".to_string(),
                    format!("{}unfaithful -->", review::FIDELITY),
                ),
            ],
            pr_checks: HashMap::from([
                ("501".to_string(), true),
                ("502".to_string(), false),
                ("503".to_string(), false),
                ("504".to_string(), true),
            ]),
            failing_checks: HashMap::from([
                ("502".to_string(), Vec::new()),
                ("503".to_string(), vec!["rust — FAILURE".to_string()]),
            ]),
            pr_mergeable: HashMap::from([("504".to_string(), Some(false))]),
            ..FakeGitHub::default()
        };
        let stands: Vec<(u64, RulesPr)> = rules_prs(&gh, &rules, &open)
            .await
            .expect("read")
            .into_iter()
            .map(|(task, _, stands)| (task.number, stands))
            .collect();
        assert_eq!(
            stands,
            [
                (9, RulesPr::Mergeable),
                (10, RulesPr::Checking),
                (11, RulesPr::Red),
                (12, RulesPr::Red),
                (13, RulesPr::Unjudged),
            ],
            "faithful and conflicting is red: attempted again to rebase"
        );
        assert_eq!(
            rules_ready_to_merge(&gh, &rules, &open)
                .await
                .expect("read")
                .map(|(t, _)| t.number),
            Some(9)
        );
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
            failing_checks: HashMap::from([("231".to_string(), Vec::new())]),
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
        let came_to = run(&red, &waiting(), &pr("68", "chore/rust"), &Logbook::null())
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
        let came_to = run(
            &pending,
            &waiting(),
            &pr("68", "chore/rust"),
            &Logbook::null(),
        )
        .await
        .expect("waits, not failed");
        assert_eq!(came_to, Attempt::Pending);
        assert_eq!(
            pending.writes(),
            [] as [Wrote; 0],
            "nothing written while pending"
        );
    }

    #[tokio::test]
    async fn a_repair_labelled_pr_past_its_cap_with_no_other_path_is_escalated_on_its_task() {
        // ForgeDesMondes #498's shape: a task PR labelled for repair, never
        // `harness:to-review`, red past its two repairs.
        let fixes = format!("{}\n", review::FIX_MARKER).repeat(review::MAX_FIXES);
        let red = |num: &str| (num.to_string(), vec!["rust — FAILURE".to_string()]);
        let gh = FakeGitHub {
            pr_comment_bodies: ["98", "97", "96", "95", "94", "93", "90"]
                .iter()
                .map(|num| ((*num).to_string(), fixes.clone()))
                .chain([("92".to_string(), format!("{}\n", review::FIX_MARKER))])
                .collect(),
            failing_checks: HashMap::from([
                red("98"),
                red("97"),
                red("96"),
                red("95"),
                red("92"),
                red("90"),
                ("94".to_string(), Vec::new()),
                ("93".to_string(), Vec::new()),
            ]),
            pr_mergeable: HashMap::from([
                ("94".to_string(), Some(false)),
                ("93".to_string(), Some(true)),
            ]),
            issues: vec![
                Issue {
                    number: 98,
                    body: "Fixes #498".to_string(),
                    ..Issue::default()
                },
                Issue {
                    number: 94,
                    body: "Closes #494".to_string(),
                    ..Issue::default()
                },
                Issue {
                    number: 93,
                    body: "Closes #493".to_string(),
                    ..Issue::default()
                },
                Issue {
                    number: 95,
                    body: "no closing line".to_string(),
                    ..Issue::default()
                },
                task(498, &[labels::AGENT], "feat/a"),
                task(494, &[labels::AGENT], "feat/b"),
                task(493, &[labels::AGENT], "feat/c"),
            ],
            ..FakeGitHub::default()
        };
        let to_fix = [
            pr("98", "feat/a"),
            pr("97", "feat/under-review"),
            Pr {
                num: "96".to_string(),
                head: "milestone/22-fiche".to_string(),
                ..Pr::default()
            },
            pr("95", "feat/orphan"),
            pr("94", "feat/b"),
            pr("93", "feat/c"),
            pr("92", "feat/one-repair"),
            pr("90", "rules/12"),
            pr("x", "feat/unnumbered"),
        ];
        let escalated = exhausted_repairs(
            &gh,
            &to_fix,
            &[pr("97", "feat/under-review")],
            &[],
            &Logbook::null(),
        )
        .await
        .expect("read");
        assert_eq!(
            escalated,
            [(498, "#98".to_string()), (494, "#94".to_string())],
            "red or conflicting past the cap; not under review, not a landing PR, not \
             green (#93), not within the cap (#92), not orphaned (#95), not a \
             rules' PR (#90)"
        );
        assert!(
            gh.writes()
                .contains(&Wrote::Label(498, labels::NEEDS_DECISION.to_string()))
        );
        assert!(
            !gh.writes()
                .iter()
                .any(|w| matches!(w, Wrote::Label(493 | 495, _)))
        );
    }

    #[tokio::test]
    async fn an_owner_s_answer_to_the_review_question_is_never_asked_over() {
        // ForgeDesMondes #157, 2026-10-20: the owner answered, decide paid
        // nothing, and "still blocked after 2 repairs" posed the label again
        // nine times in 1 h 40.
        let mut answered = task(157, &[labels::AGENT], "feat/create-session");
        answered.body.push_str(
            "\n## Decision\n\nkind: review\nblocks: PR #279 still does not merge\nround: 1\n\
             decided-by: owner\nverdict: I repair it by hand\n",
        );
        let gh = FakeGitHub::default();
        assert!(
            !escalate(&gh, &answered, &pr("279", "feat/create-session"), "")
                .await
                .expect("read")
        );
        assert_eq!(gh.writes(), [] as [Wrote; 0]);
    }

    #[tokio::test]
    async fn a_landing_pr_asks_its_milestone_repair_or_wait_never_merge_or_cut() {
        let milestone = Issue {
            number: 22,
            state: "open".to_string(),
            title: "Fiche entite".to_string(),
            labels: vec![labels::MILESTONE.to_string()],
            body: "the milestone".to_string(),
            ..Issue::default()
        };
        let landing = Pr {
            num: "423".to_string(),
            head: "milestone/22-fiche-entite".to_string(),
            base: "main_agent".to_string(),
            ..Pr::default()
        };
        let gh = FakeGitHub::default();
        assert!(
            escalate(&gh, &milestone, &landing, "")
                .await
                .expect("asked")
        );
        let writes = gh.writes();
        assert!(writes.iter().any(|w| matches!(
            w,
            Wrote::Body(22, body) if body.contains("kind: review") && body.contains("`main_agent`")
        )));
        assert!(writes.iter().any(|w| matches!(
            w,
            Wrote::Comment(22, said) if said.contains("never merged red nor cut")
        )));
        assert!(writes.contains(&Wrote::Label(22, labels::NEEDS_DECISION.to_string())));
    }

    #[tokio::test]
    async fn a_pr_under_repair_on_a_lane_is_neither_escalated_nor_merged() {
        use crate::common::review;
        let board = vec![task(
            272,
            &[labels::AGENT, labels::REVIEW_PENDING],
            "feat/upload",
        )];
        let open = [pr("90", "feat/upload")];
        let fixes = format!("{}\n", review::FIX_MARKER).repeat(review::MAX_FIXES);
        let gh = FakeGitHub {
            pr_comment_bodies: vec![(
                "90".to_string(),
                format!("{fixes}{}\n{}\n", review::MARKER, review::CLEAN),
            )],
            pr_checks: HashMap::from([("90".to_string(), false)]),
            failing_checks: HashMap::from([("90".to_string(), vec!["rust — FAILURE".to_string()])]),
            ..FakeGitHub::default()
        };
        let (found, escalated) = ready_to_merge(&gh, &board, &open, &[90])
            .await
            .expect("read");
        assert!(found.is_none() && escalated.is_empty());
        assert_eq!(gh.writes(), [] as [Wrote; 0]);
    }

    #[tokio::test]
    async fn a_pr_body_without_its_closes_line_gets_it_before_the_merge() {
        let gh = FakeGitHub {
            pr_checks: HashMap::from([("68".to_string(), true)]),
            issues: vec![Issue {
                number: 68,
                body: "## Plan\n- the list".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        };
        run(&gh, &waiting(), &pr("68", "chore/rust"), &Logbook::null())
            .await
            .expect("merged");
        let writes = gh.writes();
        let body_at = writes
            .iter()
            .position(|w| *w == Wrote::Body(68, "## Plan\n- the list\n\nCloses #64\n".to_string()))
            .expect("the line is written");
        let merged_at = writes
            .iter()
            .position(|w| *w == Wrote::MergedPr("68".to_string()))
            .expect("merged");
        assert!(body_at < merged_at, "{writes:?}");
        // A body that carries it is left as it is.
        let gh = FakeGitHub {
            pr_checks: HashMap::from([("68".to_string(), true)]),
            issues: vec![Issue {
                number: 68,
                body: "Closes #64".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        };
        run(&gh, &waiting(), &pr("68", "chore/rust"), &Logbook::null())
            .await
            .expect("merged");
        assert!(!gh.writes().iter().any(|w| matches!(w, Wrote::Body(68, _))));
    }

    /// PR #547's comments when decide #1 started: two repairs, the review
    /// still blocking.
    fn exhausted_547() -> String {
        format!(
            "{}{}\n{}\n",
            format!("{}\n", review::FIX_MARKER).repeat(review::MAX_FIXES),
            review::MARKER,
            review::BLOCKING
        )
    }

    /// The board and the GitHub of `ForgeDesMondes`' #542 (PR #547), with
    /// the PR's comments as `comments`.
    fn incident(comments: &str) -> (Vec<Issue>, Vec<Pr>, FakeGitHub) {
        let board = vec![task(
            542,
            &[labels::AGENT, labels::REVIEW_PENDING],
            "feat/journal",
        )];
        let open = vec![pr("547", "feat/journal")];
        let gh = FakeGitHub {
            pr_comment_bodies: vec![("547".to_string(), comments.to_string())],
            pr_checks: HashMap::from([("547".to_string(), false)]),
            failing_checks: HashMap::from([(
                "547".to_string(),
                vec!["rust — FAILURE".to_string()],
            )]),
            issues: vec![
                Issue {
                    number: 547,
                    body: "Closes #542".to_string(),
                    ..Issue::default()
                },
                board[0].clone(),
            ],
            ..FakeGitHub::default()
        };
        (board, open, gh)
    }

    /// How many times a decision was asked on #542 — one decide each.
    fn asked(gh: &FakeGitHub) -> usize {
        gh.writes()
            .iter()
            .filter(|w| **w == Wrote::Label(542, labels::NEEDS_DECISION.to_string()))
            .count()
    }

    #[tokio::test]
    async fn the_duplicate_decide_of_542_is_one_decide() {
        // ForgeDesMondes' #542, PR #547, 2026-10-25: decide took the label
        // off before its session, the next tick saw the task blocked and
        // unasked, asked again, and a second decide paid the same "repair".
        let mut decides = 0;
        // Tick 1: two repairs, still blocked — the harness decides.
        let (board, open, gh) = incident(&exhausted_547());
        let (_, escalated) = ready_to_merge(&gh, &board, &open, &[]).await.expect("read");
        assert_eq!(escalated, [(542, "#547".to_string())]);
        decides += asked(&gh);
        // Tick 2: decide #1 runs on a lane keyed #542, the label taken off.
        let (board, open, gh) = incident(&exhausted_547());
        let (found, escalated) = ready_to_merge(&gh, &board, &open, &[542])
            .await
            .expect("read");
        assert!(found.is_none() && escalated.is_empty(), "{escalated:?}");
        let escalated = exhausted_repairs(&gh, &open, &[], &[542], &Logbook::null())
            .await
            .expect("read");
        assert!(escalated.is_empty(), "{escalated:?}");
        assert_eq!(gh.writes(), [] as [Wrote; 0]);
        decides += asked(&gh);
        // Tick 3: decide #1 decided to repair once more; its lane is free,
        // the repair not run yet — decided, repairing.
        let repairing = format!(
            "{}{}\nrepair once more\n",
            exhausted_547(),
            review::DECIDED_MARKER
        );
        let (board, open, gh) = incident(&repairing);
        let (_, escalated) = ready_to_merge(&gh, &board, &open, &[]).await.expect("read");
        assert!(escalated.is_empty(), "{escalated:?}");
        let escalated = exhausted_repairs(&gh, &open, &[], &[], &Logbook::null())
            .await
            .expect("read");
        assert!(escalated.is_empty(), "{escalated:?}");
        decides += asked(&gh);
        assert_eq!(decides, 1, "one decide on #542");
    }

    #[tokio::test]
    async fn a_task_a_lane_holds_is_escalated_by_no_path() {
        // Without the lane, both paths ask: the guard is the lane alone.
        let (board, open, gh) = incident(&exhausted_547());
        let (_, escalated) = ready_to_merge(&gh, &board, &open, &[]).await.expect("read");
        assert_eq!(escalated.len(), 1);
        let (_, open, gh) = incident(&exhausted_547());
        let escalated = exhausted_repairs(&gh, &open, &[], &[], &Logbook::null())
            .await
            .expect("read");
        assert_eq!(escalated, [(542, "#547".to_string())]);
        // A lane on another number holds nothing here.
        let (board, open, gh) = incident(&exhausted_547());
        let (_, escalated) = ready_to_merge(&gh, &board, &open, &[541])
            .await
            .expect("read");
        assert_eq!(escalated.len(), 1);
        let (_, open, gh) = incident(&exhausted_547());
        let escalated = exhausted_repairs(&gh, &open, &[], &[541], &Logbook::null())
            .await
            .expect("read");
        assert_eq!(escalated.len(), 1);
    }
}
