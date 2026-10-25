//! What a repair writes: the label it was asked by consumed; after the
//! session, a contract it stopped on asked and the PR made to wait, and
//! what it committed pushed.
//!
//! **Removed before the session is paid for, not after it succeeds.** The
//! alternative was tried on paper and rejected: a PR whose CI stays red —
//! a broken dependency, an environment the agent cannot reach, a test that
//! needs a human decision — would then buy one opus session every poll,
//! forever, with nobody watching. One label is one attempt, and the human
//! re-poses it if the attempt was worth repeating.
//!
//! The cost of that choice, stated plainly: a session that dies on a
//! transient error has spent the request. That is recoverable by re-posing
//! a label; an unbounded spend is not.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Issue, Outcome, Pr, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::git::Repo;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Logbook;

use crate::common::contract_ask::{self, Asked, Requester};
use crate::common::{branching, contest, delivery, labels};
use crate::contract_manager::data::request as contract;
use crate::pr_fix::data::state::FixState;

/// Removes `harness:pr-fix` from the PR: the request has been taken.
pub struct ConsumeRequest {
    /// What removes the label.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<FixState> for ConsumeRequest {
    async fn run(&self, ctx: &mut Context<FixState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        // Nothing broke and the review asks for nothing, so nothing was asked
        // of anyone: leaving the label in place lets the poll that finds a
        // real failure use it.
        if !ctx.state.asks_for_a_repair() {
            return Ok(Verdict::Continue);
        }
        let pr = ctx.state.pr();
        let number = pr.num.parse::<u64>().map_err(|_| {
            Halt::Failed(format!(
                "{:?} is not a PR number — a label is posed by number, and \
                 this came from a PR listing that should only ever yield one",
                pr.num
            ))
        })?;
        self.gh.remove_label(number, labels::PR_FIX).await?;
        ctx.traces.say(&format!(
            "#{number}: {} consumed — one attempt follows",
            labels::PR_FIX
        ));
        Ok(Verdict::Continue)
    }
}

/// The issue a PR's contract stop is asked for, with the issue as read: a
/// landing PR's milestone (its head is the milestone's branch), else the
/// task its body closes (`delivery::closed_task`, the merge train's reading
/// of `Closes`/`Fixes`/`Resolves #N`).
async fn requester(gh: &dyn GitHub, pr: &Pr) -> Outcome<Option<(Requester, Issue)>> {
    if let Some(milestone) = branching::milestone_number(&pr.head) {
        let issue = gh.issue(milestone).await?;
        return Ok(Some((
            Requester {
                issue: milestone,
                milestone: Some(milestone),
                body: issue.body.clone(),
                task: false,
            },
            issue,
        )));
    }
    let Ok(number) = pr.num.parse::<u64>() else {
        return Ok(None);
    };
    let Some(task) = delivery::closed_task(&gh.issue(number).await?.body) else {
        return Ok(None);
    };
    let issue = gh.issue(task).await?;
    // A task PR targets its milestone's branch: that is its milestone.
    Ok(Some((
        Requester {
            issue: task,
            milestone: branching::milestone_number(&pr.base),
            body: issue.body.clone(),
            task: true,
        },
        issue,
    )))
}

/// A repair stopped on a contract another milestone produces: its change
/// asked (`common::contract_ask`), the PR waits on it.
///
/// - **Opened or found open**: the PR carries the contract wait
///   (`contest::contract_wait_marker`) — no repair, no escalation, no merge
///   until the change is closed and the PR's base holds the manifest; then
///   the harness resumes it, its repairs counted anew
///   (`contest::resume_contract`).
/// - **The PR's own to evolve** (an aggregate, its own milestone's
///   contract): the answer is posted on the PR — where the next repair
///   reads it — with `harness:pr-fix`, and **no decided marker**: the repair
///   cap holds, so a session that keeps stopping on it is not repaired
///   forever.
///
/// `harness:pr-fix` is posed in both cases: a landing PR carries no other
/// label the router lists, and a waiting PR is never repaired.
///
/// **Within the waits cap** ([`contest::MAX_WAITS`]): each landed change
/// resumes the PR and counts its repairs anew, so repair → stop → wait →
/// resume would have no bound of its own (`ForgeDesMondes`' #482: a change
/// landed that was not what the repair needed, the repair stopped on the
/// same id again). At the cap, no new wait: the PR's owner is escalated
/// (`milestone_merge::escalate`, a human's past the cap).
/// `ForgeDesMondes`' #423, #156, #482/#500: three contract changes a human
/// opened by hand after a repair said "push nothing".
pub struct RaiseContract {
    /// What reads the requester and writes the request and the PR's marker.
    pub gh: Rc<dyn GitHub>,
    /// The integration branch, where the plan's graph is read.
    pub integration: String,
}

#[async_trait(?Send)]
impl Action<FixState> for RaiseContract {
    async fn run(&self, ctx: &mut Context<FixState>) -> Outcome<Verdict> {
        let Some(reason) = ctx.state.contract_stop.clone() else {
            return Ok(Verdict::Continue);
        };
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some((id, need)) = contract::asked(&reason) else {
            return Ok(Verdict::Continue);
        };
        let pr = ctx.state.pr().clone();
        let number = pr_number(&pr)?;
        let Some((owner, owner_issue)) = requester(self.gh.as_ref(), &pr).await? else {
            // Nobody to ask it for: the stop stays the repair's halt.
            return Err(Halt::Halted(format!(
                "{reason} — {} closes no task, and is no milestone's landing PR: the contract \
                 change cannot be asked for anyone",
                pr.reference()
            )));
        };
        let waits = contest::waits_decided(&ctx.state.comments);
        if waits >= contest::MAX_WAITS
            && contract_ask::tasks_own(self.gh.as_ref(), owner.milestone, &id, &self.integration)
                .await
                .is_none()
        {
            self.gh
                .post_issue_comment(
                    number,
                    &format!(
                        "The repair stopped on `{id}` again ({need}), after {waits} wait(s) on \
                         this PR: no new wait. #{} decides what becomes of it.",
                        owner.issue
                    ),
                )
                .await?;
            let asked = crate::milestone_merge::escalate(
                self.gh.as_ref(),
                &owner_issue,
                &pr,
                &ctx.state.comments,
            )
            .await?;
            ctx.traces.warn(&format!(
                "{}: stopped on `{id}` past its {waits} wait(s) — no new wait, #{} escalated{}",
                pr.reference(),
                owner.issue,
                if asked { "" } else { " (asked already)" }
            ));
            return Ok(Verdict::Continue);
        }
        let asked = contract_ask::ask(
            self.gh.as_ref(),
            &owner,
            (&id, &need),
            &self.integration,
            &ctx.traces,
        )
        .await?;
        let said = match &asked {
            Asked::TasksOwn(why) => format!(
                "The repair stopped on `{id}` ({need}). It is this PR's own to evolve — {why}: \
                 evolve it here, versioned — the manifest or aggregate.json with its version \
                 raised, what reads or judges it, their tests. One repair follows."
            ),
            Asked::Opened(request) | Asked::Existing(request) => format!(
                "{}\nThe repair stopped on `{id}`, a contract another milestone produces \
                 ({need}). Contract change #{request} asks it; this PR waits — no repair, \
                 decision or merge — until #{request} is closed and `{}` holds the new manifest. \
                 Then the harness resumes it, its repairs counted anew.",
                contest::contract_wait_marker(*request),
                pr.base
            ),
        };
        self.gh.post_issue_comment(number, &said).await?;
        self.gh.add_label(number, labels::PR_FIX).await?;
        ctx.traces.say(&match asked {
            Asked::TasksOwn(_) => format!(
                "{}: stopped on `{id}`, its own to evolve — answered, one repair follows",
                pr.reference()
            ),
            Asked::Opened(request) => format!(
                "{}: stopped on `{id}` — contract change #{request} opened, the PR waits on it",
                pr.reference()
            ),
            Asked::Existing(request) => format!(
                "{}: stopped on `{id}` — contract change #{request} already asks it, the PR \
                 waits on it",
                pr.reference()
            ),
        });
        Ok(Verdict::Continue)
    }
}

/// After the session, pushes what it committed and did not push.
///
/// Only when
/// it answered `AGENT_LOOP_OK` saying it pushed, never what a session that
/// said "push nothing" or stopped left behind — and tells a repair that
/// pushed nothing at all.
///
/// **What is not pushed to the PR is set aside, never left in the
/// checkout**: pushed to `harness/unpushed/<pr>-<sha>` and the checkout reset
/// to `origin/<head>`. The lane's next mount refuses to reset a checkout
/// holding unpushed commits (`provisioning`), and `decide` halted on
/// `router-lane-0` over two commits a repair left (`ForgeDesMondes`' #496,
/// 12:25: no `AGENT_LOOP_OK` marker, one commit ahead; 18:05: the halt).
pub struct PushWhatWasCommitted {
    /// What reads the PR's head.
    pub gh: Rc<dyn GitHub>,
    /// The repair's checkout.
    pub git: Rc<dyn Repo>,
}

/// The branch the commits of a repair not pushed to its PR are set aside on.
fn aside_branch(pr: &str, sha: &str) -> String {
    let short = sha.get(..sha.len().min(12)).unwrap_or(sha);
    format!("harness/unpushed/{pr}-{short}")
}

impl PushWhatWasCommitted {
    /// Sets the commits `unpushed` aside on a branch of their own, then
    /// resets the checkout to the PR's head on `origin` — the shas journaled
    /// first, so a failed push loses nothing a human cannot find.
    async fn set_aside(&self, pr: &Pr, unpushed: &[String], log: &Logbook) -> Outcome<()> {
        let shas: Vec<&str> = unpushed
            .iter()
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        let head = self.git.head_sha().await?;
        let aside = aside_branch(&pr.num, &head);
        let ran = self.git.push_head(&aside).await?;
        if ran.ok() {
            log.warn(&format!(
                "{}: {} commit(s) not pushed to {} ({}) set aside on {aside}",
                pr.reference(),
                shas.len(),
                pr.head,
                shas.join(", ")
            ));
        } else {
            log.warn(&format!(
                "{}: {} commit(s) not pushed to {} ({}) could not be set aside on {aside}: {}",
                pr.reference(),
                shas.len(),
                pr.head,
                shas.join(", "),
                ran.stderr.lines().last().unwrap_or_default()
            ));
        }
        let reset = self.git.reset_hard(&format!("origin/{}", pr.head)).await?;
        if !reset.ok() {
            log.warn(&format!(
                "{}: the checkout could not be reset to origin/{}: {}",
                pr.reference(),
                pr.head,
                reset.stderr.lines().last().unwrap_or_default()
            ));
        }
        Ok(())
    }
}

#[async_trait(?Send)]
impl Action<FixState> for PushWhatWasCommitted {
    async fn run(&self, ctx: &mut Context<FixState>) -> Outcome<Verdict> {
        let stopped = ctx.state.contract_stop.is_some();
        if ctx.settings.dry_run || (ctx.state.answered_ok.is_none() && !stopped) {
            return Ok(Verdict::Continue);
        }
        let said_ok = ctx.state.answered_ok == Some(true) && !stopped;
        let pr = ctx.state.pr().clone();
        let unpushed = self.git.unpushed().await?;
        let mut pushed = false;
        if !unpushed.is_empty() {
            if said_ok {
                let ran = self.git.push_head(&pr.head).await?;
                if ran.ok() {
                    pushed = true;
                    ctx.traces.say(&format!(
                        "{}: pushed what the repair committed ({} commit(s)) to {}",
                        pr.reference(),
                        unpushed.len(),
                        pr.head
                    ));
                } else {
                    ctx.traces.warn(&format!(
                        "{}: what the repair committed could not be pushed to {}: {}",
                        pr.reference(),
                        pr.head,
                        ran.stderr.lines().last().unwrap_or_default()
                    ));
                }
            } else if !stopped {
                ctx.traces.warn(&format!(
                    "{}: the repair committed {} commit(s) and did not answer AGENT_LOOP_OK — \
                     not pushed",
                    pr.reference(),
                    unpushed.len()
                ));
            }
            if !pushed {
                self.set_aside(&pr, &unpushed, &ctx.traces).await?;
            }
        }
        if stopped {
            return Ok(Verdict::Continue);
        }
        let after = self.gh.branch_sha(&pr.head).await.ok().flatten();
        if !pushed && ctx.state.head_before.is_some() && after == ctx.state.head_before {
            ctx.traces.warn(&format!(
                "{}: repair pushed nothing — {} is where it was",
                pr.reference(),
                pr.head
            ));
        }
        Ok(Verdict::Continue)
    }
}

/// The PR's number, as a label or a comment on it needs one.
fn pr_number(pr: &Pr) -> Outcome<u64> {
    pr.num
        .parse()
        .map_err(|_| Halt::Failed(format!("PR {} has no number to write on", pr.reference())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Pr;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool, num: &str, failing: &[&str]) -> Context<FixState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            FixState {
                pr: Some(Pr {
                    num: num.to_string(),
                    ..Pr::default()
                }),
                failing: failing.iter().map(|f| (*f).to_string()).collect(),
                comments: String::new(),
                review_blocking: false,
                conflicting: false,
                ..FixState::default()
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn the_label_is_removed_once_something_is_actually_broken() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, "32", &["ci — FAILURE"]);
        ConsumeRequest { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("consumed");
        assert_eq!(
            gh.writes(),
            vec![Wrote::Unlabelled(32, labels::PR_FIX.to_string())]
        );
    }

    #[tokio::test]
    async fn a_pr_with_nothing_broken_keeps_its_label_for_a_later_poll() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, "32", &[]);
        ConsumeRequest { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("nothing to consume");
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(true, "32", &["ci — FAILURE"]);
        ConsumeRequest { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("nothing to do");
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn a_pr_reference_that_is_not_a_number_fails_rather_than_labelling_something_else() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, "https://github.com/o/r/pull/32", &["ci — FAILURE"]);
        let err = ConsumeRequest { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    // --- after the session: a contract stop asked, a commit pushed ---------

    const GRAPH: &str = r#"{"version":1,"milestones":[
      {"number":20,"title":"sessions","systems":["session"],"produces":["Session"],"consumes":[],"depends_on":[]},
      {"number":22,"title":"fiche","systems":["campaign"],"produces":[],"consumes":["Session"],"depends_on":[20]}
    ],"ids":{
      "Session":{"kind":"aggregate","producer":20,"state":"built","version":1},
      "session.listScenes":{"kind":"capability","producer":20,"state":"built","version":1}
    }}"#;

    fn after(pr: Pr, stop: Option<&str>) -> Context<FixState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            FixState {
                pr: Some(pr),
                contract_stop: stop.map(ToString::to_string),
                ..FixState::default()
            },
            Logbook::null(),
        )
    }

    fn board() -> FakeGitHub {
        FakeGitHub {
            files: std::collections::HashMap::from([(
                ("plan/graph.json".to_string(), "main_agent".to_string()),
                GRAPH.to_string(),
            )]),
            labels: vec![labels::CONTRACT_CHANGE.to_string()],
            issues: vec![
                harness_core::domain::Issue {
                    number: 500,
                    body: "## Plan\n\nCloses #482".to_string(),
                    ..harness_core::domain::Issue::default()
                },
                harness_core::domain::Issue {
                    number: 482,
                    state: "open".to_string(),
                    body: "## Scope\n\nbranch: feat/fiche".to_string(),
                    ..harness_core::domain::Issue::default()
                },
                harness_core::domain::Issue {
                    number: 22,
                    state: "open".to_string(),
                    body: "the milestone".to_string(),
                    ..harness_core::domain::Issue::default()
                },
            ],
            ..FakeGitHub::default()
        }
    }

    fn raise(gh: &Rc<FakeGitHub>) -> RaiseContract {
        RaiseContract {
            gh: Rc::clone(gh) as Rc<dyn GitHub>,
            integration: "main_agent".to_string(),
        }
    }

    const STOP: &str = "AGENT_LOOP_STOP: contract session.listScenes — a source field (/fix)";

    #[tokio::test]
    async fn a_repair_stopped_on_a_contract_opens_its_change_and_the_pr_waits() {
        let gh = Rc::new(board());
        let task_pr = Pr {
            num: "500".to_string(),
            head: "feat/fiche".to_string(),
            base: "milestone/22-fiche".to_string(),
            ..Pr::default()
        };
        raise(&gh)
            .run(&mut after(task_pr, Some(STOP)))
            .await
            .expect("asked");
        let writes = gh.writes();
        assert!(
            writes.iter().any(|w| matches!(
                w,
                Wrote::CreatedIssue(title, body, _)
                    if title == "Contract change: session.listScenes for #482"
                        && body.contains("milestone: #22")
            )),
            "{writes:?}"
        );
        assert!(writes.contains(&Wrote::BlockedByLink(482, 1)));
        assert!(writes.iter().any(|w| matches!(
            w,
            Wrote::Comment(500, said) if said.starts_with(&contest::contract_wait_marker(1))
        )));
        assert!(writes.contains(&Wrote::Label(500, labels::PR_FIX.to_string())));
    }

    #[tokio::test]
    async fn a_third_contract_stop_escalates_rather_than_waiting_again() {
        // Two stops, two changes landed, two resumes: the third stop meets
        // the waits cap — no new request, no new wait, no repair asked.
        let mut fixes = board();
        fixes.issues[0].body = "## Plan\n\nFixes #482".to_string();
        let gh = Rc::new(fixes);
        let task_pr = Pr {
            num: "500".to_string(),
            head: "feat/fiche".to_string(),
            base: "milestone/22-fiche".to_string(),
            ..Pr::default()
        };
        let mut context = after(task_pr, Some(STOP));
        context.state.comments = format!(
            "{}\n{}\n{}\n{}\n",
            contest::contract_wait_marker(503),
            contest::resumed_marker(503),
            contest::contract_wait_marker(507),
            contest::resumed_marker(507)
        );
        raise(&gh).run(&mut context).await.expect("escalated");
        let writes = gh.writes();
        assert!(
            !writes.iter().any(|w| matches!(
                w,
                Wrote::CreatedIssue(..) | Wrote::Label(500, _) | Wrote::BlockedByLink(..)
            )),
            "{writes:?}"
        );
        assert!(!writes.iter().any(|w| matches!(
            w,
            Wrote::Comment(500, said) if said.contains(contest::CONTRACT_WAIT)
        )));
        assert!(
            writes.contains(&Wrote::Label(482, labels::HUMAN.to_string())),
            "the task named by `Fixes #482` is a human's past the cap: {writes:?}"
        );
        // One wait left: the stop still waits on its change.
        let gh = Rc::new(board());
        let mut context = after(
            Pr {
                num: "500".to_string(),
                head: "feat/fiche".to_string(),
                base: "milestone/22-fiche".to_string(),
                ..Pr::default()
            },
            Some(STOP),
        );
        context.state.comments = format!(
            "{}\n{}\n",
            contest::contract_wait_marker(503),
            contest::resumed_marker(503)
        );
        raise(&gh).run(&mut context).await.expect("asked");
        assert!(gh.writes().iter().any(|w| matches!(
            w,
            Wrote::Comment(500, said) if said.starts_with(&contest::contract_wait_marker(1))
        )));
    }

    #[tokio::test]
    async fn a_landing_pr_asks_for_its_milestone_and_a_second_stop_reuses_the_request() {
        let mut open = board();
        open.issues.push(harness_core::domain::Issue {
            number: 503,
            state: "open".to_string(),
            labels: vec![labels::CONTRACT_CHANGE.to_string()],
            body: crate::contract_manager::data::request::Request {
                id: "session.listScenes".to_string(),
                task: 482,
                milestone: Some(22),
                need: "a source field".to_string(),
            }
            .render(),
            ..harness_core::domain::Issue::default()
        });
        let gh = Rc::new(open);
        let landing = Pr {
            num: "423".to_string(),
            head: "milestone/22-fiche".to_string(),
            base: "main_agent".to_string(),
            ..Pr::default()
        };
        raise(&gh)
            .run(&mut after(landing, Some(STOP)))
            .await
            .expect("asked");
        let writes = gh.writes();
        assert!(
            !writes.iter().any(|w| matches!(
                w,
                Wrote::CreatedIssue(..) | Wrote::BlockedByLink(..) | Wrote::Body(..)
            )),
            "the open request is reused, nothing written on the milestone: {writes:?}"
        );
        assert!(writes.iter().any(|w| matches!(
            w,
            Wrote::Comment(423, said) if said.starts_with(&contest::contract_wait_marker(503))
        )));
    }

    #[tokio::test]
    async fn a_contract_of_the_pr_s_own_is_answered_with_no_decided_marker() {
        let gh = Rc::new(board());
        let task_pr = Pr {
            num: "500".to_string(),
            head: "feat/fiche".to_string(),
            base: "milestone/22-fiche".to_string(),
            ..Pr::default()
        };
        raise(&gh)
            .run(&mut after(
                task_pr,
                Some("AGENT_LOOP_STOP: contract Session — an invariant"),
            ))
            .await
            .expect("answered");
        let writes = gh.writes();
        let said = writes
            .iter()
            .find_map(|w| match w {
                Wrote::Comment(500, said) => Some(said.clone()),
                _ => None,
            })
            .expect("answered on the PR");
        assert!(said.contains("is an aggregate"), "{said}");
        assert!(
            !said.contains(crate::common::review::DECIDED_MARKER),
            "the cap holds: {said}"
        );
        assert!(writes.contains(&Wrote::Label(500, labels::PR_FIX.to_string())));
        assert!(!writes.iter().any(|w| matches!(w, Wrote::CreatedIssue(..))));
    }

    #[tokio::test]
    async fn a_stop_with_nobody_to_ask_for_stays_a_halt_and_no_stop_does_nothing() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![harness_core::domain::Issue {
                number: 77,
                body: "no closing line".to_string(),
                ..harness_core::domain::Issue::default()
            }],
            ..FakeGitHub::default()
        });
        let orphan = Pr {
            num: "77".to_string(),
            head: "fix/x".to_string(),
            base: "main_agent".to_string(),
            ..Pr::default()
        };
        assert!(matches!(
            raise(&gh).run(&mut after(orphan.clone(), Some(STOP))).await,
            Err(Halt::Halted(_))
        ));
        raise(&gh)
            .run(&mut after(orphan, None))
            .await
            .expect("nothing to do");
        assert_eq!(gh.writes(), [] as [Wrote; 0]);
    }

    fn pushing(
        git: &crate::common::fake_git::FakeRepos,
        gh: &Rc<FakeGitHub>,
    ) -> PushWhatWasCommitted {
        PushWhatWasCommitted {
            gh: Rc::clone(gh) as Rc<dyn GitHub>,
            git: harness_core::ports::shell::git::Repos::at(git, std::path::Path::new("/w")),
        }
    }

    fn ran(ok: bool) -> Context<FixState> {
        let mut context = after(
            Pr {
                num: "496".to_string(),
                head: "feat/x".to_string(),
                ..Pr::default()
            },
            None,
        );
        context.state.answered_ok = Some(ok);
        context
    }

    #[tokio::test]
    async fn committed_but_unpushed_is_pushed_only_on_ok() {
        let gh = Rc::new(FakeGitHub::default());
        let git = crate::common::fake_git::FakeRepos::default().with_unpushed(&["abc fix: merge"]);
        pushing(&git, &gh)
            .run(&mut ran(true))
            .await
            .expect("pushed");
        assert!(git.asked("push HEAD:feat/x"), "{:?}", git.calls());
        assert!(!git.asked("reset_hard"), "{:?}", git.calls());
        // A session that said "push nothing" never pushes to the PR.
        let git = crate::common::fake_git::FakeRepos::default().with_unpushed(&["abc partial"]);
        pushing(&git, &gh).run(&mut ran(false)).await.expect("said");
        assert!(!git.asked("push HEAD:feat/x"), "{:?}", git.calls());
    }

    #[tokio::test]
    async fn a_reply_without_its_marker_sets_its_commit_aside_and_clears_the_checkout() {
        // ForgeDesMondes #496, 12:25: no AGENT_LOOP_OK marker, one commit
        // ahead — left in router-lane-0, it halted the next mount at 18:05.
        let gh = Rc::new(FakeGitHub::default());
        let git = crate::common::fake_git::FakeRepos::default()
            .with_unpushed(&["9f1c2d3 fix: merge milestone/24-table"]);
        pushing(&git, &gh)
            .run(&mut ran(false))
            .await
            .expect("set aside");
        assert_eq!(
            git.calls()
                .iter()
                .filter_map(|call| call.split_once(": ").map(|(_, verb)| verb.to_string()))
                .filter(|verb| verb.starts_with("push") || verb.starts_with("reset_hard"))
                .collect::<Vec<_>>(),
            [
                "push HEAD:harness/unpushed/496-abc1234".to_string(),
                "reset_hard origin/feat/x".to_string()
            ],
            "set aside first, then the reset"
        );
        // Nothing committed: nothing set aside, nothing reset.
        let git = crate::common::fake_git::FakeRepos::default();
        pushing(&git, &gh).run(&mut ran(false)).await.expect("said");
        assert!(!git.asked("push") && !git.asked("reset_hard"));
    }

    #[test]
    fn the_aside_branch_names_the_pr_and_a_short_sha() {
        assert_eq!(
            aside_branch("496", "0123456789abcdef0123"),
            "harness/unpushed/496-0123456789ab"
        );
        assert_eq!(aside_branch("7", "abc"), "harness/unpushed/7-abc");
    }

    #[tokio::test]
    async fn nothing_committed_is_said_and_nothing_pushed() {
        let gh = Rc::new(FakeGitHub {
            branch_shas: std::collections::HashMap::from([(
                "feat/x".to_string(),
                Some("s1".to_string()),
            )]),
            ..FakeGitHub::default()
        });
        let git = crate::common::fake_git::FakeRepos::default();
        let mut context = ran(true);
        context.state.head_before = Some("s1".to_string());
        pushing(&git, &gh).run(&mut context).await.expect("said");
        assert!(!git.asked("push"));
        // A session that never answered is left alone.
        let mut silent = ran(true);
        silent.state.answered_ok = None;
        let git = crate::common::fake_git::FakeRepos::default().with_unpushed(&["abc"]);
        pushing(&git, &gh).run(&mut silent).await.expect("left");
        assert_eq!(git.calls(), [] as [String; 0]);
        // One that stopped on a contract never pushes to the PR: what it
        // committed is set aside.
        let mut stopped = ran(true);
        stopped.state.answered_ok = None;
        stopped.state.contract_stop = Some(STOP.to_string());
        let git = crate::common::fake_git::FakeRepos::default().with_unpushed(&["abc"]);
        pushing(&git, &gh)
            .run(&mut stopped)
            .await
            .expect("set aside");
        assert!(!git.asked("push HEAD:feat/x"), "{:?}", git.calls());
        assert!(
            git.asked("push HEAD:harness/unpushed/496-"),
            "{:?}",
            git.calls()
        );
        assert!(git.asked("reset_hard origin/feat/x"), "{:?}", git.calls());
    }
}
