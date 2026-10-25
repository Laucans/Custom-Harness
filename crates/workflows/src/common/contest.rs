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
//! while `#N` still blocks and no person holds it ([`wait_stands`]). A waiting PR
//! gets no repair, no escalation, no decision and no merge; the review
//! still runs. A resumed marker clears the wait on `#N`.
//!
//! **A settled contest is followed through for free** ([`settled`],
//! [`resume`]): once its issue is closed and the branch the PR lands on
//! holds the settled rules, the harness deletes `CONTEST.md` where the
//! contest wrote it — on the PR's branch when its own diff added it, on a
//! milestone branch a contest about that branch named, when the file names
//! no rule beyond that contest's — brings a PR with no lift of its own up
//! to date with its base, so its checks run on the settled rules, and marks
//! the PR resumed, which counts its repairs anew (`review::repairs`).
//!
//! **A decided wait that settled is followed through too**
//! ([`resume_decided`], from `milestone_merge::escalate`): once the work
//! of the issue it named reached the PR's base ([`landed_on_base`]: closed
//! or `waiting-merge`, never cut — built under another milestone, that
//! milestone landed and the base caught up), the PR is marked resumed on
//! it — its repairs counted anew, one repair asked — never handed to a
//! human for having waited (`ForgeDesMondes`' #569 and #580, 2026-10-25:
//! the waits' cap sent both to a human minutes after #567 and #582
//! landed). One cut ([`was_cut`]) is no landing: the PR goes to a human.
//!
//! **A repair that stopped on a contract waits on its change** (H2,
//! `CONTRACT_WAIT`): until a resumed marker follows the change — posted once
//! it is closed and its work reached the PR's base ([`contract_landed`]:
//! the producer task the contract manager opened landed there
//! ([`landed_on_base`]), or, with none recorded, the base holds the
//! manifest), then [`resume_contract`], which counts the PR's repairs anew
//! too — or, the producer cut, hands the PR to a human. Both kinds of
//! marker count in [`MAX_WAITS`].

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

/// What opens the marker a repair leaves when it stopped on a contract.
///
/// The contract is another milestone's (`common::contract_ask`); the marker is
/// followed by the contract change's number and ` -->`. The PR waits until
/// the change is closed **and** its base holds the manifest
/// ([`contract_landed`]); a resumed marker on it then clears it and counts
/// the PR's repairs anew.
pub const CONTRACT_WAIT: &str = "<!-- harness-waits-on-contract: #";

/// How many decided waits one PR gets before a human decides: a decision
/// that keeps answering "wait" is not left to loop.
pub const MAX_WAITS: usize = 2;

/// The marker a decided `wait` on `on` leaves on the PR.
#[must_use]
pub fn waits_on_marker(on: u64) -> String {
    format!("{WAITS_ON}{on} -->")
}

/// The marker a repair stopped on a contract leaves on the PR, waiting on
/// the contract change `on`.
#[must_use]
pub fn contract_wait_marker(on: u64) -> String {
    format!("{CONTRACT_WAIT}{on} -->")
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

/// The contract changes a repair stopped on and no resumed marker cleared
/// since, in the order first named.
#[must_use]
pub fn contract_waits(comments: &str) -> Vec<u64> {
    let resumed = markers(comments, RESUMED);
    let mut found: Vec<u64> = Vec::new();
    for (at, on) in markers(comments, CONTRACT_WAIT) {
        let cleared = resumed
            .iter()
            .any(|(later, number)| *number == on && *later > at);
        if !cleared && !found.contains(&on) {
            found.push(on);
        }
    }
    found
}

/// How many times the harness decided the PR waits, in all — a decided
/// `wait` or a contract its repair stopped on: both count in the cap
/// ([`MAX_WAITS`]).
#[must_use]
pub fn waits_decided(comments: &str) -> usize {
    markers(comments, WAITS_ON)
        .len()
        .saturating_add(markers(comments, CONTRACT_WAIT).len())
}

/// Whether a decided wait on `target` still holds the PR.
///
/// The issue still blocks — open, and not landed (`harness:waiting-merge`,
/// a task on its milestone's branch, a milestone on the integration
/// branch: [`tasks::still_blocks`]) — and no person holds it.
///
/// One a person holds (`harness:human`) is stuck ([`wait_stuck`]): the
/// wait cannot settle by itself, and the PR is a human's too. One landed
/// lets the PR go to `milestone_merge::escalate`, which reads whether its
/// work reached the PR's base ([`landed_on_base`]) before it resumes it.
#[must_use]
pub fn wait_stands(target: &Issue) -> bool {
    tasks::still_blocks(target) && !wait_stuck(target)
}

/// Whether a decided wait on `target` is stuck: the issue is open and a
/// person holds it (`harness:human`).
#[must_use]
pub fn wait_stuck(target: &Issue) -> bool {
    target.is_open() && target.has(crate::common::labels::HUMAN)
}

/// Whether `issue` was cut: closed with nothing it was to deliver.
///
/// Closed as not planned (`Issue::not_planned`) — how `decide`'s `cut`
/// and the re-study's not-needed task close one — or, cut before those
/// closed as not planned, closed as completed with a `## Decision` verdict
/// saying `cut`: never "landed".
#[must_use]
pub fn was_cut(issue: &Issue) -> bool {
    issue.is_closed()
        && (issue.not_planned
            || crate::common::sections::parse(&issue.body)
                .get(crate::common::sections::DECISION)
                .and_then(|text| crate::common::decision::Question::parse(text))
                .and_then(|asked| asked.verdict)
                .is_some_and(|verdict| verdict.trim_start().starts_with("cut")))
}

/// The milestone `issue` was built under: itself for a milestone, else the
/// milestone issue its native milestone mirrors (`Issue::milestone`).
#[must_use]
pub fn home_of(issue: &Issue) -> Option<u64> {
    if issue.has(crate::common::labels::MILESTONE) {
        Some(issue.number)
    } else {
        issue.milestone
    }
}

/// Where an issue a PR waits on stands against the PR's base
/// ([`landed_on_base`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// Its work is on the PR's base.
    Landed,
    /// Not yet: still to land, or landed where the PR's base has not
    /// caught up.
    NotYet,
    /// The issue, or the milestone it was built under, was cut
    /// ([`was_cut`]): its work will never come.
    Cut(u64),
    /// Built under this milestone, which waits on the PR's own milestone
    /// (`blocked_by`): it cannot land before the PR does.
    Cycle(u64),
}

/// Whether the work of `target` — an issue a PR waits on, built under the
/// milestone `home` — reached the PR's base.
///
/// Landed is closed or `harness:waiting-merge` ([`tasks::still_blocks`]),
/// never cut ([`was_cut`]). Built under the PR's own milestone — the base
/// of a task's PR, the head of a landing PR — landed is enough: it is
/// there. Built under another, that milestone landed too (closed or
/// `harness:waiting-merge`: on the integration branch), and the base holds
/// what the integration branch has of the files `touched` matches
/// ([`base_holds`]). A milestone that waits on the PR's own (`blocked_by`)
/// is [`Landing::Cycle`]: waiting for it would never end. With no home
/// known (a task on no native milestone), landed and the base holding the
/// files.
///
/// # Errors
/// [`harness_core::domain::Halt`] from the read of the home milestone or
/// its blockers.
pub async fn landed_on_base(
    gh: &dyn GitHub,
    pr: &Pr,
    target: &Issue,
    home: Option<u64>,
    touched: impl Fn(&str) -> bool,
    integration: &str,
) -> Outcome<Landing> {
    if was_cut(target) {
        return Ok(Landing::Cut(target.number));
    }
    let own: Vec<u64> = [&pr.base, &pr.head]
        .into_iter()
        .filter_map(|branch| branching::milestone_number(branch))
        .collect();
    let Some(home) = home else {
        if tasks::still_blocks(target) {
            return Ok(Landing::NotYet);
        }
        return Ok(holds(base_holds(gh, &pr.base, integration, touched).await));
    };
    if own.contains(&home) {
        return Ok(if tasks::still_blocks(target) {
            Landing::NotYet
        } else {
            Landing::Landed
        });
    }
    let milestone = if home == target.number {
        target.clone()
    } else {
        gh.issue(home).await?
    };
    if was_cut(&milestone) {
        return Ok(Landing::Cut(home));
    }
    if tasks::still_blocks(target) || tasks::still_blocks(&milestone) {
        let behind_own = !own.is_empty()
            && gh
                .blocked_by(home)
                .await?
                .iter()
                .any(|blocker| own.contains(&blocker.number) && tasks::still_blocks(blocker));
        return Ok(if behind_own {
            Landing::Cycle(home)
        } else {
            Landing::NotYet
        });
    }
    Ok(holds(base_holds(gh, &pr.base, integration, touched).await))
}

/// [`Landing::Landed`] when `held`, else [`Landing::NotYet`].
const fn holds(held: bool) -> Landing {
    if held {
        Landing::Landed
    } else {
        Landing::NotYet
    }
}

/// Follows a settled decided wait through, one repair asked.
///
/// `harness:pr-fix` first, then the resumed marker on `pr` for every issue
/// of `landed` — which clears the waits and counts the PR's repairs anew
/// (`review::repairs`): one repair on the base that holds them now. In
/// that order so a stop between the two leaves a label without a marker,
/// which the next escalation follows through again, never a PR cleared of
/// its waits with no label the router repairs.
///
/// What the decision itself said: "the task waits on #567 and is repaired
/// once afterwards" (`ForgeDesMondes`' #568). Without it the waits' cap
/// counted the settled wait and handed the PR to a human.
///
/// # Errors
/// [`harness_core::domain::Halt`] from the label or the comment, or a PR
/// with no number.
pub async fn resume_decided(gh: &dyn GitHub, pr: &Pr, landed: &[u64]) -> Outcome<()> {
    let number: u64 = pr.num.parse().map_err(|_| {
        harness_core::domain::Halt::Failed(format!("PR {:?} has no number to comment on", pr.num))
    })?;
    if landed.is_empty() {
        return Ok(());
    }
    let markers: String = landed.iter().map(|on| resumed_marker(*on)).collect();
    let named = landed
        .iter()
        .map(|on| format!("#{on}"))
        .collect::<Vec<_>>()
        .join(", ");
    gh.add_label(number, crate::common::labels::PR_FIX).await?;
    gh.post_issue_comment(
        number,
        &format!(
            "{markers}\nWhat this PR waited on landed on its base ({named}). The repairs of this \
             PR are counted anew: one repair follows, on its base as it stands now — bring the \
             branch up to its base first."
        ),
    )
    .await
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
    /// Its repair stopped on a contract another milestone produces: it
    /// waits until the contract change is closed and its base holds the
    /// manifest ([`contract_landed`]).
    Contract {
        /// The `harness:contract-change` issue.
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
            Self::Contract { issue } => {
                format!("contract change #{issue} (until its base holds the new manifest)")
            }
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
/// the issue still blocks and no person holds it ([`wait_stands`]).
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
        if let Some(on) = contract_waits(&comments).first() {
            found.push((pr.clone(), PrWait::Contract { issue: *on }));
            continue;
        }
        for on in waits_named(&comments) {
            if wait_stands(&gh.issue(on).await?) {
                found.push((pr.clone(), PrWait::Decided { issue: on }));
                break;
            }
        }
    }
    Ok(found)
}

/// Whether `pr` waits, read from its comments and the open rules' issues.
///
/// `comments` are its comments as read: its open contest, a contract change its
/// repair stopped on not resumed yet, or an issue a decided `wait` named
/// that still stands ([`wait_stands`]). What keeps a PR's escalation from being paid while it
/// waits, where the router's own reads of the waits do not reach — a
/// landing PR past its repairs carries no label the router lists.
///
/// # Errors
/// [`harness_core::domain::Halt`] from an issue read.
pub async fn held(gh: &dyn GitHub, pr: &Pr, comments: &str) -> Outcome<bool> {
    if !contract_waits(comments).is_empty() {
        return Ok(true);
    }
    for on in waits_named(comments) {
        if wait_stands(&gh.issue(on).await?) {
            return Ok(true);
        }
    }
    let Ok(number) = pr.num.parse::<u64>() else {
        return Ok(false);
    };
    let rules = gh
        .issues_labelled(crate::common::labels::RULES, "open")
        .await?;
    Ok(open_contest(number, &rules).is_some())
}

/// A contract change a waiting PR can resume from: closed, and the PR's
/// base holds what it changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractLanded {
    /// The `harness:contract-change` issue, closed.
    pub request: u64,
    /// The PR that waited on it (`500`).
    pub pr: String,
}

/// Whether `path` is the manifest of `id` (`<system>.<name>`): under
/// `crates/<system>/`, in the unit's folder (`kebab(name)`), named as a
/// manifest is.
fn is_manifest_of(path: &str, id: &str) -> bool {
    is_in_unit(path, id) && crate::main_agent_merge::data::merge_train::is_manifest(path)
}

/// Whether `path` is a file of the unit `id` (`<system>.<name>`): under
/// `crates/<system>/`, in the unit's folder (`kebab(name)`).
fn is_in_unit(path: &str, id: &str) -> bool {
    let Some((system, name)) = id.split_once('.') else {
        return false;
    };
    let folder = format!("/{}/", harness_architecture::skeleton::kebab(name));
    path.starts_with(&format!("crates/{system}/")) && path.contains(&folder)
}

/// Whether the integration branch changed no file `touched` matches since
/// `base` parted from it — `base` holds what it has. A compare GitHub lists
/// in part, or cannot read, is "no".
async fn base_holds(
    gh: &dyn GitHub,
    base: &str,
    integration: &str,
    touched: impl Fn(&str) -> bool,
) -> bool {
    base == integration
        || matches!(
            gh.changed_files(base, integration).await,
            Ok(Some(files)) if !files.iter().any(|path| touched(path))
        )
}

/// Where the producer task a contract change recorded stands against the
/// PR's base ([`landed_on_base`], the unit's folder as what it touched):
/// built under the milestone the request names, else the one its native
/// milestone mirrors.
///
/// # Errors
/// [`harness_core::domain::Halt`] from an issue read.
async fn producer_landing(
    gh: &dyn GitHub,
    pr: &Pr,
    producer: crate::contract_manager::data::request::Producer,
    id: &str,
    integration: &str,
) -> Outcome<Landing> {
    let task = gh.issue(producer.task).await?;
    let home = producer.milestone.or(task.milestone);
    landed_on_base(
        gh,
        pr,
        &task,
        home,
        |path| is_in_unit(path, id),
        integration,
    )
    .await
}

/// The PRs of `waiting` whose contract change landed.
///
/// Those that wait on a contract change ([`PrWait::Contract`])
/// and may resume now: the change closed, and its work reached the PR's
/// base. With a producer task recorded on the change
/// (`contract_manager::data::request::producer`), that task landed there
/// ([`landed_on_base`]: closed or `waiting-merge`, never cut — built under
/// another milestone than the PR's, that milestone landed too and the base
/// holds the unit's folder as the integration branch has it): the
/// manifest alone is not the contract built (`ForgeDesMondes`' #580,
/// 2026-10-25: resumed on #581's v2 manifest while #582, which built it,
/// had not landed — `rules-hold` stayed red and its repairs burned). A
/// producer cut is listed too: [`resume_contract`] hands the PR to a
/// human instead of resuming it. A producer under a milestone that waits
/// on the PR's own ([`Landing::Cycle`]) could never land first: the PR
/// reads the manifest then, as with no producer recorded — its consumers
/// read the fixture that stands for it. With none — handed back, or a
/// request written before the line — the base holds the manifest of its
/// id: the integration branch changed no manifest of it since they parted
/// (`changed_files(base, integration)`, after the base's catch-up). A
/// change handed back changed nothing: its base holds it at once. A
/// compare GitHub lists in part, or cannot read, is "not yet".
///
/// A PR resumed while its base still lacks the change would stop again on
/// the same contract and ask a second change of it (`ForgeDesMondes`' #482:
/// the contract manager was asked twice).
///
/// # Errors
/// [`harness_core::domain::Halt`] from an issue read.
pub async fn contract_landed(
    gh: &dyn GitHub,
    waiting: &[(Pr, PrWait)],
    integration: &str,
) -> Outcome<Vec<ContractLanded>> {
    let mut found = Vec::new();
    for (pr, wait) in waiting {
        let PrWait::Contract { issue } = wait else {
            continue;
        };
        let request = gh.issue(*issue).await?;
        if request.is_open() {
            continue;
        }
        let id = crate::contract_manager::data::request::Request::parse(&request.body)
            .map(|asked| asked.id)
            .unwrap_or_default();
        let manifest = |path: &str| is_manifest_of(path, &id);
        let holds = match crate::contract_manager::data::request::producer(&request.body) {
            Some(producer) => match producer_landing(gh, pr, producer, &id, integration).await? {
                Landing::Landed | Landing::Cut(_) => true,
                Landing::NotYet => false,
                Landing::Cycle(_) => base_holds(gh, &pr.base, integration, manifest).await,
            },
            None => base_holds(gh, &pr.base, integration, manifest).await,
        };
        if holds {
            found.push(ContractLanded {
                request: *issue,
                pr: pr.num.clone(),
            });
        }
    }
    Ok(found)
}

/// Resumes a PR whose contract change landed.
///
/// `harness:pr-fix` first, then the resumed marker — which clears the wait
/// and counts its repairs anew — with what the contract manager said
/// closing the change (a new version to build against, or why nothing
/// changed), so the repair reads it on the PR: one repair against the new
/// contract. In that order so a stop between the two leaves the PR
/// waiting, labelled, and resumed again at the next tick — never cleared
/// with no label the router repairs.
///
/// A change whose producer task was cut ([`was_cut`], the task or its
/// milestone) is no landing: nothing will build it, and the PR is handed
/// to a human ([`hand_over_cut`]), still waiting.
///
/// # Errors
/// [`harness_core::domain::Halt`] from a read, the comment or the label.
pub async fn resume_contract(
    gh: &dyn GitHub,
    landed: &ContractLanded,
    log: &Logbook,
) -> Outcome<()> {
    let number: u64 = landed.pr.parse().map_err(|_| {
        harness_core::domain::Halt::Failed(format!(
            "PR {:?} has no number to comment on",
            landed.pr
        ))
    })?;
    let request = gh.issue(landed.request).await?;
    if let Some(producer) = crate::contract_manager::data::request::producer(&request.body)
        && let Some(cut) = producer_cut(gh, producer).await?
    {
        return hand_over_cut(gh, landed, number, cut, log).await;
    }
    let said = gh.issue_comments(landed.request).await?.last().map_or_else(
        || "(the change closed without a word)".to_string(),
        |last| last.trim().to_string(),
    );
    gh.add_label(number, crate::common::labels::PR_FIX).await?;
    gh.post_issue_comment(
        number,
        &format!(
            "{}\nContract change #{} closed, and this PR's base holds it now — the task that \
             builds it landed there, when the change opened one. What it said:\n\n> {}\n\nThe \
             repairs of this PR are counted anew: one repair follows, against the contract as \
             it stands.",
            resumed_marker(landed.request),
            landed.request,
            said.replace('\n', "\n> ")
        ),
    )
    .await?;
    log.say(&format!(
        "watch: contract change #{} landed -> PR #{} resumed, its repairs counted anew",
        landed.request, landed.pr
    ));
    Ok(())
}

/// The producer task, or the milestone it was built under, when it was cut
/// ([`was_cut`]); `None` while its work may still come.
///
/// # Errors
/// [`harness_core::domain::Halt`] from an issue read.
async fn producer_cut(
    gh: &dyn GitHub,
    producer: crate::contract_manager::data::request::Producer,
) -> Outcome<Option<u64>> {
    let task = gh.issue(producer.task).await?;
    if was_cut(&task) {
        return Ok(Some(task.number));
    }
    let Some(home) = producer.milestone.or(task.milestone) else {
        return Ok(None);
    };
    Ok(was_cut(&gh.issue(home).await?).then_some(home))
}

/// Hands a PR whose contract change's producer was cut (`cut`) to a human:
/// the task its body closes, else the milestone whose landing PR it is,
/// else the PR itself — told why, labelled `harness:human`, once. The PR
/// keeps waiting: no repair is paid against a contract nothing will build.
///
/// # Errors
/// [`harness_core::domain::Halt`] from a read, the comment or the label.
async fn hand_over_cut(
    gh: &dyn GitHub,
    landed: &ContractLanded,
    number: u64,
    cut: u64,
    log: &Logbook,
) -> Outcome<()> {
    let owner = match crate::common::delivery::closed_task(&gh.issue(number).await?.body) {
        Some(task) => task,
        None => branching::milestone_number(&gh.pr(&landed.pr).await?.head).unwrap_or(number),
    };
    if gh.issue(owner).await?.has(crate::common::labels::HUMAN) {
        log.debug(&format!(
            "watch: PR #{} waits on contract change #{}, whose producer #{cut} was cut — #{owner} \
             is a human's already",
            landed.pr, landed.request
        ));
        return Ok(());
    }
    gh.post_issue_comment(
        owner,
        &format!(
            "PR #{} waits on contract change #{}, closed — but #{cut}, which was to build it, \
             was cut: nothing will build it, and the harness stops spending on it. A human \
             decides: reopen #{cut}, build it by hand, or cut this task. To let the PR be \
             repaired against its base as it stands, post `harness-resumed: #{}` on it as an \
             HTML comment. Then remove `{}`.",
            landed.pr,
            landed.request,
            landed.request,
            crate::common::labels::HUMAN
        ),
    )
    .await?;
    gh.add_label(owner, crate::common::labels::HUMAN).await?;
    log.warn(&format!(
        "watch: contract change #{} closed but its producer #{cut} was cut -> PR #{} handed to \
         a human (#{owner})",
        landed.request, landed.pr
    ));
    Ok(())
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

        #[test]
        fn a_contract_wait_reads_back_until_its_resume_and_counts_as_a_wait(
            on in 1u64..100_000,
            other in 1u64..100_000,
            decided in 0usize..3,
        ) {
            let decided_waits: String =
                (0..decided).fold(String::new(), |mut all, n| {
                    all.push_str(&waits_on_marker(900_000 + n as u64));
                    all.push('\n');
                    all
                });
            let waited = format!("{decided_waits}stopped\n{}\nwhy", contract_wait_marker(on));
            prop_assert_eq!(contract_waits(&waited), vec![on]);
            prop_assert!(waits_named(&waited).iter().all(|named| *named != on));
            prop_assert_eq!(waits_decided(&waited), decided + 1, "both kinds count in the cap");
            let resumed = format!("{waited}\n{}", resumed_marker(on));
            prop_assert!(contract_waits(&resumed).is_empty());
            prop_assert_eq!(waits_decided(&resumed), decided + 1, "a resume is no wait");
            let twice = format!("{waited}\n{}", contract_wait_marker(on));
            prop_assert_eq!(contract_waits(&twice), vec![on], "named once");
            if other != on {
                let elsewhere = format!("{waited}\n{}", resumed_marker(other));
                prop_assert_eq!(contract_waits(&elsewhere), vec![on]);
            }
            let before = format!("{}\n{waited}", resumed_marker(on));
            prop_assert_eq!(contract_waits(&before), vec![on], "a resume before clears nothing");
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

    #[test]
    fn the_latest_closed_contest_is_this_pr_s_and_closed() {
        let closed = vec![
            rules_issue(517, "closed", "Rules contested: PR #516"),
            rules_issue(600, "closed", "Rules contested: PR #12"),
            rules_issue(700, "open", "Rules contested: PR #516 (2)"),
        ];
        assert_eq!(latest_closed_contest(516, &closed), Some(517));
    }

    #[test]
    fn only_rules_json_and_the_rules_tests_are_the_rules() {
        assert!(is_rules_path("rules.json"));
        assert!(is_rules_path("tests/rules/business/tests/r10.rs"));
        for path in ["src/a.rs", "crates/x/rules.json", "tests/other/r10.rs"] {
            assert!(!is_rules_path(path), "{path}");
        }
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

    #[tokio::test]
    async fn a_decided_wait_stands_while_its_issue_still_blocks_and_no_person_holds_it() {
        let target = |state: &str, labelled: &[&str]| Issue {
            number: 567,
            state: state.to_string(),
            labels: labelled.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        };
        // (state, labels) -> (stands, stuck)
        for (state, labelled, stands, stuck) in [
            ("open", &[][..], true, false),
            ("open", &[labels::WAITING_MERGE][..], false, false),
            ("open", &[labels::HUMAN][..], false, true),
            ("closed", &[][..], false, false),
            ("closed", &[labels::HUMAN][..], false, false),
        ] {
            let issue = target(state, labelled);
            assert_eq!(wait_stands(&issue), stands, "{state} {labelled:?}");
            assert_eq!(wait_stuck(&issue), stuck, "{state} {labelled:?}");
        }
        // Open, #567 holds PR #569; landed (`waiting-merge`, delivered on
        // its branch) it lets it go to the escalation, which reads whether
        // it reached the base; held by a person too — read as stuck there.
        for (labelled, holds) in [
            (&[][..], true),
            (&[labels::WAITING_MERGE][..], false),
            (&[labels::HUMAN][..], false),
        ] {
            let gh = FakeGitHub {
                issues: vec![target("open", labelled)],
                pr_comment_bodies: vec![("569".to_string(), waits_on_marker(567))],
                ..FakeGitHub::default()
            };
            let prs = [pr("569", "feat/scene-board", "milestone/28-x")];
            let found = waiting(&gh, &prs, &[], &[]).await.expect("read");
            assert_eq!(found.len(), usize::from(holds), "{labelled:?}");
            assert_eq!(
                held(&gh, &prs[0], &waits_on_marker(567))
                    .await
                    .expect("read"),
                holds,
                "{labelled:?}"
            );
        }
    }

    #[test]
    fn a_cut_issue_is_closed_with_nothing_delivered() {
        let closed = |not_planned: bool, verdict: &str| Issue {
            number: 582,
            state: "closed".to_string(),
            not_planned,
            body: format!(
                "## Decision\n\nkind: review\nblocks: x\nround: 1\ndecided-by: ai\nverdict: \
                 {verdict}\n"
            ),
            ..Issue::default()
        };
        assert!(was_cut(&closed(true, "repair — x (4/5)")), "not planned");
        assert!(
            was_cut(&closed(false, "cut — out of scope (4/5)")),
            "a decided cut"
        );
        assert!(!was_cut(&closed(false, "repair — x (4/5)")), "merged");
        let mut open = closed(true, "cut — out of scope (4/5)");
        open.state = "open".to_string();
        assert!(!was_cut(&open), "open is never cut");
        assert!(!was_cut(&Issue {
            state: "closed".to_string(),
            ..Issue::default()
        }));
    }

    #[test]
    fn a_milestone_is_its_own_home_and_a_task_lives_on_its_native_milestone() {
        let milestone = Issue {
            number: 27,
            labels: vec![labels::MILESTONE.to_string()],
            milestone: Some(3),
            ..Issue::default()
        };
        assert_eq!(home_of(&milestone), Some(27));
        let task = Issue {
            number: 567,
            milestone: Some(28),
            ..Issue::default()
        };
        assert_eq!(home_of(&task), Some(28));
        assert_eq!(home_of(&Issue::default()), None);
    }

    /// An issue for [`landed_on_base`]: its number, state, labels and home.
    fn waited(number: u64, state: &str, labelled: &[&str], home: Option<u64>) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            labels: labelled.iter().map(|l| (*l).to_string()).collect(),
            milestone: home,
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn landed_is_on_the_pr_s_base_never_merely_closed_elsewhere() {
        let pr_569 = pr("569", "feat/scene-board", "milestone/28-scenes");
        let read = |gh: FakeGitHub, target: Issue| {
            let pr = &pr_569;
            async move {
                let home = home_of(&target);
                landed_on_base(&gh, pr, &target, home, |_| true, "main_agent")
                    .await
                    .expect("read")
            }
        };
        // Under the PR's own milestone: closed or waiting-merge is there.
        for (state, labelled, landing) in [
            ("open", &[][..], Landing::NotYet),
            ("open", &[labels::WAITING_MERGE][..], Landing::Landed),
            ("closed", &[][..], Landing::Landed),
        ] {
            assert_eq!(
                read(
                    FakeGitHub::default(),
                    waited(567, state, labelled, Some(28))
                )
                .await,
                landing,
                "{state} {labelled:?}"
            );
        }
        // Under milestone #27: closed is merged into milestone/27 only.
        let elsewhere = |milestone: Issue, behind: Vec<String>| FakeGitHub {
            issues: vec![milestone],
            changed_files: HashMap::from([(
                "milestone/28-scenes...main_agent".to_string(),
                Some(behind),
            )]),
            ..FakeGitHub::default()
        };
        let task = waited(567, "closed", &[], Some(27));
        assert_eq!(
            read(
                elsewhere(waited(27, "open", &[labels::MILESTONE], None), Vec::new()),
                task.clone()
            )
            .await,
            Landing::NotYet,
            "#27 still building"
        );
        assert_eq!(
            read(
                elsewhere(
                    waited(
                        27,
                        "open",
                        &[labels::MILESTONE, labels::WAITING_MERGE],
                        None
                    ),
                    vec!["crates/scene/src/lib.rs".to_string()]
                ),
                task.clone()
            )
            .await,
            Landing::NotYet,
            "#27 landed, the base not caught up"
        );
        assert_eq!(
            read(
                elsewhere(
                    waited(
                        27,
                        "open",
                        &[labels::MILESTONE, labels::WAITING_MERGE],
                        None
                    ),
                    Vec::new()
                ),
                task.clone()
            )
            .await,
            Landing::Landed,
            "#27 landed (waiting-merge), the base caught up"
        );
        assert_eq!(
            read(
                elsewhere(waited(27, "closed", &[labels::MILESTONE], None), Vec::new()),
                task.clone()
            )
            .await,
            Landing::Landed,
            "#27 promoted"
        );
        // A milestone waited on itself: landed on the integration branch.
        assert_eq!(
            read(
                FakeGitHub::default(),
                waited(
                    27,
                    "open",
                    &[labels::MILESTONE, labels::WAITING_MERGE],
                    None
                )
            )
            .await,
            Landing::Landed
        );
    }

    #[tokio::test]
    async fn a_cut_or_homeless_wait_lands_by_its_own_rule() {
        let pr_569 = pr("569", "feat/scene-board", "milestone/28-scenes");
        let read = |gh: FakeGitHub, target: Issue| {
            let pr = &pr_569;
            async move {
                let home = home_of(&target);
                landed_on_base(&gh, pr, &target, home, |_| true, "main_agent")
                    .await
                    .expect("read")
            }
        };
        let elsewhere = |milestone: Issue, behind: Vec<String>| FakeGitHub {
            issues: vec![milestone],
            changed_files: HashMap::from([(
                "milestone/28-scenes...main_agent".to_string(),
                Some(behind),
            )]),
            ..FakeGitHub::default()
        };
        let task = waited(567, "closed", &[], Some(27));
        // Cut: the task, or the milestone it was built under.
        let mut cut = waited(567, "closed", &[], Some(28));
        cut.not_planned = true;
        assert_eq!(read(FakeGitHub::default(), cut).await, Landing::Cut(567));
        let mut cut_27 = waited(27, "closed", &[labels::MILESTONE], None);
        cut_27.not_planned = true;
        assert_eq!(
            read(elsewhere(cut_27, Vec::new()), task.clone()).await,
            Landing::Cut(27)
        );
        // No home known: landed, and the base holding what it changed.
        assert_eq!(
            read(
                elsewhere(waited(1, "open", &[], None), vec!["x".to_string()]),
                waited(567, "closed", &[], None)
            )
            .await,
            Landing::NotYet
        );
        assert_eq!(
            read(FakeGitHub::default(), waited(567, "open", &[], None)).await,
            Landing::NotYet
        );
        assert_eq!(
            read(FakeGitHub::default(), waited(567, "closed", &[], None)).await,
            Landing::Landed
        );
    }

    #[tokio::test]
    async fn a_milestone_that_waits_on_the_pr_s_own_is_a_cycle_never_a_wait() {
        // #27 is blocked by #28, the PR's own milestone, still building:
        // #567 under #27 cannot land before PR #569 does.
        let pr_569 = pr("569", "feat/scene-board", "milestone/28-scenes");
        let board = |blocker_state: &str| {
            let mut milestone_27 = waited(27, "open", &[labels::MILESTONE], None);
            milestone_27.blocked_by = vec![waited(28, blocker_state, &[labels::MILESTONE], None)];
            FakeGitHub {
                issues: vec![milestone_27.clone()],
                subs: vec![(1, vec![milestone_27])],
                ..FakeGitHub::default()
            }
        };
        for (blocker, landing) in [("open", Landing::Cycle(27)), ("closed", Landing::NotYet)] {
            let gh = board(blocker);
            let target = waited(567, "open", &[], Some(27));
            assert_eq!(
                landed_on_base(&gh, &pr_569, &target, Some(27), |_| true, "main_agent")
                    .await
                    .expect("read"),
                landing,
                "#28 {blocker}"
            );
        }
    }

    #[tokio::test]
    async fn a_settled_decided_wait_resumes_its_pr_once_with_one_repair() {
        let gh = FakeGitHub::default();
        let pr_569 = pr("569", "feat/scene-board", "milestone/28-x");
        resume_decided(&gh, &pr_569, &[567, 571])
            .await
            .expect("resumed");
        let writes = gh.writes();
        let said = writes
            .iter()
            .find_map(|w| match w {
                Wrote::Comment(569, said) => Some(said.clone()),
                _ => None,
            })
            .expect("the PR told");
        assert!(
            said.starts_with(&format!("{}{}", resumed_marker(567), resumed_marker(571))),
            "{said}"
        );
        assert!(said.contains("landed on its base (#567, #571)"), "{said}");
        assert_eq!(
            waits_named(&format!(
                "{}{}\n{said}",
                waits_on_marker(567),
                waits_on_marker(571)
            )),
            [] as [u64; 0]
        );
        // The label first: a stop between the two leaves a label without a
        // marker — resumed again by the next escalation — never a PR
        // cleared of its waits with no label the router repairs.
        assert_eq!(
            writes,
            [
                Wrote::Label(569, labels::PR_FIX.to_string()),
                Wrote::Comment(569, said.clone())
            ]
        );
        let refused = FakeGitHub {
            refused_comments: vec![569],
            ..FakeGitHub::default()
        };
        assert!(resume_decided(&refused, &pr_569, &[567]).await.is_err());
        assert_eq!(
            refused.writes(),
            [Wrote::Label(569, labels::PR_FIX.to_string())]
        );
        // Nothing landed: nothing written; a PR with no number halts.
        let quiet = FakeGitHub::default();
        resume_decided(&quiet, &pr_569, &[]).await.expect("nothing");
        assert_eq!(quiet.writes(), [] as [Wrote; 0]);
        assert!(
            resume_decided(&quiet, &pr("x", "feat/a", "milestone/28-x"), &[567])
                .await
                .is_err()
        );
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
    async fn a_contest_of_a_pr_no_longer_open_never_settles_another_pr() {
        let gh = settled_board(false);
        let closed = [rules_issue(517, "closed", "Rules contested: PR #516")];
        let other = [pr("530", "feat/other", "main_agent")];
        let found = settled(&gh, &closed, &[], &other, &[], "main_agent")
            .await
            .expect("read");
        assert_eq!(found, Vec::<Settled>::new(), "#530 never contested");
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
    async fn a_contest_of_a_closed_milestone_lifts_nothing() {
        let closed_milestone = Issue {
            number: 28,
            state: "closed".to_string(),
            title: "x".to_string(),
            ..Issue::default()
        };
        let branch = branching::milestone_branch(28, "x");
        let gh = FakeGitHub {
            files: HashMap::from([(("CONTEST.md".to_string(), branch), "R10".to_string())]),
            ..FakeGitHub::default()
        };
        let closed = [rules_issue(
            459,
            "closed",
            "Rules contested: R10 on milestone/28",
        )];
        let found = settled(&gh, &closed, &[], &[], &[closed_milestone], "main_agent")
            .await
            .expect("read");
        assert_eq!(found, Vec::<Settled>::new(), "the milestone is closed");
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
        let (_, escalated) = crate::milestone_merge::ready_to_merge(
            &gh,
            &[task486()],
            &open,
            &[],
            "main_agent",
            &Logbook::null(),
        )
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

    // --- contract waits: landed, resumed --------------------------------

    fn request(number: u64, state: &str, id: &str) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            body: crate::contract_manager::data::request::Request {
                id: id.to_string(),
                task: 482,
                milestone: Some(22),
                need: "a source field".to_string(),
            }
            .render(),
            ..Issue::default()
        }
    }

    const MANIFEST: &str = "crates/session/src/capabilities/list-scenes/capability.json";

    #[test]
    fn a_manifest_of_the_id_is_under_its_system_and_its_unit_s_folder() {
        assert!(is_manifest_of(MANIFEST, "session.listScenes"));
        assert!(
            !is_manifest_of(MANIFEST, "campaign.listScenes"),
            "another system"
        );
        assert!(
            !is_manifest_of(MANIFEST, "session.listCampaigns"),
            "another unit"
        );
        assert!(
            !is_manifest_of(
                "crates/session/src/capabilities/list-scenes/handler.rs",
                "session.listScenes"
            ),
            "no manifest"
        );
        assert!(!is_manifest_of(MANIFEST, "Session"), "an id with no system");
    }

    #[tokio::test]
    async fn a_contract_change_lands_once_closed_and_held_by_the_pr_s_base() {
        let waiting_on = |pr: Pr, issue: u64| (pr, PrWait::Contract { issue });
        let task_pr = pr("500", "feat/fiche", "milestone/22-fiche");
        let landing = pr("423", "milestone/22-fiche", "main_agent");
        let gh = FakeGitHub {
            issues: vec![
                request(503, "open", "session.listScenes"),
                request(504, "closed", "session.listScenes"),
                request(505, "closed", "session.listScenes"),
                request(506, "closed", "session.listScenes"),
                request(507, "closed", "session.listScenes"),
            ],
            changed_files: HashMap::from([
                (
                    "milestone/22-fiche...main_agent".to_string(),
                    Some(vec![MANIFEST.to_string()]),
                ),
                (
                    "milestone/23-x...main_agent".to_string(),
                    Some(vec!["crates/session/src/lib.rs".to_string()]),
                ),
                ("milestone/24-y...main_agent".to_string(), None),
            ]),
            broken_compares: vec!["milestone/25-z...main_agent".to_string()],
            ..FakeGitHub::default()
        };
        let waiting = [
            // Still open: not yet.
            waiting_on(task_pr.clone(), 503),
            // Closed, into the integration branch itself: held at once.
            waiting_on(landing, 504),
            // Closed, the base still lacks the manifest: not yet.
            waiting_on(task_pr, 505),
            // Closed, the base differs on no manifest of it: landed.
            waiting_on(pr("510", "feat/a", "milestone/23-x"), 506),
            // A compare listed in part, or unreadable: not yet.
            waiting_on(pr("511", "feat/b", "milestone/24-y"), 507),
            waiting_on(pr("512", "feat/c", "milestone/25-z"), 507),
            // Another kind of wait: never read here.
            (
                pr("513", "feat/d", "milestone/23-x"),
                PrWait::Decided { issue: 503 },
            ),
        ];
        let landed = contract_landed(&gh, &waiting, "main_agent")
            .await
            .expect("read");
        assert_eq!(
            landed,
            [
                ContractLanded {
                    request: 504,
                    pr: "423".to_string(),
                },
                ContractLanded {
                    request: 506,
                    pr: "510".to_string(),
                },
            ]
        );
    }

    #[test]
    fn a_file_of_the_unit_is_any_file_in_its_folder_under_its_system() {
        assert!(is_in_unit(MANIFEST, "session.listScenes"));
        assert!(is_in_unit(
            "crates/session/src/capabilities/list-scenes/handler.rs",
            "session.listScenes"
        ));
        assert!(
            !is_in_unit(MANIFEST, "campaign.listScenes"),
            "another system"
        );
        assert!(
            !is_in_unit(MANIFEST, "session.listCampaigns"),
            "another unit"
        );
        assert!(!is_in_unit(MANIFEST, "Session"), "an id with no system");
    }

    /// Contract change #581 as the contract manager closed it: v2 of
    /// `campaign.listCampaigns`, built by `producer` (#582 under #30).
    fn request_581(producer: Option<(u64, Option<u64>)>) -> Issue {
        let mut issue = request(581, "closed", "campaign.listCampaigns");
        if let Some((task, milestone)) = producer {
            issue.body = crate::contract_manager::data::request::with_producer(
                &issue.body,
                crate::contract_manager::data::request::Producer { task, milestone },
            );
        }
        issue
    }

    const LIST_CAMPAIGNS: &str = "crates/campaign/capabilities/list-campaigns/capability.json";

    /// How many of `waiting` landed on `gh`'s board.
    async fn landed(gh: FakeGitHub, waiting: &[(Pr, PrWait)]) -> usize {
        contract_landed(&gh, waiting, "main_agent")
            .await
            .expect("read")
            .len()
    }

    #[tokio::test]
    async fn the_replay_of_580_lands_once_582_landed_not_once_the_manifest_did() {
        let pr_580 = pr(
            "580",
            "feat/campaign-list-micro-ui",
            "milestone/30-campaign-home",
        );
        let waiting = [(pr_580.clone(), PrWait::Contract { issue: 581 })];
        let producer_582 = |state: &str| Issue {
            number: 582,
            state: state.to_string(),
            ..Issue::default()
        };
        // 15:55: the v2 manifest on the base, #582 not built — not landed
        // (the harness resumed it then, and its repairs burned).
        let gh = FakeGitHub {
            issues: vec![request_581(Some((582, Some(30)))), producer_582("open")],
            changed_files: HashMap::from([(
                "milestone/30-campaign-home...main_agent".to_string(),
                Some(Vec::new()),
            )]),
            ..FakeGitHub::default()
        };
        assert_eq!(
            contract_landed(&gh, &waiting, "main_agent")
                .await
                .expect("read"),
            [] as [ContractLanded; 0]
        );
        // #582 merged into milestone/30, the PR's own base: landed.
        let gh = FakeGitHub {
            issues: vec![request_581(Some((582, Some(30)))), producer_582("closed")],
            ..FakeGitHub::default()
        };
        assert_eq!(
            contract_landed(&gh, &waiting, "main_agent")
                .await
                .expect("read"),
            [ContractLanded {
                request: 581,
                pr: "580".to_string(),
            }]
        );
        // Built under another milestone (#27): its landing first — on the
        // integration branch, `waiting-merge` and still open until the
        // promotion — then the base holding the unit as the integration
        // branch has it.
        let elsewhere = |landed_27: bool, behind: Vec<String>| FakeGitHub {
            issues: vec![
                request_581(Some((582, Some(27)))),
                producer_582("closed"),
                Issue {
                    number: 27,
                    state: "open".to_string(),
                    labels: if landed_27 {
                        vec![
                            labels::MILESTONE.to_string(),
                            labels::WAITING_MERGE.to_string(),
                        ]
                    } else {
                        vec![labels::MILESTONE.to_string()]
                    },
                    ..Issue::default()
                },
            ],
            changed_files: HashMap::from([(
                "milestone/30-campaign-home...main_agent".to_string(),
                Some(behind),
            )]),
            ..FakeGitHub::default()
        };
        let read = |gh: FakeGitHub| landed(gh, &waiting);
        assert_eq!(
            read(elsewhere(false, Vec::new())).await,
            0,
            "#27 not landed"
        );
        assert_eq!(
            read(elsewhere(
                true,
                vec!["crates/campaign/capabilities/list-campaigns/src/lib.rs".to_string()]
            ))
            .await,
            0,
            "the base lacks the built unit"
        );
        assert_eq!(
            read(elsewhere(true, vec!["crates/media/src/lib.rs".to_string()])).await,
            1,
            "#27 landed (waiting-merge) and the base holds the unit"
        );
        // #27 promoted to main: closed, landed all the same.
        let mut promoted = elsewhere(true, Vec::new());
        promoted.issues[2].state = "closed".to_string();
        assert_eq!(read(promoted).await, 1, "#27 closed");
    }

    #[tokio::test]
    async fn the_replay_of_580_reads_the_manifest_when_582_s_milestone_waits_on_its_own() {
        // #27 itself waits on #30, the PR's own milestone: it could never
        // land first — the manifest is read, as with no producer.
        let pr_580 = pr(
            "580",
            "feat/campaign-list-micro-ui",
            "milestone/30-campaign-home",
        );
        let waiting = [(pr_580, PrWait::Contract { issue: 581 })];
        let read = |gh: FakeGitHub| landed(gh, &waiting);
        let cycle = |behind: Vec<String>| {
            let mut gh = FakeGitHub {
                issues: vec![
                    request_581(Some((582, Some(27)))),
                    Issue {
                        number: 582,
                        state: "closed".to_string(),
                        ..Issue::default()
                    },
                    Issue {
                        number: 27,
                        state: "open".to_string(),
                        labels: vec![labels::MILESTONE.to_string()],
                        ..Issue::default()
                    },
                ],
                changed_files: HashMap::from([(
                    "milestone/30-campaign-home...main_agent".to_string(),
                    Some(behind),
                )]),
                ..FakeGitHub::default()
            };
            let mut milestone_27 = gh.issues[2].clone();
            milestone_27.blocked_by = vec![Issue {
                number: 30,
                state: "open".to_string(),
                ..Issue::default()
            }];
            gh.subs = vec![(12, vec![milestone_27])];
            gh
        };
        assert_eq!(
            read(cycle(Vec::new())).await,
            1,
            "the base holds the manifest"
        );
        assert_eq!(
            read(cycle(vec![LIST_CAMPAIGNS.to_string()])).await,
            0,
            "the base lacks the manifest"
        );
        // A request with no producer recorded reads the manifest, as before.
        let gh = FakeGitHub {
            issues: vec![request_581(None)],
            changed_files: HashMap::from([(
                "milestone/30-campaign-home...main_agent".to_string(),
                Some(vec![LIST_CAMPAIGNS.to_string()]),
            )]),
            ..FakeGitHub::default()
        };
        assert_eq!(read(gh).await, 0, "the base lacks the manifest");
    }

    #[tokio::test]
    async fn a_landing_pr_holds_a_producer_built_under_its_own_milestone_once_closed() {
        // The landing PR of #30 waits on #581: #582 merged into its head,
        // milestone/30 — #30 itself still open, since it is what lands.
        let landing_30 = [(
            pr("590", "milestone/30-campaign-home", "main_agent"),
            PrWait::Contract { issue: 581 },
        )];
        let board = |producer_state: &str| FakeGitHub {
            issues: vec![
                request_581(Some((582, Some(30)))),
                Issue {
                    number: 582,
                    state: producer_state.to_string(),
                    ..Issue::default()
                },
                Issue {
                    number: 30,
                    state: "open".to_string(),
                    ..Issue::default()
                },
            ],
            ..FakeGitHub::default()
        };
        assert_eq!(landed(board("open"), &landing_30).await, 0);
        assert_eq!(landed(board("closed"), &landing_30).await, 1);
    }

    #[tokio::test]
    async fn the_replay_of_580_with_582_cut_hands_the_pr_to_a_human_never_resumes_it() {
        // #582 closed as not planned (or cut by a decision): nothing will
        // build v2 of `campaign.listCampaigns`. The PR is listed, and its
        // follow-up hands it to a human instead of resuming it.
        let pr_580 = pr(
            "580",
            "feat/campaign-list-micro-ui",
            "milestone/30-campaign-home",
        );
        let waiting = [(pr_580, PrWait::Contract { issue: 581 })];
        let board = |not_planned: bool, owner_labels: &[&str]| FakeGitHub {
            issues: vec![
                request_581(Some((582, Some(30)))),
                Issue {
                    number: 582,
                    state: "closed".to_string(),
                    not_planned,
                    body: if not_planned {
                        String::new()
                    } else {
                        "## Decision\n\nkind: review\nblocks: x\nround: 1\ndecided-by: ai\n\
                         verdict: cut — covered by #590 (4/5)\n"
                            .to_string()
                    },
                    ..Issue::default()
                },
                Issue {
                    number: 580,
                    body: "Closes #576".to_string(),
                    ..Issue::default()
                },
                Issue {
                    number: 576,
                    state: "open".to_string(),
                    labels: owner_labels.iter().map(|l| (*l).to_string()).collect(),
                    ..Issue::default()
                },
            ],
            ..FakeGitHub::default()
        };
        for not_planned in [true, false] {
            let gh = board(not_planned, &[labels::AGENT]);
            let found = contract_landed(&gh, &waiting, "main_agent")
                .await
                .expect("read");
            assert_eq!(found.len(), 1, "listed, to be handed over");
            resume_contract(&gh, &found[0], &Logbook::null())
                .await
                .expect("handed over");
            let writes = gh.writes();
            assert!(writes.contains(&Wrote::Label(576, labels::HUMAN.to_string())));
            assert!(
                writes.iter().any(|w| matches!(w, Wrote::Comment(576, said)
                    if said.contains("#582, which was to build it, was cut")
                        && !said.contains(RESUMED))),
                "{writes:?}"
            );
            assert!(
                !writes
                    .iter()
                    .any(|w| matches!(w, Wrote::Comment(580, _) | Wrote::Label(580, _))),
                "the PR keeps waiting, no repair: {writes:?}"
            );
        }
        // Once a human's, nothing more is said.
        let told = board(true, &[labels::AGENT, labels::HUMAN]);
        resume_contract(
            &told,
            &ContractLanded {
                request: 581,
                pr: "580".to_string(),
            },
            &Logbook::null(),
        )
        .await
        .expect("read");
        assert!(told.writes().is_empty(), "{:?}", told.writes());
    }

    #[tokio::test]
    async fn a_cut_producer_with_no_task_to_ask_hands_over_the_landing_milestone_or_the_pr() {
        let cut_582 = Issue {
            number: 582,
            state: "closed".to_string(),
            not_planned: true,
            ..Issue::default()
        };
        // A landing PR closes no task: its milestone is asked.
        let gh = FakeGitHub {
            issues: vec![
                request_581(Some((582, Some(27)))),
                cut_582.clone(),
                Issue {
                    number: 590,
                    ..Issue::default()
                },
                Issue {
                    number: 30,
                    state: "open".to_string(),
                    ..Issue::default()
                },
            ],
            prs: vec![(
                "590".to_string(),
                pr("590", "milestone/30-campaign-home", "main_agent"),
            )],
            ..FakeGitHub::default()
        };
        let landed = ContractLanded {
            request: 581,
            pr: "590".to_string(),
        };
        resume_contract(&gh, &landed, &Logbook::null())
            .await
            .expect("handed over");
        assert!(
            gh.writes()
                .contains(&Wrote::Label(30, labels::HUMAN.to_string()))
        );
        // Neither: the PR itself.
        let gh = FakeGitHub {
            issues: vec![
                request_581(Some((582, None))),
                cut_582,
                Issue {
                    number: 595,
                    ..Issue::default()
                },
            ],
            prs: vec![("595".to_string(), pr("595", "feat/x", "milestone/30-x"))],
            ..FakeGitHub::default()
        };
        resume_contract(
            &gh,
            &ContractLanded {
                request: 581,
                pr: "595".to_string(),
            },
            &Logbook::null(),
        )
        .await
        .expect("handed over");
        assert!(
            gh.writes()
                .contains(&Wrote::Label(595, labels::HUMAN.to_string()))
        );
    }

    #[tokio::test]
    async fn a_producer_is_cut_when_its_task_or_its_milestone_was() {
        // The producer's milestone cut, the task merged: cut all the same.
        let gh = FakeGitHub {
            issues: vec![
                request_581(Some((582, Some(27)))),
                Issue {
                    number: 582,
                    state: "closed".to_string(),
                    ..Issue::default()
                },
                Issue {
                    number: 27,
                    state: "closed".to_string(),
                    not_planned: true,
                    ..Issue::default()
                },
                Issue {
                    number: 595,
                    ..Issue::default()
                },
            ],
            prs: vec![("595".to_string(), pr("595", "feat/x", "milestone/30-x"))],
            ..FakeGitHub::default()
        };
        assert_eq!(
            producer_cut(
                &gh,
                crate::contract_manager::data::request::Producer {
                    task: 582,
                    milestone: Some(27)
                }
            )
            .await
            .expect("read"),
            Some(27)
        );
        assert_eq!(
            producer_cut(
                &gh,
                crate::contract_manager::data::request::Producer {
                    task: 582,
                    milestone: None
                }
            )
            .await
            .expect("read"),
            None
        );
    }

    #[tokio::test]
    async fn a_landed_change_resumes_its_pr_with_what_the_manager_said() {
        let gh = FakeGitHub {
            issues: vec![request(503, "closed", "session.listScenes")],
            issue_comments: vec![
                "working on it".to_string(),
                "session.listScenes v2 landed:\nsource added".to_string(),
            ],
            ..FakeGitHub::default()
        };
        let landed = ContractLanded {
            request: 503,
            pr: "500".to_string(),
        };
        resume_contract(&gh, &landed, &Logbook::null())
            .await
            .expect("resumed");
        let writes = gh.writes();
        let said = writes
            .iter()
            .find_map(|w| match w {
                Wrote::Comment(500, said) => Some(said.clone()),
                _ => None,
            })
            .expect("a comment on the PR");
        assert!(said.starts_with(&resumed_marker(503)), "{said}");
        assert!(
            said.contains("> session.listScenes v2 landed:\n> source added"),
            "the last comment, quoted: {said}"
        );
        // The label first: a stop before the marker leaves the PR waiting,
        // labelled, resumed again at the next tick.
        assert_eq!(
            writes,
            [
                Wrote::Label(500, labels::PR_FIX.to_string()),
                Wrote::Comment(500, said.clone())
            ]
        );
        assert_eq!(
            contract_waits(&format!("{}\n{said}", contract_wait_marker(503))),
            [] as [u64; 0]
        );
        let refused = FakeGitHub {
            issues: vec![request(503, "closed", "session.listScenes")],
            refused_comments: vec![500],
            ..FakeGitHub::default()
        };
        assert!(
            resume_contract(&refused, &landed, &Logbook::null())
                .await
                .is_err()
        );
        assert_eq!(
            refused.writes(),
            [Wrote::Label(500, labels::PR_FIX.to_string())]
        );
        // A change closed without a word says so; a PR with no number halts.
        let silent = FakeGitHub {
            issues: vec![request(503, "closed", "session.listScenes")],
            ..FakeGitHub::default()
        };
        resume_contract(&silent, &landed, &Logbook::null())
            .await
            .expect("resumed");
        assert!(silent.writes().iter().any(|w| matches!(
            w,
            Wrote::Comment(500, said) if said.contains("closed without a word")
        )));
        let unnumbered = ContractLanded {
            request: 503,
            pr: "x".to_string(),
        };
        assert!(
            resume_contract(&silent, &unnumbered, &Logbook::null())
                .await
                .is_err()
        );
    }
}
