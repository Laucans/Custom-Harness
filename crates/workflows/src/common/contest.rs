//! A pull request that waits, and a rules contest followed through — read
//! from the board alone, so a watch restarted mid-contest reads the same
//! waits back.
//!
//! **A PR waits when the board says it waits on an issue** ([`waiting`]):
//! an open `Rules contested: PR #<n>` issue (the title the rules' gate
//! writes, nothing else written); its latest contest closed and not yet
//! followed through — no `<!-- harness-resumed: #N -->` marker for it, its
//! base still to catch up on the settled rules; or a
//! `<!-- harness-waits-on: #N -->` marker a decided `wait` left on the PR
//! while `#N` still blocks. A waiting PR gets no repair, no escalation, no
//! decision and no merge; the review still runs. A resumed marker clears
//! the wait on `#N`.
//!
//! **A settled contest is followed through for free** ([`settled`],
//! [`resume`]): once its issue is closed and the branch the PR lands on
//! holds the settled rules, the harness deletes `CONTEST.md` where the
//! contest wrote it — on the PR's branch when its own diff added it, on a
//! milestone branch a contest about that branch named, when the file names
//! no rule beyond that contest's — brings a PR with no lift of its own up
//! to date with its base, so its checks run on the settled rules, and marks
//! the PR resumed, which counts its repairs anew (`review::repairs`). Only
//! a settled contest resets the repairs: a decided wait does not.

use harness_core::domain::{Issue, Outcome, Pr};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook};

use crate::common::{branching, dossier};
use crate::dev_loop::data::tasks;
use crate::rules::data::request::CONTESTED;

/// The file a development writes at the root to contest a rule's test.
pub const FILE: &str = "CONTEST.md";

/// What opens the marker a decided `wait` leaves on the PR, followed by
/// the issue's number and ` -->`.
pub const WAITS_ON: &str = "<!-- harness-waits-on: #";

/// What opens the marker that clears a wait on an issue, followed by its
/// number and ` -->`.
pub const RESUMED: &str = "<!-- harness-resumed: #";

/// How many decided waits one PR gets before a human decides: a decision
/// that keeps answering "wait" is not left to loop.
pub const MAX_WAITS: usize = 2;

/// The marker a decided `wait` on `on` leaves on the PR.
#[must_use]
pub fn waits_on_marker(on: u64) -> String {
    format!("{WAITS_ON}{on} -->")
}

/// The marker that clears a wait on `on`.
#[must_use]
pub fn resumed_marker(on: u64) -> String {
    format!("{RESUMED}{on} -->")
}

/// Every `(position, number)` a marker opened by `prefix` names, in order.
fn markers(comments: &str, prefix: &str) -> Vec<(usize, u64)> {
    comments
        .match_indices(prefix)
        .filter_map(|(at, _)| {
            let rest = comments.get(at.saturating_add(prefix.len())..)?;
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok().map(|number| (at, number))
        })
        .collect()
}

/// The issues a decided `wait` named on the PR and no resumed marker
/// cleared since, in the order first named.
#[must_use]
pub fn waits_named(comments: &str) -> Vec<u64> {
    let resumed = markers(comments, RESUMED);
    let mut found: Vec<u64> = Vec::new();
    for (at, on) in markers(comments, WAITS_ON) {
        let cleared = resumed
            .iter()
            .any(|(later, number)| *number == on && *later > at);
        if !cleared && !found.contains(&on) {
            found.push(on);
        }
    }
    found
}

/// How many times the harness decided the PR waits, in all.
#[must_use]
pub fn waits_decided(comments: &str) -> usize {
    markers(comments, WAITS_ON).len()
}

/// Whether the PR was marked resumed on `on` — what keeps a settled
/// contest from being followed through twice.
#[must_use]
pub fn resumed_on(comments: &str, on: u64) -> bool {
    markers(comments, RESUMED)
        .iter()
        .any(|(_, number)| *number == on)
}

/// The title the rules' gate gives the contest of PR `pr`.
#[must_use]
pub fn title(pr: u64) -> String {
    format!("{CONTESTED}: PR #{pr}")
}

/// The PR a contest's title names — [`title`]'s, or with a ` (k)` suffix
/// for a later contest of the same PR.
#[must_use]
pub fn contested_pr(title: &str) -> Option<u64> {
    let rest = title
        .trim()
        .strip_prefix(CONTESTED)?
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix("PR #")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let after = rest.get(digits.len()..).unwrap_or_default();
    if !(after.is_empty() || after.starts_with(" (")) {
        return None;
    }
    digits.parse().ok().filter(|n| *n > 0)
}

/// The milestone a contest about a milestone branch names —
/// `Rules contested: R10 on milestone/28`, written when the branch itself
/// carries `CONTEST.md`.
#[must_use]
pub fn contested_milestone(title: &str) -> Option<u64> {
    let rest = title.trim().strip_prefix(CONTESTED)?;
    let (_, branch) = rest.rsplit_once(" on ")?;
    branching::milestone_number(branch.trim())
}

/// The open contest of PR `pr`, if one is open.
#[must_use]
pub fn open_contest(pr: u64, rules: &[Issue]) -> Option<&Issue> {
    rules
        .iter()
        .filter(|issue| issue.is_open())
        .find(|issue| contested_pr(&issue.title) == Some(pr))
}

/// The latest contest of PR `pr` among `closed`, when it is closed — the
/// one a resume follows through.
#[must_use]
pub fn latest_closed_contest(pr: u64, closed: &[Issue]) -> Option<u64> {
    closed
        .iter()
        .filter(|issue| !issue.is_open() && contested_pr(&issue.title) == Some(pr))
        .map(|issue| issue.number)
        .max()
}

/// What a PR waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrWait {
    /// Its own contest is open: the rules' task settles it first.
    Contest {
        /// The contest issue.
        issue: u64,
    },
    /// Its contest is closed, not followed through yet: its base takes the
    /// settled rules (its catch-up) before it resumes ([`settled`]).
    Settling {
        /// The contest issue, closed.
        issue: u64,
    },
    /// The harness decided it waits for this issue to land.
    Decided {
        /// The issue waited for.
        issue: u64,
    },
}

impl PrWait {
    /// How the journal names it.
    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Self::Contest { issue } => format!("contest #{issue}"),
            Self::Settling { issue } => {
                format!("contest #{issue} settled (its base takes the settled rules first)")
            }
            Self::Decided { issue } => format!("#{issue} (decided)"),
        }
    }
}

/// The PRs of `prs` that wait, each with what it waits on.
///
/// The open contest first, read from `rules` (the open `harness:rules`
/// issues) with no call; then, one comments read per PR, its latest
/// contest among `closed` (the closed `harness:rules` issues) while no
/// resumed marker follows it through — between the contest closing and
/// the resume, the PR is still red on the old test and its base has not
/// caught up: a repair then would be paid against the old rules — then the
/// waits a decision left on the PR, one issue read per wait named, while
/// the issue still blocks (open, not `waiting-merge`).
///
/// # Errors
/// [`harness_core::domain::Halt`] from a comments or an issue read: a
/// routing read that fails fails the tick.
pub async fn waiting(
    gh: &dyn GitHub,
    prs: &[Pr],
    rules: &[Issue],
    closed: &[Issue],
) -> Outcome<Vec<(Pr, PrWait)>> {
    let mut found = Vec::new();
    for pr in prs.iter().filter(|pr| pr.is_open()) {
        let Ok(number) = pr.num.parse::<u64>() else {
            continue;
        };
        if let Some(contest) = open_contest(number, rules) {
            found.push((
                pr.clone(),
                PrWait::Contest {
                    issue: contest.number,
                },
            ));
            continue;
        }
        let comments = gh.pr_comments(&pr.num).await?;
        if let Some(contest) = latest_closed_contest(number, closed)
            && !resumed_on(&comments, contest)
        {
            found.push((pr.clone(), PrWait::Settling { issue: contest }));
            continue;
        }
        for on in waits_named(&comments) {
            if tasks::still_blocks(&gh.issue(on).await?) {
                found.push((pr.clone(), PrWait::Decided { issue: on }));
                break;
            }
        }
    }
    Ok(found)
}

/// `prs` without those that wait.
#[must_use]
pub fn not_waiting(prs: &[Pr], waiting: &[(Pr, PrWait)]) -> Vec<Pr> {
    prs.iter()
        .filter(|pr| !waiting.iter().any(|(held, _)| held.num == pr.num))
        .cloned()
        .collect()
}

/// A settled contest to follow through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    /// The contest issue, closed.
    pub contest: u64,
    /// The PR that waited on it, to mark resumed — `None` for a contest
    /// about a milestone branch.
    pub pr: Option<String>,
    /// The branch to delete [`FILE`] on, when it is there and was written
    /// for this contest.
    pub lift: Option<String>,
}

/// Whether a path is the rules' or their tests'.
fn is_rules_path(path: &str) -> bool {
    path == "rules.json" || path.starts_with("tests/rules/")
}

/// Whether `branch` holds the rules as the integration branch has them:
/// the integration branch changed neither `rules.json` nor `tests/rules/`
/// since they parted. A compare GitHub lists in part, or cannot read, is
/// "not yet": one tick more.
async fn holds_rules(gh: &dyn GitHub, branch: &str, integration: &str) -> bool {
    if branch == integration {
        return true;
    }
    matches!(
        gh.changed_files(branch, integration).await,
        Ok(Some(files)) if !files.iter().any(|path| is_rules_path(path))
    )
}

/// Whether a [`FILE`] reading `text` was written for `contest`: it names a
/// rule, and none the contest's title and body do not — a file a later
/// session wrote for another rule is that contest's, still to open.
fn written_for(text: &str, contest: &Issue) -> bool {
    let named = dossier::rule_ids(text);
    let settled = dossier::rule_ids(&format!("{}\n{}", contest.title, contest.body));
    !named.is_empty() && named.iter().all(|id| settled.contains(id))
}

/// Whether an open contest names a PR whose head is `branch` — a landing
/// PR's contest, whose [`FILE`] lies on its milestone's branch.
fn contested_from(branch: &str, rules: &[Issue], prs: &[Pr]) -> bool {
    rules.iter().filter(|issue| issue.is_open()).any(|issue| {
        contested_pr(&issue.title).is_some_and(|n| {
            prs.iter()
                .any(|pr| pr.head == branch && pr.num == n.to_string())
        })
    })
}

/// Whether the PR's own diff adds or changes [`FILE`] — never a copy it
/// took from its base, which is the base's to lift.
async fn writes_the_file(gh: &dyn GitHub, pr: &Pr) -> Outcome<bool> {
    match gh.changed_files(&pr.base, &pr.head).await {
        Ok(Some(files)) => Ok(files.iter().any(|path| path == FILE)),
        // Listed in part, or not at all: the file is the PR's own when its
        // base has none.
        Ok(None) | Err(_) => Ok(gh.file_text(FILE, &pr.base).await?.is_none()),
    }
}

/// The settled contests to follow through, lowest contest first.
///
/// `closed` holds the closed `harness:rules` issues (one page, read once),
/// `rules` the open ones, `prs` the PRs of the harness's cycle, as read,
/// `milestones` the open milestones. A contest of PR `#n` is settled when
/// it is closed, no contest of `#n` is open again, `#n` is open and not
/// marked resumed on it yet, and the branch `#n` lands on holds the settled
/// rules (after its catch-up) — so CI reruns on the fixed test. A contest
/// about a milestone branch is settled when it is closed, the milestone is
/// open, no contest of the branch is open — neither one about it nor one
/// of a PR whose head it is — its branch still carries [`FILE`], written
/// for that contest (every rule it names is the contest's), and holds the
/// settled rules.
/// Idempotent: a lift already made leaves no file, a resume already made a
/// marker.
///
/// # Errors
/// [`harness_core::domain::Halt`] from a comments or a file read.
pub async fn settled(
    gh: &dyn GitHub,
    closed: &[Issue],
    rules: &[Issue],
    prs: &[Pr],
    milestones: &[Issue],
    integration: &str,
) -> Outcome<Vec<Settled>> {
    let mut contests: Vec<&Issue> = closed.iter().filter(|issue| !issue.is_open()).collect();
    // The latest contest of a PR or a branch is the one that counts.
    contests.sort_by_key(|issue| std::cmp::Reverse(issue.number));
    let mut found: Vec<Settled> = Vec::new();
    let mut seen_prs: Vec<u64> = Vec::new();
    let mut seen_milestones: Vec<u64> = Vec::new();
    for contest in contests {
        if let Some(number) = contested_pr(&contest.title) {
            if seen_prs.contains(&number) {
                continue;
            }
            seen_prs.push(number);
            if let Some(next) =
                pr_settled(gh, contest.number, number, rules, prs, integration).await?
            {
                found.push(next);
            }
        } else if let Some(milestone) = contested_milestone(&contest.title) {
            if seen_milestones.contains(&milestone) {
                continue;
            }
            seen_milestones.push(milestone);
            let still_open = rules.iter().any(|issue| {
                issue.is_open() && contested_milestone(&issue.title) == Some(milestone)
            });
            let Some(issue) = milestones
                .iter()
                .find(|m| m.number == milestone && m.is_open())
            else {
                continue;
            };
            if still_open {
                continue;
            }
            let branch = branching::milestone_branch(issue.number, &issue.title);
            if contested_from(&branch, rules, prs) {
                continue;
            }
            let Some(text) = gh.file_text(FILE, &branch).await? else {
                continue;
            };
            if written_for(&text, contest) && holds_rules(gh, &branch, integration).await {
                found.push(Settled {
                    contest: contest.number,
                    pr: None,
                    lift: Some(branch),
                });
            }
        }
    }
    // One lift per branch: a landing PR's head is its milestone's branch.
    let mut lifted: Vec<String> = Vec::new();
    for next in &mut found {
        if let Some(branch) = &next.lift {
            if lifted.contains(branch) {
                next.lift = None;
            } else {
                lifted.push(branch.clone());
            }
        }
    }
    found.retain(|next| next.pr.is_some() || next.lift.is_some());
    found.sort_by_key(|next| next.contest);
    Ok(found)
}

/// The contest `contest` of PR `number`, when it is settled now.
async fn pr_settled(
    gh: &dyn GitHub,
    contest: u64,
    number: u64,
    rules: &[Issue],
    prs: &[Pr],
    integration: &str,
) -> Outcome<Option<Settled>> {
    if open_contest(number, rules).is_some() {
        return Ok(None);
    }
    let Some(pr) = prs
        .iter()
        .find(|pr| pr.is_open() && pr.num == number.to_string())
    else {
        return Ok(None);
    };
    if resumed_on(&gh.pr_comments(&pr.num).await?, contest) {
        return Ok(None);
    }
    if !holds_rules(gh, &pr.base, integration).await {
        return Ok(None);
    }
    let lift = (writes_the_file(gh, pr).await? && gh.file_text(FILE, &pr.head).await?.is_some())
        .then(|| pr.head.clone());
    Ok(Some(Settled {
        contest,
        pr: Some(pr.num.clone()),
        lift,
    }))
}

/// What makes a PR with no lift of its own run its checks again, said as
/// its resumed comment says it.
///
/// A `pull_request` CI does not run again when only the base moves: the
/// PR's base is merged into its head (GitHub's update-branch), which
/// brings the settled rules and a commit the CI runs on — nothing when the
/// head holds the base already, whose last run then saw the settled rules.
/// A landing PR's head is its milestone's branch: its catch-up does that,
/// never a merge from here. A base that does not merge is said, not
/// halted on: the repair the PR goes to brings it up to date.
async fn bring_up_to_date(gh: &dyn GitHub, pr: &str, log: &Logbook) -> Outcome<String> {
    let read = gh.pr(pr).await?;
    if branching::milestone_number(&read.head).is_some() {
        return Ok(format!(
            "no {FILE} of its own to lift; its head is a milestone's branch, whose catch-up \
             brings the settled rules — its checks run again on that push"
        ));
    }
    Ok(match gh.merge_branch(&read.head, &read.base).await {
        Ok(()) => format!(
            "no {FILE} of its own to lift; {} holds {} now, so its checks run on the settled rules",
            read.head, read.base
        ),
        Err(halt) => {
            log.warn(&format!(
                "watch: pr #{pr} — {} does not merge into {}: {halt}",
                read.base, read.head
            ));
            format!(
                "no {FILE} of its own to lift, and {} does not merge into {}: its checks still \
                 reflect the old base until a repair brings it up to date",
                read.base, read.head
            )
        }
    })
}

/// Follows a settled contest through, then records it.
///
/// [`FILE`] deleted on its branch (a commit made on GitHub, no checkout) —
/// the CI runs again on that commit; with no lift of its own, the PR
/// brought up to date with its base instead (`bring_up_to_date`) — then the
/// PR marked resumed, its repairs counted anew. A file already gone is no
/// failure: the resume is what a crash between the two left to do.
///
/// # Errors
/// [`harness_core::domain::Halt`] from the delete, the PR's read or the
/// comment.
pub async fn resume(gh: &dyn GitHub, settled: &Settled, log: &Logbook) -> Outcome<()> {
    let mut lifted_on = None;
    if let Some(branch) = &settled.lift {
        let message = format!("chore: lift {FILE} — contest #{} settled", settled.contest);
        if gh.delete_file(FILE, branch, &message).await? {
            lifted_on = Some(branch.clone());
        }
    }
    if let Some(pr) = &settled.pr {
        let number: u64 = pr.parse().map_err(|_| {
            harness_core::domain::Halt::Failed(format!("PR {pr:?} has no number to comment on"))
        })?;
        let rerun = match &lifted_on {
            Some(branch) => format!(
                "{FILE} lifted on {branch}, so its checks run again on that commit, on the \
                 settled rules"
            ),
            None => bring_up_to_date(gh, pr, log).await?,
        };
        gh.post_issue_comment(
            number,
            &format!(
                "{}\nContest #{} settled — {rerun}. The repairs of this PR are counted anew.",
                resumed_marker(settled.contest),
                settled.contest
            ),
        )
        .await?;
    }
    log.event(&Event::Resumed {
        contest: settled.contest,
        pr: settled.pr.clone(),
        branch: lifted_on,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::common::{labels, review};
    use proptest::prelude::*;
    use std::collections::HashMap;

    fn rules_issue(number: u64, state: &str, title: &str) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            title: title.to_string(),
            labels: vec![labels::RULES.to_string()],
            ..Issue::default()
        }
    }

    fn pr(num: &str, head: &str, base: &str) -> Pr {
        Pr {
            num: num.to_string(),
            head: head.to_string(),
            base: base.to_string(),
            state: "OPEN".to_string(),
            ..Pr::default()
        }
    }

    proptest! {
        #[test]
        fn a_contest_title_reads_back_its_pr(n in 1u64..1_000_000, k in proptest::option::of(2u32..9)) {
            let written = k.map_or_else(|| title(n), |k| format!("{} ({k})", title(n)));
            prop_assert_eq!(contested_pr(&written), Some(n));
            prop_assert_eq!(contested_milestone(&written), None);
        }

        #[test]
        fn a_wait_marker_reads_back_until_its_resume(on in 1u64..100_000, other in 1u64..100_000) {
            let waited = format!("decided\n{}\nwhy", waits_on_marker(on));
            prop_assert_eq!(waits_named(&waited), vec![on]);
            prop_assert_eq!(waits_decided(&waited), 1);
            prop_assert!(!resumed_on(&waited, on));
            let resumed = format!("{waited}\n{}", resumed_marker(on));
            prop_assert!(waits_named(&resumed).is_empty());
            prop_assert!(resumed_on(&resumed, on));
            prop_assert_eq!(waits_decided(&resumed), 1, "a resume is no wait");
            if other != on {
                let elsewhere = format!("{waited}\n{}", resumed_marker(other));
                prop_assert_eq!(waits_named(&elsewhere), vec![on], "another issue's resume clears nothing");
                prop_assert!(!resumed_on(&elsewhere, on));
            }
            // A resume posted before the wait does not clear it.
            let before = format!("{}\n{waited}", resumed_marker(on));
            prop_assert_eq!(waits_named(&before), vec![on]);
        }
    }

    #[test]
    fn titles_that_are_no_contest_of_a_pr_read_as_none() {
        assert_eq!(contested_pr("Rules contested: PR #12"), Some(12));
        assert_eq!(contested_pr("Rules contested: PR #12 (2)"), Some(12));
        assert_eq!(contested_pr("Rules contested: R10 on milestone/28"), None);
        assert_eq!(contested_pr("Rules contested: PR #12x"), None);
        assert_eq!(contested_pr("Rules contested: PR #0"), None);
        assert_eq!(contested_pr("Rules changed: R3"), None);
        assert_eq!(
            contested_milestone("Rules contested: R10 on milestone/28"),
            Some(28)
        );
        assert_eq!(
            contested_milestone("Rules contested: R10 on milestone/28-closing-a-session"),
            Some(28)
        );
        assert_eq!(
            contested_milestone("Rules changed: R10 on milestone/28"),
            None
        );
        assert_eq!(
            contested_milestone("Rules contested: R10 on main_agent"),
            None
        );
    }

    #[test]
    fn only_an_open_contest_titled_for_this_pr_holds_it() {
        let rules = vec![
            rules_issue(517, "open", "Rules contested: PR #516"),
            rules_issue(400, "closed", "Rules contested: PR #12"),
            rules_issue(3, "open", "Rules: write the rules and their tests"),
        ];
        assert_eq!(open_contest(516, &rules).map(|i| i.number), Some(517));
        assert!(open_contest(12, &rules).is_none(), "closed");
        assert!(open_contest(5160, &rules).is_none());
    }

    #[tokio::test]
    async fn a_pr_waits_on_its_open_contest_or_on_a_decided_wait_still_blocking() {
        let rules = vec![rules_issue(517, "open", "Rules contested: PR #516")];
        let open_blocker = Issue {
            number: 20,
            state: "open".to_string(),
            ..Issue::default()
        };
        let landed = Issue {
            number: 21,
            state: "closed".to_string(),
            ..Issue::default()
        };
        let gh = FakeGitHub {
            issues: vec![open_blocker, landed],
            pr_comment_bodies: vec![
                ("40".to_string(), waits_on_marker(20)),
                ("41".to_string(), waits_on_marker(21)),
                (
                    "42".to_string(),
                    format!("{}\n{}", waits_on_marker(20), resumed_marker(20)),
                ),
            ],
            ..FakeGitHub::default()
        };
        let prs = [
            pr("516", "feat/scene", "milestone/27-x"),
            pr("40", "feat/a", "milestone/27-x"),
            pr("41", "feat/b", "milestone/27-x"),
            pr("42", "feat/c", "milestone/27-x"),
            pr("43", "feat/d", "milestone/27-x"),
        ];
        let found = waiting(&gh, &prs, &rules, &[]).await.expect("read");
        let told: Vec<(&str, PrWait)> = found.iter().map(|(p, w)| (p.num.as_str(), *w)).collect();
        assert_eq!(
            told,
            [
                ("516", PrWait::Contest { issue: 517 }),
                ("40", PrWait::Decided { issue: 20 })
            ],
            "#41's issue landed, #42 was resumed, #43 waits on nothing"
        );
        assert_eq!(
            not_waiting(&prs, &found)
                .iter()
                .map(|p| p.num.as_str())
                .collect::<Vec<_>>(),
            ["41", "42", "43"]
        );
        assert_eq!(found[0].1.describe(), "contest #517");
        assert_eq!(found[1].1.describe(), "#20 (decided)");
    }

    /// The board of the live case: PR #516 into milestone/27, its contest
    /// #517 closed, the rules PR merged on `main_agent`.
    fn settled_board(base_behind: bool) -> FakeGitHub {
        let mut changed = HashMap::from([(
            "milestone/27-x...feat/scene".to_string(),
            Some(vec!["CONTEST.md".to_string(), "src/a.rs".to_string()]),
        )]);
        if base_behind {
            changed.insert(
                "milestone/27-x...main_agent".to_string(),
                Some(vec!["tests/rules/business/tests/r10.rs".to_string()]),
            );
        }
        FakeGitHub {
            changed_files: changed,
            files: HashMap::from([(
                ("CONTEST.md".to_string(), "feat/scene".to_string()),
                "R10 cannot hold".to_string(),
            )]),
            ..FakeGitHub::default()
        }
    }

    #[tokio::test]
    async fn a_closed_contest_on_a_base_holding_the_rules_is_settled() {
        let gh = settled_board(false);
        let closed = [rules_issue(517, "closed", "Rules contested: PR #516")];
        let prs = [pr("516", "feat/scene", "milestone/27-x")];
        let found = settled(&gh, &closed, &[], &prs, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(
            found,
            [Settled {
                contest: 517,
                pr: Some("516".to_string()),
                lift: Some("feat/scene".to_string()),
            }]
        );
        // Followed through: lifted, then resumed.
        let log = Logbook::null();
        resume(&gh, &found[0], &log).await.expect("resumed");
        let writes = gh.writes();
        assert!(matches!(
            &writes[0],
            Wrote::DeletedFile(path, branch, message)
                if path == "CONTEST.md" && branch == "feat/scene"
                    && message == "chore: lift CONTEST.md — contest #517 settled"
        ));
        assert!(matches!(
            &writes[1],
            Wrote::Comment(516, said) if said.starts_with(&resumed_marker(517))
                && said.contains("CONTEST.md lifted on feat/scene, so its checks run again")
        ));
        assert_eq!(writes.len(), 2, "a lift is its own push: {writes:?}");
    }

    #[tokio::test]
    async fn a_contest_not_yet_opened_by_ci_or_still_open_is_not_settled() {
        let gh = settled_board(false);
        let prs = [pr("516", "feat/scene", "milestone/27-x")];
        let none = settled(&gh, &[], &[], &prs, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(
            none,
            Vec::<Settled>::new(),
            "no contest issue yet: CI has not run"
        );
        let closed = [rules_issue(517, "closed", "Rules contested: PR #516")];
        let reopened = [rules_issue(530, "open", "Rules contested: PR #516 (2)")];
        let again = settled(&gh, &closed, &reopened, &prs, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(
            again,
            Vec::<Settled>::new(),
            "a second contest of #516 is open"
        );
        let gone = settled(&gh, &closed, &[], &[], &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(gone, Vec::<Settled>::new(), "#516 is no longer open");
    }

    #[tokio::test]
    async fn a_base_still_behind_the_rules_waits() {
        let gh = settled_board(true);
        let closed = [rules_issue(517, "closed", "Rules contested: PR #516")];
        let prs = [pr("516", "feat/scene", "milestone/27-x")];
        let found = settled(&gh, &closed, &[], &prs, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(
            found,
            Vec::<Settled>::new(),
            "the catch-up brings the rules first"
        );
        // A compare GitHub cannot read waits too.
        let unread = FakeGitHub {
            broken_compares: vec!["milestone/27-x...main_agent".to_string()],
            ..settled_board(false)
        };
        let found = settled(&unread, &closed, &[], &prs, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(found, Vec::<Settled>::new());
        // A landing PR's base is the integration branch: it holds them.
        let landing = [pr("520", "milestone/27-x", "main_agent")];
        let closed = [rules_issue(521, "closed", "Rules contested: PR #520")];
        let found = settled(&gh, &closed, &[], &landing, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(found.len(), 1);
    }

    #[tokio::test]
    async fn a_resumed_pr_is_not_resumed_again_and_a_lift_made_only_resumes() {
        let closed = [rules_issue(517, "closed", "Rules contested: PR #516")];
        let prs = [pr("516", "feat/scene", "milestone/27-x")];
        let done = FakeGitHub {
            pr_comment_bodies: vec![("516".to_string(), resumed_marker(517))],
            ..settled_board(false)
        };
        let found = settled(&done, &closed, &[], &prs, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(found, Vec::<Settled>::new(), "resumed already");
        // A crash after the delete, before the marker: no file, a resume
        // that brings the PR up to date so its checks run again.
        let crashed = FakeGitHub {
            files: HashMap::new(),
            prs: vec![("516".to_string(), prs[0].clone())],
            ..settled_board(false)
        };
        let found = settled(&crashed, &closed, &[], &prs, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(
            found,
            [Settled {
                contest: 517,
                pr: Some("516".to_string()),
                lift: None,
            }]
        );
        resume(&crashed, &found[0], &Logbook::null())
            .await
            .expect("resumed");
        let writes = crashed.writes();
        assert_eq!(writes.len(), 2, "nothing deleted: {writes:?}");
        assert_eq!(
            writes[0],
            Wrote::MergedBranch("feat/scene".to_string(), "milestone/27-x".to_string()),
            "the base merged into the head: a commit the CI runs on"
        );
        assert!(matches!(
            &writes[1],
            Wrote::Comment(516, said) if said.contains("no CONTEST.md of its own")
                && said.contains("feat/scene holds milestone/27-x now")
        ));
    }

    #[tokio::test]
    async fn a_resume_with_no_lift_says_what_reruns_the_checks() {
        let settled = Settled {
            contest: 532,
            pr: Some("531".to_string()),
            lift: None,
        };
        // The base does not merge: said, not halted on.
        let conflicting = FakeGitHub {
            prs: vec![(
                "531".to_string(),
                pr("531", "feat/journal", "milestone/28-x"),
            )],
            refused_merges: vec!["feat/journal".to_string()],
            ..FakeGitHub::default()
        };
        resume(&conflicting, &settled, &Logbook::null())
            .await
            .expect("resumed");
        assert!(matches!(
            conflicting.writes().as_slice(),
            [Wrote::Comment(531, said)] if said.contains("does not merge into feat/journal")
                && said.contains("still reflect the old base")
                && !said.contains("run on the settled rules")
        ));
        // A landing PR: its milestone's catch-up moves its head, never a
        // merge from here.
        let landing = FakeGitHub {
            prs: vec![("539".to_string(), pr("539", "milestone/28-x", "main_agent"))],
            ..FakeGitHub::default()
        };
        let settled = Settled {
            contest: 540,
            pr: Some("539".to_string()),
            lift: None,
        };
        resume(&landing, &settled, &Logbook::null())
            .await
            .expect("resumed");
        assert!(matches!(
            landing.writes().as_slice(),
            [Wrote::Comment(539, said)] if said.contains("whose catch-up brings the settled rules")
        ));
        // A PR that cannot be read is no resume: the next tick tries again.
        let unread = FakeGitHub::default();
        assert!(resume(&unread, &settled, &Logbook::null()).await.is_err());
        assert_eq!(unread.writes(), Vec::<Wrote>::new());
    }

    #[tokio::test]
    async fn an_inherited_contest_md_is_never_lifted_on_the_pr() {
        // PR #531 took milestone/28's CONTEST.md: its own diff has none.
        let gh = FakeGitHub {
            changed_files: HashMap::from([(
                "milestone/28-x...feat/journal".to_string(),
                Some(vec!["src/journal.rs".to_string()]),
            )]),
            files: HashMap::from([
                (
                    ("CONTEST.md".to_string(), "feat/journal".to_string()),
                    "R10".to_string(),
                ),
                (
                    ("CONTEST.md".to_string(), "milestone/28-x".to_string()),
                    "R10".to_string(),
                ),
            ]),
            ..FakeGitHub::default()
        };
        let closed = [rules_issue(532, "closed", "Rules contested: PR #531")];
        let prs = [pr("531", "feat/journal", "milestone/28-x")];
        let found = settled(&gh, &closed, &[], &prs, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].lift, None, "the base's copy is the base's to lift");
        // A diff listed in part: the base's own copy says it is inherited.
        let partial = FakeGitHub {
            changed_files: HashMap::from([("milestone/28-x...feat/journal".to_string(), None)]),
            ..gh
        };
        let found = settled(&partial, &closed, &[], &prs, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(found[0].lift, None);
    }

    #[tokio::test]
    async fn a_settled_contest_on_a_milestone_branch_lifts_it_once_caught_up() {
        // milestone/28 carries CONTEST.md for R10 (#459, closed).
        let milestone = Issue {
            number: 28,
            state: "open".to_string(),
            title: "Closing a session".to_string(),
            ..Issue::default()
        };
        let branch = branching::milestone_branch(28, "Closing a session");
        let behind = FakeGitHub {
            files: HashMap::from([(
                ("CONTEST.md".to_string(), branch.clone()),
                "R10".to_string(),
            )]),
            changed_files: HashMap::from([(
                format!("{branch}...main_agent"),
                Some(vec!["rules.json".to_string()]),
            )]),
            ..FakeGitHub::default()
        };
        let closed = [rules_issue(
            459,
            "closed",
            "Rules contested: R10 on milestone/28",
        )];
        let milestones = [milestone];
        let found = settled(&behind, &closed, &[], &[], &milestones, "main_agent")
            .await
            .expect("read");
        assert_eq!(
            found,
            Vec::<Settled>::new(),
            "the branch has not caught up yet"
        );
        let caught_up = FakeGitHub {
            changed_files: HashMap::new(),
            ..behind
        };
        let found = settled(&caught_up, &closed, &[], &[], &milestones, "main_agent")
            .await
            .expect("read");
        assert_eq!(
            found,
            [Settled {
                contest: 459,
                pr: None,
                lift: Some(branch.clone()),
            }]
        );
        resume(&caught_up, &found[0], &Logbook::null())
            .await
            .expect("lifted");
        assert!(matches!(
            caught_up.writes().as_slice(),
            [Wrote::DeletedFile(path, on, _)] if path == "CONTEST.md" && *on == branch
        ));
        // Lifted already: nothing left to do.
        let lifted = FakeGitHub::default();
        let found = settled(&lifted, &closed, &[], &[], &milestones, "main_agent")
            .await
            .expect("read");
        assert_eq!(found, Vec::<Settled>::new());
        // A milestone closed since, or a contest of it open again: nothing.
        let found = settled(&caught_up, &closed, &[], &[], &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(found, Vec::<Settled>::new());
        let open_again = [rules_issue(
            600,
            "open",
            "Rules contested: R11 on milestone/28",
        )];
        let found = settled(
            &caught_up,
            &closed,
            &open_again,
            &[],
            &milestones,
            "main_agent",
        )
        .await
        .expect("read");
        assert_eq!(found, Vec::<Settled>::new());
    }

    #[tokio::test]
    async fn a_landing_pr_and_its_milestone_contest_lift_its_branch_once() {
        let milestone = Issue {
            number: 28,
            state: "open".to_string(),
            title: "x".to_string(),
            ..Issue::default()
        };
        let branch = branching::milestone_branch(28, "x");
        let gh = FakeGitHub {
            files: HashMap::from([(
                ("CONTEST.md".to_string(), branch.clone()),
                "R10".to_string(),
            )]),
            changed_files: HashMap::from([(
                format!("main_agent...{branch}"),
                Some(vec!["CONTEST.md".to_string()]),
            )]),
            ..FakeGitHub::default()
        };
        let closed = [
            rules_issue(459, "closed", "Rules contested: R10 on milestone/28"),
            rules_issue(540, "closed", "Rules contested: PR #539"),
        ];
        let prs = [pr("539", &branch, "main_agent")];
        let found = settled(&gh, &closed, &[], &prs, &[milestone], "main_agent")
            .await
            .expect("read");
        let lifts: Vec<&Option<String>> = found.iter().map(|s| &s.lift).collect();
        assert_eq!(lifts.iter().filter(|l| l.is_some()).count(), 1, "{found:?}");
        assert!(found.iter().any(|s| s.pr.as_deref() == Some("539")));
    }

    /// The live window: #518 merged on `main_agent`, #517 closed by
    /// `Closes`, milestone/24 not caught up yet — PR #516 still red on
    /// `CONTEST.md` and the old R103 test.
    fn window(comments: &str) -> FakeGitHub {
        FakeGitHub {
            changed_files: HashMap::from([
                (
                    "milestone/24-x...main_agent".to_string(),
                    Some(vec!["tests/rules/business/tests/r103.rs".to_string()]),
                ),
                (
                    "milestone/24-x...feat/scene".to_string(),
                    Some(vec!["CONTEST.md".to_string()]),
                ),
            ]),
            files: HashMap::from([(
                ("CONTEST.md".to_string(), "feat/scene".to_string()),
                "R103 getByText('CA') hits 3".to_string(),
            )]),
            pr_comment_bodies: vec![("516".to_string(), comments.to_string())],
            failing_checks: HashMap::from([(
                "516".to_string(),
                vec!["node-gates — FAILURE".to_string()],
            )]),
            pr_checks: HashMap::from([("516".to_string(), false)]),
            ..FakeGitHub::default()
        }
    }

    fn task486() -> Issue {
        Issue {
            number: 486,
            state: "open".to_string(),
            labels: vec![
                labels::AGENT.to_string(),
                labels::REVIEW_PENDING.to_string(),
            ],
            body: "## Scope\n\nbranch: feat/scene\n".to_string(),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_pr_whose_contest_closed_waits_until_it_is_resumed() {
        // Regression: in the window no repair was due to be paid on the
        // old test.
        let gh = window("");
        let closed = [rules_issue(517, "closed", "Rules contested: PR #516")];
        let prs = [pr("516", "feat/scene", "milestone/24-x")];
        let waits = waiting(&gh, &prs, &[], &closed).await.expect("read");
        assert_eq!(
            waits
                .iter()
                .map(|(p, w)| (p.num.as_str(), *w))
                .collect::<Vec<_>>(),
            [("516", PrWait::Settling { issue: 517 })]
        );
        assert!(waits[0].1.describe().starts_with("contest #517 settled"));
        let settled_now = settled(&gh, &closed, &[], &prs, &[], "main_agent")
            .await
            .expect("read");
        assert!(settled_now.is_empty(), "the base has not caught up");
        let due = crate::pr_fix::data::candidates::due(&gh, &[], &not_waiting(&prs, &waits))
            .await
            .expect("read");
        assert!(due.is_empty(), "no paid repair in the window: {due:?}");
        // Resumed on #517: it waits no more. An older contest resumed
        // does not count for a later one.
        let resumed = window(&resumed_marker(517));
        assert_eq!(
            waiting(&resumed, &prs, &[], &closed).await.expect("read"),
            Vec::<(Pr, PrWait)>::new()
        );
        let later = [
            rules_issue(517, "closed", "Rules contested: PR #516"),
            rules_issue(530, "closed", "Rules contested: PR #516 (2)"),
        ];
        assert_eq!(
            waiting(&resumed, &prs, &[], &later).await.expect("read")[0].1,
            PrWait::Settling { issue: 530 }
        );
        assert_eq!(latest_closed_contest(516, &later), Some(530));
        assert_eq!(latest_closed_contest(516, &[]), None);
        // A contest still open is the open wait, not the settling one.
        let open_again = [rules_issue(530, "open", "Rules contested: PR #516 (2)")];
        assert_eq!(
            waiting(&resumed, &prs, &open_again, &later)
                .await
                .expect("read")[0]
                .1,
            PrWait::Contest { issue: 530 }
        );
    }

    #[tokio::test]
    async fn a_pr_with_its_repairs_spent_is_not_escalated_in_the_window() {
        // Regression: #516 at deploy, 2 pr_fix spent and a clean review —
        // the window asked for a paid decision on #486.
        let fixes = format!("{}\n", review::FIX_MARKER).repeat(review::MAX_FIXES);
        let comments = format!("{fixes}{}\n{}\n", review::MARKER, review::CLEAN);
        let gh = window(&comments);
        let closed = [rules_issue(517, "closed", "Rules contested: PR #516")];
        let prs = [pr("516", "feat/scene", "milestone/24-x")];
        let waits = waiting(&gh, &prs, &[], &closed).await.expect("read");
        let open = not_waiting(&prs, &waits);
        let (_, escalated) = crate::milestone_merge::ready_to_merge(&gh, &[task486()], &open)
            .await
            .expect("read");
        assert!(escalated.is_empty(), "{escalated:?}");
        assert!(
            !gh.writes()
                .contains(&Wrote::Label(486, labels::NEEDS_DECISION.to_string()))
        );
    }

    #[tokio::test]
    async fn a_milestone_file_written_for_another_rule_or_contest_is_not_lifted() {
        // #437 "R16 red, R111 flaky on milestone/26" closed long ago; a
        // session writes a fresh CONTEST.md on milestone/26 for R200.
        let milestone = Issue {
            number: 26,
            state: "open".to_string(),
            title: "x".to_string(),
            ..Issue::default()
        };
        let branch = branching::milestone_branch(26, "x");
        let board = |text: &str| FakeGitHub {
            files: HashMap::from([(("CONTEST.md".to_string(), branch.clone()), text.to_string())]),
            ..FakeGitHub::default()
        };
        let closed = [rules_issue(
            437,
            "closed",
            "Rules contested: R16 red, R111 flaky on milestone/26",
        )];
        let milestones = [milestone];
        for fresh in ["R200 cannot hold", "R16 and R200", "no rule named"] {
            let found = settled(&board(fresh), &closed, &[], &[], &milestones, "main_agent")
                .await
                .expect("read");
            assert_eq!(found, Vec::<Settled>::new(), "{fresh:?} is not #437's");
        }
        // The contest's own rules, named in its title or its body: lifted.
        let found = settled(
            &board("R111 flaky"),
            &closed,
            &[],
            &[],
            &milestones,
            "main_agent",
        )
        .await
        .expect("read");
        assert_eq!(found.len(), 1, "{found:?}");
        let mut by_body = rules_issue(437, "closed", "Rules contested: flaky on milestone/26");
        by_body.body = "R16 red, R111 flaky".to_string();
        let found = settled(
            &board("R16"),
            &[by_body],
            &[],
            &[],
            &milestones,
            "main_agent",
        )
        .await
        .expect("read");
        assert_eq!(found.len(), 1, "{found:?}");
        // An open contest of the landing PR, whose head is the branch: its
        // file is live, never lifted.
        let landing = [pr("539", &branch, "main_agent")];
        let open = [rules_issue(540, "open", "Rules contested: PR #539")];
        let found = settled(
            &board("R111"),
            &closed,
            &open,
            &landing,
            &milestones,
            "main_agent",
        )
        .await
        .expect("read");
        assert_eq!(found, Vec::<Settled>::new());
        // An open contest of a PR whose head is another branch holds
        // nothing here.
        let task = [pr("541", "feat/other", &branch)];
        let other = [rules_issue(542, "open", "Rules contested: PR #541")];
        let found = settled(
            &board("R111"),
            &closed,
            &other,
            &task,
            &milestones,
            "main_agent",
        )
        .await
        .expect("read");
        assert_eq!(found.len(), 1, "{found:?}");
    }
}
