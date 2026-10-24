//! Every building milestone branch brought up to the integration branch
//! each time the integration branch's head moves — and is green.
//!
//! What moved it does not matter: a milestone that landed, a rules contest
//! settled there, the plan's graph written, an upgrade, a human's push. The
//! watch reads that head every tick (one call, [`read_head`]), and while a
//! branch is behind it, that head's own checks (one more,
//! `GitHub::commit_checks`, until they conclude): the verdict is the
//! commit's, never the branch's newest run — which, seconds after a push,
//! still belongs to the head before. Once green — never merely "not red":
//! a base still running may turn red, and each catch-up push reruns every
//! milestone's CI — the router schedules one catch-up per branch behind it
//! (`routing::Route::CatchUp`, free, run in the tick, before any merge),
//! and each merges that very commit, not whatever the branch holds by then.
//!
//! One catch-up ([`catch_up`]): GitHub's merge first; on a refusal, the
//! same merge in the harness's own checkout, its generated files
//! regenerated (`branching::catch_up_locally`) — at most
//! [`LOCAL_PER_TICK`] such a tick, the rest wait for the next. Then:
//!
//! - **held** — every open task PR into the branch takes it in turn
//!   ([`refresh_task_prs`]), so its checks run against the new base;
//! - **residual** (a conflict no script solves) — the milestone's open
//!   landing PR is asked for a repair (`harness:pr-fix`, the repair cap
//!   applies, its prompt asks for a merge of the base); without one, it is
//!   said and left ([`route_residual`]). GitHub's merge alone is tried
//!   again every [`PARK_SECONDS`] at the same head — a hand or the repair
//!   may have merged the base meanwhile; the branch's tasks wait until it
//!   takes it, and without a landing PR each try says so again;
//! - **failed** — parked for [`PARK_SECONDS`], said once.
//!
//! What a watch remembers of it lives in [`CatchUps`], inside the router's
//! `Pulse`: pure, the time handed in. Two writers never meet on a branch: a
//! catch-up waits while a lane works on its milestone's landing PR (read
//! here, the label being gone once the repair started), the router holds
//! that repair while the catch-up is due, and the milestone's tasks while
//! its branch is behind or could not take the base; a task PR whose
//! session still runs is never refreshed under it ([`Busy`]).

use std::collections::{BTreeMap, BTreeSet};

use harness_core::domain::{Halt, Issue, Outcome, Pr};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Logbook;

use crate::common::branching::{self, CaughtUp};
use crate::common::labels;
use crate::dev_loop::data::board::Board;
use crate::dev_loop::data::tasks;

/// How long a catch-up that failed waits before it is tried again, and how
/// long a residual conflict waits before GitHub's merge is tried again at
/// the same head: an hour, as a lane's parked task does.
pub const PARK_SECONDS: u64 = 3600;

/// How many catch-ups a tick makes in the harness's own checkout: each one
/// a mount, a merge, the mock-up's bundles and `cargo metadata`, inside the
/// tick — the lanes wait for it to be refilled.
pub const LOCAL_PER_TICK: u32 = 1;

/// A conflict no script solves, met by a milestone's branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Residual {
    /// The integration branch's head it was met at.
    pub head: String,
    /// The second from which GitHub's merge is tried again at that head.
    pub retry_at: u64,
}

/// What a watch remembers of its catch-ups, tick to tick.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CatchUps {
    /// The integration branch's head, as last read.
    pub head: Option<String>,
    /// The verdict of that head's own checks — `Some(true)` green,
    /// `Some(false)` red — `None` until they concluded; read until green,
    /// forgotten at each move.
    pub verdict: Option<bool>,
    /// The head each milestone branch was last brought to by this watch.
    pub held: BTreeMap<u64, String>,
    /// The milestones whose branch met a conflict no script solves: at
    /// which head, and when GitHub's merge alone is tried again there.
    pub residual: BTreeMap<u64, Residual>,
    /// The second until which a failed catch-up waits.
    pub parked: BTreeMap<u64, u64>,
    /// The catch-ups in the harness's checkout this tick may still make.
    pub local_left: u32,
    /// Whether a building branch was behind the head at the last full
    /// read: the head's checks are read every tick while it is, until their
    /// verdict.
    pub behind_seen: bool,
    /// The catch-ups due at the last full read and not settled since — one
    /// left by the tick's single local catch-up, one whose push raced
    /// another: the board is read in full next tick for them.
    pub outstanding: BTreeSet<u64>,
}

impl CatchUps {
    /// Keeps the head read this tick: whether it moved since the last one
    /// (the first read is a move), its verdict forgotten if it did. An
    /// absent branch keeps the last head.
    pub fn moved(&mut self, head: Option<String>) -> bool {
        let Some(head) = head else {
            return false;
        };
        let moved = self.head.as_ref() != Some(&head);
        if moved {
            self.verdict = None;
        }
        self.head = Some(head);
        moved
    }

    /// A full read of the board at `now`: the catch-ups due, each milestone
    /// with its branch ([`Self::due`]), and the milestones whose branch
    /// could not take the base ([`Self::holding`]). Allows this tick its
    /// local catch-up, and remembers what is behind and what is due.
    pub fn read(&mut self, milestones: &[Issue], now: u64) -> (Vec<(u64, String)>, Vec<u64>) {
        let due = self.due(milestones, now);
        self.local_left = LOCAL_PER_TICK;
        self.behind_seen = !self.behind(milestones).is_empty();
        self.outstanding = due.iter().map(|(number, _)| *number).collect();
        (due, self.holding(now))
    }

    /// The building milestones whose branch this watch has not brought to
    /// the head yet, in number order.
    fn behind(&self, milestones: &[Issue]) -> Vec<(u64, String)> {
        let Some(head) = &self.head else {
            return Vec::new();
        };
        targets(milestones)
            .into_iter()
            .filter(|(number, _)| self.held.get(number) != Some(head))
            .collect()
    }

    /// Whether a failed catch-up of `milestone` still waits at `now`.
    fn parked_at(&self, milestone: u64, now: u64) -> bool {
        self.parked
            .get(&milestone)
            .is_some_and(|until| *until > now)
    }

    /// The catch-ups due at `now`, each milestone with its branch: those
    /// behind the head, not parked, and — met with a residual conflict at
    /// this head — only once its retry is due; none unless the head's own
    /// checks are green.
    #[must_use]
    pub fn due(&self, milestones: &[Issue], now: u64) -> Vec<(u64, String)> {
        if self.verdict != Some(true) {
            return Vec::new();
        }
        self.behind(milestones)
            .into_iter()
            .filter(|(number, _)| {
                !self.parked_at(*number, now)
                    && (!self.residual_here(*number)
                        || self
                            .residual
                            .get(number)
                            .is_some_and(|met| met.retry_at <= now))
            })
            .collect()
    }

    /// Whether `milestone`'s branch met a conflict no script solves at the
    /// head as it is.
    fn residual_here(&self, milestone: u64) -> bool {
        self.residual
            .get(&milestone)
            .is_some_and(|met| self.head.as_ref() == Some(&met.head))
    }

    /// The milestones whose branch could not take the base — a conflict no
    /// script solves, or a failure still parked at `now`: their tasks wait.
    #[must_use]
    pub fn holding(&self, now: u64) -> Vec<u64> {
        let mut held: Vec<u64> = self
            .residual
            .keys()
            .copied()
            .chain(
                self.parked
                    .iter()
                    .filter(|(_, until)| **until > now)
                    .map(|(number, _)| *number),
            )
            .collect();
        held.sort_unstable();
        held.dedup();
        held
    }

    /// What a catch-up of `milestone` to the head came to, remembered: a
    /// deferral alone leaves it due.
    pub fn record(&mut self, milestone: u64, caught: &CaughtUp, now: u64) {
        let Some(head) = self.head.clone() else {
            return;
        };
        match caught {
            CaughtUp::OnGitHub | CaughtUp::Locally(_) => {
                self.held.insert(milestone, head);
                self.residual.remove(&milestone);
                self.parked.remove(&milestone);
            }
            CaughtUp::Residual(_) => {
                self.residual.insert(
                    milestone,
                    Residual {
                        head,
                        retry_at: now.saturating_add(PARK_SECONDS),
                    },
                );
                self.parked.remove(&milestone);
            }
            CaughtUp::Failed(_) => {
                self.parked
                    .insert(milestone, now.saturating_add(PARK_SECONDS));
            }
            CaughtUp::Deferred => return,
        }
        self.outstanding.remove(&milestone);
    }

    /// A residual conflict GitHub still refuses at the same head: tried
    /// again in [`PARK_SECONDS`].
    fn retry_later(&mut self, milestone: u64, now: u64) {
        if let Some(met) = self.residual.get_mut(&milestone) {
            met.retry_at = now.saturating_add(PARK_SECONDS);
        }
        self.outstanding.remove(&milestone);
    }
}

/// Reads the integration branch's head, and its own checks while they are
/// awaited: whether this tick must read the board in full for the catch-ups.
///
/// The head is one call; its checks one more, while a building branch is
/// behind it — or it just moved — and it is not green: still running, or
/// red (a failed job run again may turn it green; a red base reads the
/// board every tick anyway). The board is read in full when the head moved,
/// when its verdict just came in or changed (a green that moves no issue),
/// or when a catch-up due is still unsettled ([`CatchUps::outstanding`]).
///
/// # Errors
/// Either read failing: the tick stops, as on any read of GitHub.
pub async fn read_head(
    gh: &dyn GitHub,
    catch_ups: &mut CatchUps,
    integration: &str,
) -> Outcome<bool> {
    let moved = catch_ups.moved(gh.branch_sha(integration).await?);
    let judged = if let Some(head) = catch_ups.head.clone()
        && catch_ups.verdict != Some(true)
        && (moved || catch_ups.behind_seen)
    {
        let verdict = gh.commit_checks(integration, &head).await?;
        let turned = verdict.is_some() && verdict != catch_ups.verdict;
        catch_ups.verdict = verdict;
        turned
    } else {
        false
    };
    Ok(moved || judged || !catch_ups.outstanding.is_empty())
}

/// The milestones a catch-up keeps up, with their branch, in number order:
/// open, split (`harness:triggered`, so the branch exists), not landed yet
/// (`harness:waiting-merge`).
#[must_use]
pub fn targets(milestones: &[Issue]) -> Vec<(u64, String)> {
    let mut found: Vec<(u64, String)> = milestones
        .iter()
        .filter(|m| m.is_open() && m.has(labels::TRIGGERED) && !m.has(labels::WAITING_MERGE))
        .map(|m| (m.number, branching::milestone_branch(m.number, &m.title)))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

/// What lanes work on right now, which a catch-up leaves alone: issues and
/// PRs by number, and among them the tasks whose session may still push to
/// their PR's branch.
#[derive(Debug, Clone, Default)]
pub struct Busy {
    /// The issues and PRs the lanes run, by number.
    pub numbers: Vec<u64>,
    /// The running tasks, as their milestone's board read them.
    pub tasks: Vec<Issue>,
}

impl Busy {
    /// The lanes' `running` numbers, the tasks among them found on `boards`.
    #[must_use]
    pub fn of(running: &[u64], boards: &[Board]) -> Self {
        Self {
            numbers: running.to_vec(),
            tasks: boards
                .iter()
                .flat_map(|board| &board.tasks)
                .filter(|task| running.contains(&task.number))
                .cloned()
                .collect(),
        }
    }

    /// Whether a lane works on `pr`: its own number, or a running task's —
    /// the branch its body declares, or its number in the PR's title.
    #[must_use]
    pub fn works_on(&self, pr: &Pr) -> bool {
        pr.num
            .parse::<u64>()
            .is_ok_and(|number| self.numbers.contains(&number))
            || self.tasks.iter().any(|task| tasks::pr_names_task(pr, task))
    }
}

/// The milestone a catch-up brings up, and to what.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    /// The milestone's number.
    pub milestone: u64,
    /// Its branch.
    pub branch: &'a str,
    /// The integration branch.
    pub integration: &'a str,
}

/// Brings one milestone's branch up to the integration branch's head, and
/// remembers it in `catch_ups`.
///
/// The head is the commit read this tick — merged as it is, never the
/// branch's tip, which may have moved since. GitHub's merge first; on a
/// refusal, `local` — the merge of that commit in the harness's checkout —
/// when this tick still allows one; otherwise [`CaughtUp::Deferred`]. A
/// branch already met with a residual conflict at this head is tried on
/// GitHub only, and said again while it has no landing PR. Held: the open
/// task PRs into the branch take it ([`refresh_task_prs`]), but those a
/// lane works on (`busy`). Residual: [`route_residual`]. Failed: parked,
/// said once. A dry run writes nothing and takes the branch as held, so its
/// tasks are not held.
#[allow(clippy::too_many_arguments)] // the catch-up's whole context, handed as it is
pub async fn catch_up(
    gh: &dyn GitHub,
    target: Target<'_>,
    catch_ups: &mut CatchUps,
    busy: &Busy,
    dry_run: bool,
    now: u64,
    local: impl AsyncFnOnce(String) -> CaughtUp,
    log: &Logbook,
) -> CaughtUp {
    let Target {
        milestone,
        branch,
        integration,
    } = target;
    if dry_run {
        log.say(&format!(
            "dry run: {branch} would take {integration} — nothing merged"
        ));
        catch_ups.record(milestone, &CaughtUp::OnGitHub, now);
        return CaughtUp::OnGitHub;
    }
    let Some(head) = catch_ups.head.clone() else {
        return CaughtUp::Deferred;
    };
    let landing = landing(gh, target).await;
    if let Some(pr) = &landing
        && busy.works_on(pr)
    {
        log.debug(&format!(
            "{branch}: a repair works on its landing PR {} — it takes {integration} after",
            pr.reference()
        ));
        catch_ups.outstanding.remove(&milestone);
        return CaughtUp::Deferred;
    }
    let caught = match gh.merge_branch(branch, &head).await {
        Ok(()) => {
            log.say(&format!("{branch} holds {integration}"));
            CaughtUp::OnGitHub
        }
        Err(refused) if catch_ups.residual_here(milestone) => {
            catch_ups.retry_later(milestone, now);
            remind(target, landing.as_ref(), &refused, log);
            return CaughtUp::Deferred;
        }
        Err(refused) if catch_ups.local_left == 0 => {
            log.say(&format!(
                "{branch} cannot take {integration} on GitHub ({}) — its local catch-up waits \
                 for the next tick",
                refused.reason()
            ));
            return CaughtUp::Deferred;
        }
        Err(refused) => {
            catch_ups.local_left -= 1;
            log.debug(&format!(
                "{branch}: GitHub refused to merge {integration} ({}) — merged in the \
                 harness's checkout",
                refused.reason()
            ));
            local(head).await
        }
    };
    catch_ups.record(milestone, &caught, now);
    match &caught {
        CaughtUp::OnGitHub | CaughtUp::Locally(_) => {
            refresh_task_prs(gh, branch, busy, log).await;
        }
        CaughtUp::Residual(files) => route_residual(gh, target, files, log).await,
        CaughtUp::Failed(why) => log.warn(&format!(
            "{branch} cannot take {integration}: {why} — tried again in an hour, its tasks wait"
        )),
        CaughtUp::Deferred => {}
    }
    caught
}

/// A residual conflict GitHub's merge still refuses at the same head:
/// quiet while the landing PR's repair has it; without one, said again —
/// the branch waits for a hand, and its tasks with it.
fn remind(target: Target<'_>, landing: Option<&Pr>, refused: &Halt, log: &Logbook) {
    let Target {
        branch,
        integration,
        ..
    } = target;
    match landing {
        Some(pr) => log.debug(&format!(
            "{branch} still cannot take {integration} on GitHub ({}) — its landing PR {} has \
             the repair",
            refused.reason(),
            pr.reference()
        )),
        None => log.warn(&format!(
            "{branch} still cannot take {integration} ({}): a conflict no script solves, and no \
             landing PR to repair — merge {integration} into it by hand; its tasks wait, tried \
             again in an hour",
            refused.reason()
        )),
    }
}

/// Routes a conflict no script solves, met on `target`'s branch once per
/// head.
///
/// Its open landing PR (head the branch, base the integration branch) is
/// labelled `harness:pr-fix` — the repair route merges the base in, within
/// its cap; without one, it is said and left until the base moves again.
/// Never `harness:human`: the next landing may bring the resolution.
pub async fn route_residual(gh: &dyn GitHub, target: Target<'_>, files: &[String], log: &Logbook) {
    let Target {
        milestone,
        branch,
        integration,
    } = target;
    let what = format!(
        "{branch} cannot take {integration}: a conflict no script solves, on {}",
        files.join(", ")
    );
    let Some(pr) = landing(gh, target).await else {
        log.say(&format!(
            "{what} — milestone #{milestone} has no landing PR open: left as it is, its tasks \
             wait, until {integration} moves again"
        ));
        return;
    };
    let asked = match pr.num.parse::<u64>() {
        Ok(_) if pr.has(labels::PR_FIX) => Ok(()),
        Ok(number) => gh.add_label(number, labels::PR_FIX).await,
        Err(_) => Err(harness_core::domain::Halt::Failed(format!(
            "{:?} is not a PR number",
            pr.num
        ))),
    };
    match asked {
        Ok(()) => log.say(&format!(
            "{what} — its landing PR {} is asked for a repair ({})",
            pr.reference(),
            labels::PR_FIX
        )),
        Err(halt) => log.warn(&format!(
            "{what} — its landing PR {} could not be asked for a repair: {}",
            pr.reference(),
            halt.reason()
        )),
    }
}

/// The milestone's open landing PR — head its branch, base the integration
/// branch — if it has one.
async fn landing(gh: &dyn GitHub, target: Target<'_>) -> Option<Pr> {
    gh.pr(target.branch)
        .await
        .ok()
        .filter(|pr| pr.is_open() && pr.base == target.integration)
}

/// Merges `branch` into the head of every open task PR whose base it is.
///
/// GitHub's merge, never a force — so its checks run again against the new
/// base; a PR a lane works on (`busy`: its own lane, or its task's
/// session still running) is left alone, a refusal (a
/// conflict) is the repair route's. The task PRs are the harness's: those
/// carrying `harness:to-review` or `harness:pr-fix`. Says which took it;
/// returns them (`#n`).
pub async fn refresh_task_prs(
    gh: &dyn GitHub,
    branch: &str,
    busy: &Busy,
    log: &Logbook,
) -> Vec<String> {
    let mut prs: Vec<Pr> = Vec::new();
    for label in [labels::TO_REVIEW, labels::PR_FIX] {
        match gh.open_prs_labelled(label).await {
            Ok(found) => prs.extend(found),
            Err(halt) => log.warn(&format!(
                "the task PRs into {branch} cannot be read ({label}): {}",
                halt.reason()
            )),
        }
    }
    prs.sort_by_key(|pr| pr.num.parse::<u64>().unwrap_or(u64::MAX));
    prs.dedup_by(|later, earlier| later.num == earlier.num);
    let mut refreshed = Vec::new();
    for pr in prs.iter().filter(|pr| pr.is_open() && pr.base == branch) {
        if busy.works_on(pr) {
            log.debug(&format!(
                "{}: a lane works on it — it takes {branch} later",
                pr.reference()
            ));
            continue;
        }
        match gh.merge_branch(&pr.head, branch).await {
            Ok(()) => refreshed.push(pr.reference()),
            Err(halt) => log.debug(&format!(
                "{} cannot take {branch}: {} — the repair route meets it",
                pr.reference(),
                halt.reason()
            )),
        }
    }
    if !refreshed.is_empty() {
        log.say(&format!(
            "{branch}: task PR(s) {} hold it now — a push where it moved runs their checks again",
            refreshed.join(", ")
        ));
    }
    refreshed
}

/// The line the watch reports a catch-up of `milestone` under
/// (`watch: catch_up -> …`, what the view's journal closes its row with),
/// and whether it is a warning: a branch that could not take the base.
#[must_use]
pub fn told(milestone: u64, caught: &CaughtUp) -> (String, bool) {
    match caught {
        CaughtUp::OnGitHub => (format!("milestone #{milestone} holds the base"), false),
        CaughtUp::Locally(regenerated) if regenerated.is_empty() => (
            format!("milestone #{milestone} holds the base, merged in the harness's checkout"),
            false,
        ),
        CaughtUp::Locally(regenerated) => (
            format!(
                "milestone #{milestone} holds the base: {} regenerated",
                regenerated.join(", ")
            ),
            false,
        ),
        CaughtUp::Residual(files) => (
            format!(
                "milestone #{milestone} cannot take the base: a conflict no script solves, on {}",
                files.join(", ")
            ),
            true,
        ),
        CaughtUp::Failed(why) => (
            format!("milestone #{milestone} cannot take the base: {why}"),
            true,
        ),
        CaughtUp::Deferred => (
            format!("milestone #{milestone} takes the base at a later tick"),
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::traces::{Sink, Verbosity};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
        fn keep(&self, _line: &str) {}
    }

    fn capture() -> (Rc<Capture>, Logbook) {
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        (capture, log)
    }

    fn milestone(number: u64, title: &str, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: title.to_string(),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn building() -> Vec<Issue> {
        vec![
            milestone(7, "Sound", &[labels::MILESTONE, labels::TRIGGERED]),
            milestone(6, "Media", &[labels::MILESTONE, labels::TRIGGERED]),
            milestone(
                5,
                "Sessions",
                &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
            ),
            milestone(8, "Later", &[labels::MILESTONE, labels::READY]),
            Issue {
                state: "closed".to_string(),
                ..milestone(4, "Done", &[labels::MILESTONE, labels::TRIGGERED])
            },
        ]
    }

    /// At `head`, its checks green.
    fn at(head: &str) -> CatchUps {
        CatchUps {
            head: Some(head.to_string()),
            verdict: Some(true),
            local_left: LOCAL_PER_TICK,
            ..CatchUps::default()
        }
    }

    /// Busy on these numbers, no task among them.
    fn busy(numbers: &[u64]) -> Busy {
        Busy {
            numbers: numbers.to_vec(),
            tasks: Vec::new(),
        }
    }

    fn pr(num: &str, head: &str, base: &str, labels: &[&str]) -> Pr {
        Pr {
            num: num.to_string(),
            head: head.to_string(),
            base: base.to_string(),
            state: "OPEN".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Pr::default()
        }
    }

    const MEDIA: Target<'static> = Target {
        milestone: 6,
        branch: "milestone/6-media",
        integration: "main_agent",
    };

    #[test]
    fn the_targets_are_the_split_milestones_not_landed_in_number_order() {
        assert_eq!(
            targets(&building()),
            [
                (6, "milestone/6-media".to_string()),
                (7, "milestone/7-sound".to_string())
            ]
        );
    }

    #[test]
    fn a_head_moves_on_its_first_read_and_on_every_change_only() {
        let mut catch_ups = CatchUps::default();
        assert!(!catch_ups.moved(None), "no head read: nothing moved");
        assert!(catch_ups.moved(Some("a".to_string())));
        assert!(!catch_ups.moved(Some("a".to_string())));
        assert!(!catch_ups.moved(None));
        assert_eq!(catch_ups.head.as_deref(), Some("a"), "a 404 keeps the last");
        catch_ups.verdict = Some(true);
        assert!(!catch_ups.moved(Some("b".to_string())) || catch_ups.verdict.is_none());
        assert_eq!(catch_ups.head.as_deref(), Some("b"));
        assert_eq!(catch_ups.verdict, None, "a new head's verdict is not in");
        catch_ups.verdict = Some(true);
        assert!(!catch_ups.moved(Some("b".to_string())));
        assert_eq!(catch_ups.verdict, Some(true), "the same head keeps it");
    }

    #[test]
    fn a_green_head_makes_every_branch_behind_it_due_and_none_twice() {
        let mut catch_ups = at("h1");
        let all = building();
        assert_eq!(catch_ups.due(&all, 0).len(), 2);
        for verdict in [None, Some(false)] {
            let pending = CatchUps {
                verdict,
                ..at("h1")
            };
            assert!(pending.due(&all, 0).is_empty(), "never before green");
        }
        catch_ups.record(6, &CaughtUp::OnGitHub, 0);
        assert_eq!(
            catch_ups.due(&all, 0),
            [(7, "milestone/7-sound".to_string())]
        );
        catch_ups.record(7, &CaughtUp::Locally(Vec::new()), 0);
        assert!(catch_ups.due(&all, 0).is_empty(), "caught up to h1");
        assert!(catch_ups.moved(Some("h2".to_string())));
        assert!(
            catch_ups.due(&all, 0).is_empty(),
            "h1's green never speaks for h2"
        );
        catch_ups.verdict = Some(true);
        assert_eq!(catch_ups.due(&all, 0).len(), 2, "the head moved");
        let headless = CatchUps {
            verdict: Some(true),
            ..CatchUps::default()
        };
        assert!(headless.due(&all, 0).is_empty(), "no head");
    }

    #[test]
    fn a_failure_is_parked_an_hour_and_holds_its_tasks_meanwhile() {
        let mut catch_ups = at("h1");
        let all = building();
        catch_ups.record(6, &CaughtUp::Failed("npx offline".to_string()), 100);
        assert_eq!(catch_ups.parked.get(&6), Some(&(100 + PARK_SECONDS)));
        assert_eq!(catch_ups.due(&all, 101).len(), 1, "#6 parked");
        assert_eq!(catch_ups.holding(101), [6]);
        let expiry = 100 + PARK_SECONDS;
        assert_eq!(catch_ups.holding(expiry - 1), [6]);
        assert!(catch_ups.holding(expiry).is_empty(), "the park is over");
        assert_eq!(catch_ups.due(&all, expiry).len(), 2, "tried again");
        catch_ups.record(6, &CaughtUp::OnGitHub, expiry);
        assert!(
            catch_ups.parked.is_empty() && catch_ups.holding(expiry).is_empty(),
            "nothing expected"
        );
    }

    #[test]
    fn a_residual_conflict_holds_its_tasks_until_the_branch_takes_the_base() {
        let mut catch_ups = at("h1");
        catch_ups.record(6, &CaughtUp::Failed("x".to_string()), 0);
        catch_ups.record(6, &CaughtUp::Residual(vec!["a.rs".to_string()]), 0);
        assert_eq!(
            catch_ups.residual.get(&6),
            Some(&Residual {
                head: "h1".to_string(),
                retry_at: PARK_SECONDS
            })
        );
        assert!(catch_ups.parked.is_empty(), "a residual is no failure");
        assert_eq!(catch_ups.holding(0), [6]);
        assert_eq!(
            catch_ups.due(&building(), PARK_SECONDS - 1),
            [(7, "milestone/7-sound".to_string())],
            "not due again at this head before its retry: its landing PR's repair is not held"
        );
        assert_eq!(
            catch_ups.due(&building(), PARK_SECONDS).len(),
            2,
            "its retry on GitHub is due"
        );
        catch_ups.retry_later(6, PARK_SECONDS);
        assert_eq!(
            catch_ups.residual.get(&6).map(|met| met.retry_at),
            Some(2 * PARK_SECONDS)
        );
        catch_ups.retry_later(9, 0);
        assert!(!catch_ups.residual.contains_key(&9), "nothing to retry");
        assert!(catch_ups.moved(Some("h2".to_string())));
        catch_ups.verdict = Some(true);
        assert_eq!(catch_ups.due(&building(), 0).len(), 2, "a new head");
        assert_eq!(catch_ups.holding(0), [6], "its tasks still wait");
        catch_ups.record(6, &CaughtUp::Deferred, 0);
        assert_eq!(catch_ups.holding(0), [6], "a deferral changes nothing");
        catch_ups.record(6, &CaughtUp::Locally(Vec::new()), 0);
        assert!(
            catch_ups.holding(0).is_empty() && catch_ups.residual.is_empty(),
            "nothing expected"
        );
        let mut unread = CatchUps::default();
        unread.record(6, &CaughtUp::OnGitHub, 0);
        assert!(unread.held.is_empty(), "no head read: nothing to remember");
    }

    #[test]
    fn a_full_read_allows_one_local_catch_up_and_remembers_what_is_due() {
        let mut catch_ups = at("h1");
        catch_ups.local_left = 0;
        catch_ups.record(7, &CaughtUp::Failed("x".to_string()), 0);
        let (due, held) = catch_ups.read(&building(), 1);
        assert_eq!(due, [(6, "milestone/6-media".to_string())]);
        assert_eq!(held, [7]);
        assert_eq!(catch_ups.local_left, LOCAL_PER_TICK);
        assert!(catch_ups.behind_seen);
        assert_eq!(catch_ups.outstanding, BTreeSet::from([6]));
        catch_ups.record(6, &CaughtUp::Deferred, 1);
        assert_eq!(catch_ups.outstanding, BTreeSet::from([6]), "still due");
        catch_ups.record(6, &CaughtUp::OnGitHub, 1);
        assert!(catch_ups.outstanding.is_empty(), "settled");
        catch_ups.record(7, &CaughtUp::OnGitHub, 1);
        let (due, held) = catch_ups.read(&building(), 1);
        assert!(due.is_empty() && held.is_empty(), "nothing expected");
        assert!(!catch_ups.behind_seen, "nothing behind");
        assert_eq!(LOCAL_PER_TICK, 1);
        assert_eq!(PARK_SECONDS, 3600);
    }

    /// A GitHub whose `main_agent` points at `head`, its checks answering
    /// `verdicts` by sha.
    fn base_at(head: &str, verdicts: &[(&str, Option<bool>)]) -> FakeGitHub {
        FakeGitHub {
            branch_shas: std::collections::HashMap::from([(
                "main_agent".to_string(),
                Some(head.to_string()),
            )]),
            commit_checks: verdicts
                .iter()
                .map(|(sha, verdict)| ((*sha).to_string(), *verdict))
                .collect(),
            ..FakeGitHub::default()
        }
    }

    #[tokio::test]
    async fn the_head_s_verdict_is_its_own_commit_s_never_an_older_one_s() {
        // h1 green, then a push: h2's runs not created yet.
        let mut catch_ups = CatchUps::default();
        let gh = base_at("h1", &[("h1", Some(true))]);
        assert!(
            read_head(&gh, &mut catch_ups, "main_agent")
                .await
                .expect("read")
        );
        assert_eq!(catch_ups.verdict, Some(true));
        catch_ups.read(&building(), 0);
        let gh = base_at("h2", &[("h1", Some(true))]);
        assert!(
            read_head(&gh, &mut catch_ups, "main_agent")
                .await
                .expect("read"),
            "the head moved: a full read"
        );
        assert_eq!(catch_ups.head.as_deref(), Some("h2"));
        assert_eq!(catch_ups.verdict, None, "h1's green is not h2's");
        assert!(
            catch_ups.read(&building(), 0).0.is_empty(),
            "nothing due on an unverified head"
        );
        // Still running: a quiet tick stays quiet.
        assert!(
            !read_head(&gh, &mut catch_ups, "main_agent")
                .await
                .expect("read")
        );
        // h2 turns green: that tick reads the board, and both are due.
        let gh = base_at("h2", &[("h2", Some(true))]);
        assert!(
            read_head(&gh, &mut catch_ups, "main_agent")
                .await
                .expect("read")
        );
        assert_eq!(catch_ups.read(&building(), 0).0.len(), 2);
        // The verdict in, no more checks read; due and unsettled: read again.
        let gh = base_at("h2", &[]);
        assert!(
            read_head(&gh, &mut catch_ups, "main_agent")
                .await
                .expect("read")
        );
        assert_eq!(catch_ups.verdict, Some(true));
        catch_ups.record(6, &CaughtUp::OnGitHub, 0);
        catch_ups.record(7, &CaughtUp::OnGitHub, 0);
        assert!(
            !read_head(&gh, &mut catch_ups, "main_agent")
                .await
                .expect("read")
        );
    }

    #[tokio::test]
    async fn a_red_head_is_read_again_until_a_rerun_turns_it_green() {
        let mut catch_ups = CatchUps::default();
        let red = base_at("h1", &[("h1", Some(false))]);
        assert!(
            read_head(&red, &mut catch_ups, "main_agent")
                .await
                .expect("read")
        );
        assert_eq!(catch_ups.verdict, Some(false));
        catch_ups.read(&building(), 0);
        assert!(
            !read_head(&red, &mut catch_ups, "main_agent")
                .await
                .expect("read"),
            "still red: nothing new"
        );
        let rerun = base_at("h1", &[("h1", None)]);
        assert!(
            !read_head(&rerun, &mut catch_ups, "main_agent")
                .await
                .expect("read")
        );
        assert_eq!(catch_ups.verdict, None, "its job runs again");
        let green = base_at("h1", &[("h1", Some(true))]);
        assert!(
            read_head(&green, &mut catch_ups, "main_agent")
                .await
                .expect("read")
        );
        assert_eq!(catch_ups.read(&building(), 0).0.len(), 2);
    }

    #[tokio::test]
    async fn the_checks_are_read_only_while_a_branch_is_behind_an_unjudged_head() {
        let refusing = FakeGitHub {
            branch_checks_refused: true,
            ..base_at("h1", &[])
        };
        // Nothing behind at the last read, the head unmoved: one call only.
        let mut catch_ups = CatchUps {
            head: Some("h1".to_string()),
            ..CatchUps::default()
        };
        assert!(
            !read_head(&refusing, &mut catch_ups, "main_agent")
                .await
                .expect("read")
        );
        // Behind: the checks are read, and their failure stops the tick.
        catch_ups.behind_seen = true;
        assert!(
            read_head(&refusing, &mut catch_ups, "main_agent")
                .await
                .is_err()
        );
        // Judged green: read no more.
        catch_ups.verdict = Some(true);
        assert!(
            !read_head(&refusing, &mut catch_ups, "main_agent")
                .await
                .expect("read")
        );
        // An absent branch keeps the head, and reads nothing more.
        let absent = FakeGitHub {
            branch_shas: std::collections::HashMap::from([("main_agent".to_string(), None)]),
            ..FakeGitHub::default()
        };
        let mut unread = CatchUps::default();
        assert!(
            !read_head(&absent, &mut unread, "main_agent")
                .await
                .expect("read")
        );
        assert!(
            read_head(&FakeGitHub::default(), &mut unread, "main_agent")
                .await
                .is_err()
        );
    }

    /// Runs one catch-up of `#6 Media`, the local merge answering `local`.
    async fn run(
        gh: &FakeGitHub,
        catch_ups: &mut CatchUps,
        busy: &Busy,
        local: CaughtUp,
    ) -> (CaughtUp, bool, Vec<String>) {
        let (said, log) = capture();
        let went_local = RefCell::new(false);
        let flag = &went_local;
        let catch_ups_head = catch_ups.head.clone();
        let caught = catch_up(
            gh,
            MEDIA,
            catch_ups,
            busy,
            false,
            10,
            async move |head: String| {
                assert_eq!(Some(head), catch_ups_head, "the commit read is merged");
                *flag.borrow_mut() = true;
                local
            },
            &log,
        )
        .await;
        let lines = said.0.borrow().clone();
        (caught, went_local.into_inner(), lines)
    }

    #[tokio::test]
    async fn github_s_merge_comes_first_and_the_task_prs_take_the_branch() {
        let gh = FakeGitHub {
            prs_labelled: std::collections::HashMap::from([
                (
                    labels::TO_REVIEW.to_string(),
                    vec![
                        pr("31", "task/31", "milestone/6-media", &[labels::TO_REVIEW]),
                        pr("32", "task/32", "milestone/6-media", &[labels::TO_REVIEW]),
                        pr("40", "task/40", "milestone/7-sound", &[labels::TO_REVIEW]),
                    ],
                ),
                (
                    labels::PR_FIX.to_string(),
                    vec![
                        pr("31", "task/31", "milestone/6-media", &[labels::PR_FIX]),
                        Pr {
                            state: "MERGED".to_string(),
                            ..pr("33", "task/33", "milestone/6-media", &[labels::PR_FIX])
                        },
                    ],
                ),
            ]),
            ..FakeGitHub::default()
        };
        let mut catch_ups = at("h1");
        let (caught, went_local, said) =
            run(&gh, &mut catch_ups, &busy(&[32]), CaughtUp::Deferred).await;
        assert_eq!(caught, CaughtUp::OnGitHub);
        assert!(!went_local, "GitHub took it: no checkout");
        assert_eq!(catch_ups.held.get(&6).map(String::as_str), Some("h1"));
        assert_eq!(catch_ups.local_left, LOCAL_PER_TICK, "nothing local spent");
        assert_eq!(
            gh.writes(),
            [
                Wrote::MergedBranch("milestone/6-media".to_string(), "h1".to_string()),
                Wrote::MergedBranch("task/31".to_string(), "milestone/6-media".to_string()),
            ],
            "#32 is on a lane, #33 merged, #40 into another milestone"
        );
        assert_eq!(
            said,
            [
                "milestone/6-media holds main_agent",
                "milestone/6-media: task PR(s) #31 hold it now — a push where it moved runs their \
                 checks again"
            ]
        );
    }

    #[tokio::test]
    async fn a_refusal_goes_local_once_a_tick_then_waits() {
        let gh = FakeGitHub {
            refused_merges: vec!["milestone/6-media".to_string()],
            ..FakeGitHub::default()
        };
        let mut catch_ups = at("h1");
        let regenerated = CaughtUp::Locally(vec!["Cargo.lock".to_string()]);
        let (caught, went_local, _) =
            run(&gh, &mut catch_ups, &Busy::default(), regenerated.clone()).await;
        assert_eq!(caught, regenerated);
        assert!(went_local);
        assert_eq!(catch_ups.local_left, 0);
        assert_eq!(catch_ups.held.get(&6).map(String::as_str), Some("h1"));
        catch_ups.held.clear();
        let (caught, went_local, said) =
            run(&gh, &mut catch_ups, &Busy::default(), regenerated).await;
        assert_eq!(caught, CaughtUp::Deferred);
        assert!(!went_local, "this tick's local catch-up is spent");
        assert!(
            catch_ups.held.is_empty(),
            "nothing remembered: due next tick"
        );
        assert_eq!(said.len(), 1);
        assert!(
            said[0].starts_with("milestone/6-media cannot take main_agent on GitHub (")
                && said[0].ends_with("its local catch-up waits for the next tick"),
            "{said:?}"
        );
    }

    #[tokio::test]
    async fn a_failure_is_said_once_and_parked() {
        let gh = FakeGitHub {
            refused_merges: vec!["milestone/6-media".to_string()],
            ..FakeGitHub::default()
        };
        let mut catch_ups = at("h1");
        let (caught, _, said) = run(
            &gh,
            &mut catch_ups,
            &Busy::default(),
            CaughtUp::Failed("sh mock/build.sh failed: npx".to_string()),
        )
        .await;
        assert!(matches!(caught, CaughtUp::Failed(_)));
        assert_eq!(catch_ups.parked.get(&6), Some(&(10 + PARK_SECONDS)));
        assert_eq!(
            said,
            [
                "warning: milestone/6-media cannot take main_agent: sh mock/build.sh failed: npx — \
              tried again in an hour, its tasks wait"
            ]
        );
        assert!(gh.writes().is_empty(), "no task PR refreshed");
    }

    #[tokio::test]
    async fn a_residual_conflict_asks_the_landing_pr_for_a_repair_once_per_head() {
        let gh = FakeGitHub {
            refused_merges: vec!["milestone/6-media".to_string()],
            prs: vec![(
                "milestone/6-media".to_string(),
                pr("90", "milestone/6-media", "main_agent", &[]),
            )],
            ..FakeGitHub::default()
        };
        let mut catch_ups = at("h1");
        let residual = CaughtUp::Residual(vec!["crates/server/src/lib.rs".to_string()]);
        let (caught, _, said) = run(&gh, &mut catch_ups, &Busy::default(), residual.clone()).await;
        assert_eq!(caught, residual);
        assert_eq!(gh.writes(), [Wrote::Label(90, labels::PR_FIX.to_string())]);
        assert_eq!(
            said,
            [
                "milestone/6-media cannot take main_agent: a conflict no script solves, on \
              crates/server/src/lib.rs — its landing PR #90 is asked for a repair \
              (harness:pr-fix)"
            ]
        );
        assert_eq!(
            catch_ups.residual.get(&6).map(|met| met.retry_at),
            Some(10 + PARK_SECONDS)
        );
        // Its retry at the same head: GitHub only, quietly while the landing
        // PR has the repair, no local merge, nothing labelled again.
        catch_ups.local_left = LOCAL_PER_TICK;
        catch_ups.outstanding.insert(6);
        let (caught, went_local, said) =
            run(&gh, &mut catch_ups, &Busy::default(), residual.clone()).await;
        assert_eq!(caught, CaughtUp::Deferred);
        assert!(!went_local && said.is_empty(), "{said:?}");
        assert_eq!(gh.writes().len(), 1);
        assert_eq!(catch_ups.local_left, LOCAL_PER_TICK, "nothing spent");
        assert!(catch_ups.outstanding.is_empty(), "settled until its retry");
        assert_eq!(
            catch_ups.residual.get(&6).map(|met| met.retry_at),
            Some(10 + PARK_SECONDS),
            "tried again an hour after this try"
        );
        // The head moved: tried locally again.
        catch_ups.moved(Some("h2".to_string()));
        let (_, went_local, _) = run(&gh, &mut catch_ups, &Busy::default(), residual).await;
        assert!(went_local);
    }

    #[tokio::test]
    async fn a_residual_resolved_by_hand_releases_its_tasks_without_the_head_moving() {
        let refusing = FakeGitHub {
            refused_merges: vec!["milestone/6-media".to_string()],
            ..FakeGitHub::default()
        };
        let mut catch_ups = at("h1");
        let residual = CaughtUp::Residual(vec!["mock/src/commands.js".to_string()]);
        run(
            &refusing,
            &mut catch_ups,
            &Busy::default(),
            residual.clone(),
        )
        .await;
        assert_eq!(catch_ups.holding(10), [6]);
        // An hour later, still refused, no landing PR: said again.
        let (caught, went_local, said) =
            run(&refusing, &mut catch_ups, &Busy::default(), residual).await;
        assert_eq!(caught, CaughtUp::Deferred);
        assert!(!went_local);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with("warning: milestone/6-media still cannot take main_agent (")
                && said[0].contains("merge main_agent into it by hand"),
            "{said:?}"
        );
        assert!(refusing.writes().is_empty(), "no label, no harness:human");
        // A hand merged the base: GitHub's merge holds, at the same head.
        let (caught, _, _) = run(
            &FakeGitHub::default(),
            &mut catch_ups,
            &Busy::default(),
            CaughtUp::Failed("never local".to_string()),
        )
        .await;
        assert_eq!(caught, CaughtUp::OnGitHub);
        assert_eq!(
            catch_ups.head.as_deref(),
            Some("h1"),
            "the head never moved"
        );
        assert!(catch_ups.holding(10).is_empty(), "its tasks are released");
        assert!(catch_ups.residual.is_empty(), "nothing expected");
    }

    #[tokio::test]
    async fn a_residual_conflict_without_a_landing_pr_is_said_and_left() {
        let (said, log) = capture();
        let gh = FakeGitHub {
            prs: vec![(
                "milestone/6-media".to_string(),
                Pr {
                    state: "CLOSED".to_string(),
                    ..pr("90", "milestone/6-media", "main_agent", &[])
                },
            )],
            ..FakeGitHub::default()
        };
        route_residual(&gh, MEDIA, &["a.rs".to_string(), "b.js".to_string()], &log).await;
        assert!(gh.writes().is_empty(), "no harness:human, no label");
        assert_eq!(
            said.0.borrow().as_slice(),
            [
                "milestone/6-media cannot take main_agent: a conflict no script solves, on a.rs, \
              b.js — milestone #6 has no landing PR open: left as it is, its tasks wait, until \
              main_agent moves again"
            ]
        );
        // A PR into another base is no landing PR either; none at all neither.
        let gh = FakeGitHub {
            prs: vec![(
                "milestone/6-media".to_string(),
                pr("91", "milestone/6-media", "main", &[]),
            )],
            ..FakeGitHub::default()
        };
        route_residual(&gh, MEDIA, &["a.rs".to_string()], &log).await;
        route_residual(&FakeGitHub::default(), MEDIA, &["a.rs".to_string()], &log).await;
        assert!(gh.writes().is_empty(), "nothing expected");
        assert_eq!(said.0.borrow().len(), 3);
    }

    #[tokio::test]
    async fn a_landing_pr_already_asked_or_refusing_the_label_is_said() {
        let (said, log) = capture();
        let gh = FakeGitHub {
            prs: vec![(
                "milestone/6-media".to_string(),
                pr("90", "milestone/6-media", "main_agent", &[labels::PR_FIX]),
            )],
            ..FakeGitHub::default()
        };
        route_residual(&gh, MEDIA, &["a.rs".to_string()], &log).await;
        assert!(gh.writes().is_empty(), "already asked");
        assert!(said.0.borrow()[0].ends_with("is asked for a repair (harness:pr-fix)"));
        let gh = FakeGitHub {
            prs: vec![(
                "milestone/6-media".to_string(),
                pr("90", "milestone/6-media", "main_agent", &[]),
            )],
            ..FakeGitHub::default()
        };
        gh.refuse_label(labels::PR_FIX);
        route_residual(&gh, MEDIA, &["a.rs".to_string()], &log).await;
        let gh = FakeGitHub {
            prs: vec![(
                "milestone/6-media".to_string(),
                pr("x", "milestone/6-media", "main_agent", &[]),
            )],
            ..FakeGitHub::default()
        };
        route_residual(&gh, MEDIA, &["a.rs".to_string()], &log).await;
        let said = said.0.borrow();
        assert!(
            said[1].starts_with("warning: ") && said[1].contains("PR #90 could not be asked"),
            "{said:?}"
        );
        assert!(said[2].contains("PR #x could not be asked"), "{said:?}");
    }

    #[tokio::test]
    async fn a_dry_run_merges_nothing_and_holds_no_task() {
        let gh = FakeGitHub::default();
        let mut catch_ups = at("h1");
        let (said, log) = capture();
        let caught = catch_up(
            &gh,
            MEDIA,
            &mut catch_ups,
            &Busy::default(),
            true,
            0,
            async |_| CaughtUp::Failed("never".to_string()),
            &log,
        )
        .await;
        assert_eq!(caught, CaughtUp::OnGitHub);
        assert!(gh.writes().is_empty(), "nothing expected");
        assert_eq!(catch_ups.held.get(&6).map(String::as_str), Some("h1"));
        assert_eq!(
            said.0.borrow().as_slice(),
            ["dry run: milestone/6-media would take main_agent — nothing merged"]
        );
    }

    #[tokio::test]
    async fn a_task_pr_that_refuses_is_left_to_the_repair_and_unreadable_lists_are_said() {
        let gh = FakeGitHub {
            refused_merges: vec!["task/31".to_string()],
            prs_labelled: std::collections::HashMap::from([(
                labels::TO_REVIEW.to_string(),
                vec![
                    pr("31", "task/31", "milestone/6-media", &[]),
                    pr("30", "task/30", "milestone/6-media", &[]),
                ],
            )]),
            ..FakeGitHub::default()
        };
        let (said, log) = capture();
        assert_eq!(
            refresh_task_prs(&gh, "milestone/6-media", &Busy::default(), &log).await,
            ["#30"]
        );
        assert_eq!(said.0.borrow().len(), 1);
        let broken = FakeGitHub {
            broken: Some(harness_core::domain::Halt::Failed("HTTP 502".to_string())),
            ..FakeGitHub::default()
        };
        let (said, log) = capture();
        assert!(
            refresh_task_prs(&broken, "milestone/6-media", &Busy::default(), &log)
                .await
                .is_empty(),
            "nothing read, nothing refreshed"
        );
        let said = said.0.borrow();
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].starts_with("warning: the task PRs into milestone/6-media cannot be read"));
    }

    #[tokio::test]
    async fn a_catch_up_waits_while_a_lane_repairs_its_landing_pr() {
        let gh = FakeGitHub {
            prs: vec![(
                "milestone/6-media".to_string(),
                pr("90", "milestone/6-media", "main_agent", &[]),
            )],
            ..FakeGitHub::default()
        };
        let mut catch_ups = at("h1");
        catch_ups.outstanding.insert(6);
        let (caught, went_local, said) =
            run(&gh, &mut catch_ups, &busy(&[90]), CaughtUp::Deferred).await;
        assert_eq!(caught, CaughtUp::Deferred);
        assert!(!went_local && said.is_empty(), "{said:?}");
        assert!(gh.writes().is_empty(), "nothing merged under the repair");
        assert!(catch_ups.held.is_empty(), "due again at a later read");
        assert!(
            catch_ups.outstanding.is_empty(),
            "the lane's end reads the board, not every tick"
        );
        // Another PR on a lane holds nothing.
        let (caught, _, _) = run(&gh, &mut catch_ups, &busy(&[91]), CaughtUp::Deferred).await;
        assert_eq!(caught, CaughtUp::OnGitHub);
    }

    /// A task of `milestone`'s board, declaring `branch`.
    fn task(number: u64, branch: &str) -> Issue {
        Issue {
            number,
            body: format!("## Scope\nbranch: {branch}\n"),
            ..milestone(number, "task", &[labels::AGENT])
        }
    }

    #[test]
    fn a_running_task_s_pr_is_busy_by_its_declared_branch_or_its_title() {
        let boards = vec![Board {
            milestone: milestone(6, "Media", &[labels::MILESTONE]),
            tasks: vec![task(31, "feat/31-upload"), task(32, "feat/32-gallery")],
            resuming: None,
        }];
        let busy = Busy::of(&[31, 90], &boards);
        assert_eq!(busy.numbers, [31, 90]);
        assert_eq!(
            busy.tasks.iter().map(|t| t.number).collect::<Vec<_>>(),
            [31],
            "#32 runs on no lane"
        );
        assert!(busy.works_on(&pr("77", "feat/31-upload", "milestone/6-media", &[])));
        assert!(busy.works_on(&Pr {
            title: "feat: upload (#31)".to_string(),
            ..pr("78", "elsewhere", "milestone/6-media", &[])
        }));
        assert!(busy.works_on(&pr("90", "milestone/6-media", "main_agent", &[])));
        assert!(!busy.works_on(&pr("79", "feat/32-gallery", "milestone/6-media", &[])));
        assert!(!Busy::default().works_on(&pr("31", "feat/31-upload", "x", &[])));
    }

    #[tokio::test]
    async fn a_task_pr_whose_session_still_runs_is_not_refreshed_under_it() {
        let gh = FakeGitHub {
            prs_labelled: std::collections::HashMap::from([(
                labels::TO_REVIEW.to_string(),
                vec![
                    pr("77", "feat/31-upload", "milestone/6-media", &[]),
                    pr("79", "feat/32-gallery", "milestone/6-media", &[]),
                ],
            )]),
            ..FakeGitHub::default()
        };
        let boards = vec![Board {
            milestone: milestone(6, "Media", &[labels::MILESTONE]),
            tasks: vec![task(31, "feat/31-upload")],
            resuming: None,
        }];
        let (_, log) = capture();
        assert_eq!(
            refresh_task_prs(&gh, "milestone/6-media", &Busy::of(&[31], &boards), &log).await,
            ["#79"]
        );
        assert_eq!(
            gh.writes(),
            [Wrote::MergedBranch(
                "feat/32-gallery".to_string(),
                "milestone/6-media".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn a_catch_up_without_a_head_read_waits() {
        let gh = FakeGitHub::default();
        let mut catch_ups = CatchUps::default();
        let (caught, went_local, _) =
            run(&gh, &mut catch_ups, &Busy::default(), CaughtUp::Deferred).await;
        assert_eq!(caught, CaughtUp::Deferred);
        assert!(!went_local && gh.writes().is_empty(), "nothing expected");
    }

    #[test]
    fn each_outcome_is_told_under_the_watch_s_tag_a_refusal_as_a_warning() {
        assert_eq!(
            told(6, &CaughtUp::OnGitHub),
            ("milestone #6 holds the base".to_string(), false)
        );
        assert_eq!(
            told(6, &CaughtUp::Locally(Vec::new())),
            (
                "milestone #6 holds the base, merged in the harness's checkout".to_string(),
                false
            )
        );
        assert_eq!(
            told(
                6,
                &CaughtUp::Locally(vec!["Cargo.lock".to_string(), "a.js".to_string()])
            ),
            (
                "milestone #6 holds the base: Cargo.lock, a.js regenerated".to_string(),
                false
            )
        );
        assert_eq!(
            told(
                6,
                &CaughtUp::Residual(vec!["a.rs".to_string(), "b.js".to_string()])
            ),
            (
                "milestone #6 cannot take the base: a conflict no script solves, on a.rs, b.js"
                    .to_string(),
                true
            )
        );
        assert_eq!(
            told(6, &CaughtUp::Failed("npx".to_string())),
            ("milestone #6 cannot take the base: npx".to_string(), true)
        );
        assert_eq!(
            told(6, &CaughtUp::Deferred),
            (
                "milestone #6 takes the base at a later tick".to_string(),
                false
            )
        );
    }
}
