//! What a decision writes on GitHub: the request taken, then the decision
//! written, carried out, labelled and told.
//!
//! **Taken before the session is paid for, not after it succeeds** — the
//! `pr_fix` reasoning: the route outranks the work it unblocks, so a request
//! left in place after a failed attempt would be asked again every poll.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Event;

use crate::common::{labels, review, sections};
use crate::decide::data::decision::{self, Action as Act, Decided};
use crate::decide::data::state::DecideState;

/// Takes `harness:needs-decision` off: the request is taken. When the owner
/// already answered, that is all there is to do.
pub struct ConsumeRequest {
    /// What removes the label.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<DecideState> for ConsumeRequest {
    async fn run(&self, ctx: &mut Context<DecideState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let number = ctx.state.issue().number;
        self.gh.remove_label(number, labels::NEEDS_DECISION).await?;
        if ctx.state.owner_decided {
            ctx.traces.say(&format!(
                "#{number}: the owner answered under ## Decision — nothing to decide"
            ));
        }
        Ok(Verdict::Continue)
    }
}

/// Writes the decision under `## Decision`, carries it out, labels it
/// `harness:ai-decided`, comments it and tells it.
pub struct Apply {
    /// What writes on GitHub.
    pub gh: Rc<dyn GitHub>,
    /// The stage whose reply holds the decision.
    pub decide_stage: String,
}

impl Apply {
    /// The decision as the session answered it.
    fn decided(&self, ctx: &Context<DecideState>) -> Outcome<Decided> {
        let Some(reply) = ctx.results.get(&self.decide_stage) else {
            return Err(Halt::Failed(format!(
                "no reply under {} — the decide stage did not run",
                self.decide_stage
            )));
        };
        decision::parse(
            &reply.text,
            ctx.state.pr.is_some(),
            ctx.state.issue().number,
        )
        .map_err(|why| {
            Halt::Failed(format!(
                "the decision did not read, even after one retry: {why} — DecisionParses should \
                 have stopped the round"
            ))
        })
    }

    /// The label, created first on a board that has not got it yet.
    async fn label(&self, number: u64) -> Outcome<()> {
        if !self
            .gh
            .labels()
            .await?
            .iter()
            .any(|l| l == labels::AI_DECIDED)
        {
            let described = labels::ALL
                .iter()
                .find(|l| l.name == labels::AI_DECIDED)
                .map_or("", |l| l.description);
            self.gh
                .create_label(labels::AI_DECIDED, "c5def5", described)
                .await?;
        }
        self.gh.add_label(number, labels::AI_DECIDED).await
    }

    /// The repair path: the decision marked on the PR, so its repairs are
    /// counted anew, and `harness:pr-fix` posed.
    async fn repair(&self, pr: &harness_core::domain::Pr, decided: &Decided) -> Outcome<Act> {
        let number = pr_number(pr)?;
        self.gh
            .post_issue_comment(
                number,
                &format!(
                    "{}\nThe harness decided to repair this once more: {}\n\n{}",
                    review::DECIDED_MARKER,
                    decided.summary,
                    decided.answer
                ),
            )
            .await?;
        self.gh.add_label(number, labels::PR_FIX).await?;
        Ok(Act::Repair)
    }

    /// A merge decided while the PR's checks still run: nothing merges now.
    /// The PR gets the review that lets it through, so the merge train —
    /// a reviewed, mergeable, green PR of a `review-pending` task, or the
    /// rules' task's PR carrying this very decision
    /// (`milestone_merge::rules_ready_to_merge`) — takes it once green and
    /// closes the task, and the task says so.
    async fn merge_when_green(
        &self,
        task: &harness_core::domain::Issue,
        pr: &harness_core::domain::Pr,
        decided: &Decided,
    ) -> Outcome<Act> {
        self.gh
            .post_issue_comment(
                pr_number(pr)?,
                &format!(
                    "{}\n{}\n{}\nThe harness decided to merge this as it is: {}\n\nIt merges \
                     when its checks are green.",
                    review::MARKER,
                    review::CLEAN,
                    review::DECIDED_MERGE,
                    decided.summary
                ),
            )
            .await?;
        self.gh
            .post_issue_comment(
                task.number,
                &format!(
                    "PR {} merges when its checks are green — they are still running; the \
                     watch merges it at the first tick it is green and mergeable, and closes \
                     this task.",
                    pr.reference()
                ),
            )
            .await?;
        Ok(Act::Merge)
    }

    /// What the decision does beyond being written; the action carried out.
    /// A `merge` on a PR that conflicts, or whose checks failed, is a
    /// `repair`; one whose checks still run merges once they are green,
    /// never now.
    async fn carry_out(&self, ctx: &Context<DecideState>, decided: &Decided) -> Outcome<Act> {
        let issue = ctx.state.issue();
        match (decided.action, &ctx.state.pr) {
            (Act::Merge, Some(pr)) => {
                if self.gh.pr_mergeable(&pr.num).await? == Some(false) {
                    return self.repair(pr, decided).await;
                }
                match crate::milestone_merge::run(self.gh.as_ref(), issue, pr).await? {
                    crate::milestone_merge::Attempt::Merged => Ok(Act::Merge),
                    crate::milestone_merge::Attempt::Pending => {
                        self.merge_when_green(issue, pr, decided).await
                    }
                    crate::milestone_merge::Attempt::Red(_) => self.repair(pr, decided).await,
                }
            }
            (Act::Repair, Some(pr)) => self.repair(pr, decided).await,
            (Act::Cut, pr) => {
                self.gh
                    .post_issue_comment(
                        issue.number,
                        &format!(
                            "Cut by the harness: {}{}",
                            decided.summary,
                            pr.as_ref()
                                .map(|pr| format!(
                                    " — PR {} is left open, its branch kept",
                                    pr.reference()
                                ))
                                .unwrap_or_default()
                        ),
                    )
                    .await?;
                self.gh.close_issue(issue.number).await?;
                Ok(Act::Cut)
            }
            (Act::Answer, _) if issue.has(labels::RULES) => {
                // The rules' writer answers the decision: its tap opened again.
                self.gh.add_label(issue.number, labels::READY).await?;
                Ok(Act::Answer)
            }
            (Act::Wait { on }, _) => {
                // The blocker holds the task until `on` lands, for free; one
                // already there is not posted twice.
                let blocked_by = self.gh.blocked_by(issue.number).await?;
                if !blocked_by.iter().any(|blocker| blocker.number == on) {
                    self.gh.add_blocked_by(issue.number, on).await?;
                }
                Ok(Act::Wait { on })
            }
            (action, _) => Ok(action),
        }
    }
}

/// What a comment adds for the action carried out: a wait names what the
/// task is blocked by.
fn carried(done: Act) -> String {
    match done {
        Act::Wait { on } => format!("\n\nBlocked by #{on} until it lands; no session before."),
        _ => String::new(),
    }
}

/// The PR's number, as a label or a comment on it needs one.
fn pr_number(pr: &harness_core::domain::Pr) -> Outcome<u64> {
    pr.num
        .parse()
        .map_err(|_| Halt::Failed(format!("PR {} has no number to label", pr.reference())))
}

#[async_trait(?Send)]
impl Action<DecideState> for Apply {
    async fn run(&self, ctx: &mut Context<DecideState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || ctx.state.owner_decided {
            return Ok(Verdict::Continue);
        }
        let decided = self.decided(ctx)?;
        // The issue as it is now, not as the precheck read it: an owner who
        // answered while the session thought is never decided over
        // (`ForgeDesMondes`' #143, answered at 17:45 and overwritten at 17:46).
        let issue = self.gh.issue(ctx.state.issue().number).await?;
        let owner_meanwhile = sections::parse(&issue.body)
            .get(sections::DECISION)
            .and_then(|text| crate::common::decision::Question::parse(text))
            .is_some_and(|q| q.decided_by_owner());
        if owner_meanwhile {
            ctx.traces.say(&format!(
                "{}: the owner answered under ## Decision meanwhile — the harness's answer is dropped",
                issue.reference()
            ));
            return Ok(Verdict::Continue);
        }
        let section = decision::section(ctx.state.question.as_ref(), &ctx.state.kind, &decided);
        let body = sections::replace(&issue.body, &[(sections::DECISION, &section)]);
        self.gh.set_body(issue.number, &body).await?;
        let done = self.carry_out(ctx, &decided).await?;
        self.gh
            .post_issue_comment(
                issue.number,
                &format!(
                    "**The harness decided** ({}) — {}: {} ({}/5)\n\n{}{}\n\nTo overrule it, \
                     answer under `## Decision` with `decided-by: owner` and pose `{}` again; \
                     take `{}` off once read.",
                    ctx.state.kind,
                    done.name(),
                    decided.summary,
                    decided.confidence,
                    decided.answer,
                    carried(done),
                    labels::NEEDS_DECISION,
                    labels::AI_DECIDED
                ),
            )
            .await?;
        self.label(issue.number).await?;
        ctx.traces.event(&Event::DecisionTaken {
            issue: issue.number,
            kind: ctx.state.kind.clone(),
            action: done.name().to_string(),
            confidence: decided.confidence,
            summary: decided.summary.clone(),
        });
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::{Issue, Pr, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    fn ctx(answer: &str, pr: Option<Pr>, labels_now: &[&str]) -> Context<DecideState> {
        let mut ctx = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            DecideState {
                issue: Some(Issue {
                    number: 15,
                    body: "## Scope\n\nbranch: feat/pjs\n\nthe list".to_string(),
                    labels: labels_now.iter().map(ToString::to_string).collect(),
                    state: "open".to_string(),
                    ..Issue::default()
                }),
                kind: "review".to_string(),
                pr,
                ..DecideState::default()
            },
            Logbook::null(),
        );
        ctx.results.insert(
            "decide".to_string(),
            Reply {
                text: answer.to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        ctx
    }

    fn pr() -> Pr {
        Pr {
            num: "40".to_string(),
            head: "feat/pjs".to_string(),
            base: "milestone/3-x".to_string(),
            ..Pr::default()
        }
    }

    /// The issue the precheck read, as the fake answers `issue(15)`.
    fn fifteen() -> Issue {
        Issue {
            number: 15,
            body: "## Scope\n\nbranch: feat/pjs\n\nthe list".to_string(),
            state: "open".to_string(),
            ..Issue::default()
        }
    }

    fn apply(gh: &Rc<FakeGitHub>) -> Apply {
        Apply {
            gh: Rc::clone(gh) as Rc<dyn GitHub>,
            decide_stage: "decide".to_string(),
        }
    }

    #[tokio::test]
    async fn a_repair_once_more_is_marked_on_the_pr_and_the_decision_written() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![fifteen()],
            ..FakeGitHub::default()
        });
        let mut context = ctx(
            r#"{"action":"repair","summary":"fix the null check","answer":"- keep the API","confidence":4}"#,
            Some(pr()),
            &[],
        );
        apply(&gh).run(&mut context).await.expect("applied");
        let writes = gh.writes();
        let Wrote::Body(15, body) = &writes[0] else {
            panic!("the body first: {writes:?}");
        };
        let decision = sections::parse(body)
            .get(sections::DECISION)
            .cloned()
            .unwrap_or_default();
        assert!(
            decision.contains("decided-by: ai") && decision.contains("- keep the API"),
            "{decision}"
        );
        assert!(
            body.contains("branch: feat/pjs"),
            "the rest of the body kept"
        );
        assert!(
            matches!(&writes[1], Wrote::Comment(40, said) if said.contains(review::DECIDED_MARKER))
        );
        assert_eq!(writes[2], Wrote::Label(40, labels::PR_FIX.to_string()));
        assert!(matches!(&writes[3], Wrote::Comment(15, said) if said.contains("To overrule it")));
        assert!(
            writes.contains(&Wrote::CreatedLabel(
                labels::AI_DECIDED.to_string(),
                "c5def5".to_string(),
                labels::ALL
                    .iter()
                    .find(|l| l.name == labels::AI_DECIDED)
                    .map(|l| l.description.to_string())
                    .unwrap_or_default()
            ))
        );
        assert_eq!(
            writes.last(),
            Some(&Wrote::Label(15, labels::AI_DECIDED.to_string()))
        );
    }

    const MERGE: &str =
        r#"{"action":"merge","summary":"the review asks for style only","confidence":5}"#;

    /// A board whose PR #40 has the checks given: green, or not with these
    /// failing.
    fn with_checks(green: bool, failing: &[&str]) -> FakeGitHub {
        FakeGitHub {
            issues: vec![fifteen()],
            labels: vec![labels::AI_DECIDED.to_string()],
            pr_checks: HashMap::from([("40".to_string(), green)]),
            failing_checks: HashMap::from([(
                "40".to_string(),
                failing.iter().map(ToString::to_string).collect(),
            )]),
            ..FakeGitHub::default()
        }
    }

    #[tokio::test]
    async fn a_merge_as_it_is_merges_unless_the_pr_conflicts() {
        let gh = Rc::new(with_checks(true, &[]));
        let mut context = ctx(MERGE, Some(pr()), &[labels::REVIEW_PENDING]);
        apply(&gh).run(&mut context).await.expect("applied");
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::MergedPr("40".to_string())));
        assert!(
            writes.contains(&Wrote::Closed(15)),
            "milestone_merge closes the task"
        );
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::CreatedLabel(..))),
            "the label exists"
        );
        let conflicting = Rc::new(FakeGitHub {
            pr_mergeable: HashMap::from([("40".to_string(), Some(false))]),
            ..with_checks(true, &[])
        });
        let mut context = ctx(MERGE, Some(pr()), &[labels::REVIEW_PENDING]);
        apply(&conflicting)
            .run(&mut context)
            .await
            .expect("applied");
        let writes = conflicting.writes();
        assert!(!writes.contains(&Wrote::MergedPr("40".to_string())));
        assert!(
            writes.contains(&Wrote::Label(40, labels::PR_FIX.to_string())),
            "repaired instead"
        );
    }

    #[tokio::test]
    async fn a_merge_decided_on_red_checks_is_a_repair() {
        // The live failure: PR #231 merged by a decision with 7 red checks.
        let gh = Rc::new(with_checks(false, &["capability-isolation — FAILURE"]));
        let mut context = ctx(MERGE, Some(pr()), &[labels::REVIEW_PENDING]);
        apply(&gh).run(&mut context).await.expect("applied");
        let writes = gh.writes();
        assert!(!writes.contains(&Wrote::MergedPr("40".to_string())));
        assert!(
            matches!(&writes[1], Wrote::Comment(40, said) if said.contains(review::DECIDED_MARKER)),
            "{writes:?}"
        );
        assert_eq!(writes[2], Wrote::Label(40, labels::PR_FIX.to_string()));
        assert!(
            matches!(&writes[3], Wrote::Comment(15, said) if said.contains("— repair:")),
            "{writes:?}"
        );
    }

    #[tokio::test]
    async fn a_merge_decided_on_pending_checks_waits_for_the_merge_train() {
        let gh = Rc::new(with_checks(false, &[]));
        let mut context = ctx(MERGE, Some(pr()), &[labels::REVIEW_PENDING]);
        apply(&gh).run(&mut context).await.expect("applied");
        let writes = gh.writes();
        assert!(!writes.contains(&Wrote::MergedPr("40".to_string())));
        assert!(!writes.contains(&Wrote::Closed(15)), "the task stays open");
        assert!(
            !writes.contains(&Wrote::Label(40, labels::PR_FIX.to_string())),
            "no repair asked"
        );
        // The PR reads as reviewed and clean: what the merge train takes.
        let on_pr = writes
            .iter()
            .find_map(|w| match w {
                Wrote::Comment(40, said) => Some(said.clone()),
                _ => None,
            })
            .expect("a comment on the PR");
        assert_eq!(review::status(&on_pr), review::Status::Clean);
        assert!(on_pr.contains(review::DECIDED_MERGE));
        assert!(
            writes.iter().any(|w| matches!(
                w,
                Wrote::Comment(15, said) if said.contains("merges when its checks are green")
            )),
            "{writes:?}"
        );
        // The decision is written as a merge, and no decision is left open.
        assert!(
            writes.iter().any(|w| matches!(
                w,
                Wrote::Comment(15, said) if said.contains("— merge:")
            )),
            "{writes:?}"
        );
        assert!(
            !writes.contains(&Wrote::Label(15, labels::NEEDS_DECISION.to_string())),
            "no decision asked again"
        );
    }

    #[tokio::test]
    async fn a_rules_task_decided_merged_on_pending_checks_is_left_to_the_train() {
        // The live failure's own PR (#231) was a rules contest, decided
        // right after the judge's second round, before the writer's push
        // finished CI: pending is the common case for a rules decision.
        // The train reads a rules PR carrying this decision; nothing else
        // may be posed, or the writer and the judge run again over it.
        let gh = Rc::new(FakeGitHub {
            issues: vec![Issue {
                labels: vec![labels::RULES.to_string()],
                ..fifteen()
            }],
            ..with_checks(false, &[])
        });
        let rules_pr = Pr {
            num: "40".to_string(),
            head: crate::milestone_merge::rules_branch(15),
            base: "main_agent".to_string(),
            ..Pr::default()
        };
        let mut context = ctx(MERGE, Some(rules_pr.clone()), &[labels::RULES]);
        context.state.kind = "rules".to_string();
        apply(&gh).run(&mut context).await.expect("applied");
        let writes = gh.writes();
        assert!(!writes.contains(&Wrote::MergedPr("40".to_string())));
        assert!(!writes.contains(&Wrote::Closed(15)));
        assert!(
            !writes.contains(&Wrote::Label(15, labels::READY.to_string())),
            "the writer is not asked again: {writes:?}"
        );
        assert!(
            !writes.contains(&Wrote::Label(15, labels::NEEDS_DECISION.to_string())),
            "no decision asked again"
        );
        assert!(
            !writes.contains(&Wrote::Label(40, labels::PR_FIX.to_string())),
            "no repair asked"
        );
        // What the train reads: the decision is the PR's last review.
        let on_pr = writes
            .iter()
            .find_map(|w| match w {
                Wrote::Comment(40, said) => Some(said.clone()),
                _ => None,
            })
            .expect("a comment on the PR");
        assert!(review::decided_to_merge(&on_pr));
        let rules = [gh.issues[0].clone()];
        let open = [rules_pr];
        let later = FakeGitHub {
            pr_comment_bodies: vec![("40".to_string(), on_pr)],
            pr_checks: HashMap::from([("40".to_string(), true)]),
            ..FakeGitHub::default()
        };
        let taken = crate::milestone_merge::rules_ready_to_merge(&later, &rules, &open)
            .await
            .expect("read");
        assert_eq!(
            taken.map(|(t, p)| (t.number, p.num.as_str())),
            Some((15, "40"))
        );
    }

    #[tokio::test]
    async fn a_cut_closes_the_task_and_an_answer_only_writes() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![fifteen()],
            labels: vec![labels::AI_DECIDED.to_string()],
            ..FakeGitHub::default()
        });
        let mut context = ctx(r#"{"action":"cut","summary":"out of scope"}"#, None, &[]);
        apply(&gh).run(&mut context).await.expect("applied");
        assert!(gh.writes().contains(&Wrote::Closed(15)));
        let gh = Rc::new(FakeGitHub {
            issues: vec![fifteen()],
            labels: vec![labels::AI_DECIDED.to_string()],
            ..FakeGitHub::default()
        });
        let mut context = ctx(r#"{"action":"answer","summary":"Stripe"}"#, None, &[]);
        apply(&gh).run(&mut context).await.expect("applied");
        assert!(!gh.writes().contains(&Wrote::Closed(15)));
        assert!(
            gh.writes()
                .contains(&Wrote::Label(15, labels::AI_DECIDED.to_string()))
        );
    }

    #[tokio::test]
    async fn an_answer_on_the_rules_task_opens_its_writer_again() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![fifteen()],
            labels: vec![labels::AI_DECIDED.to_string()],
            ..FakeGitHub::default()
        });
        let mut context = ctx(
            r#"{"action":"answer","summary":"R3's test asserts too much: drop the count"}"#,
            None,
            &[labels::RULES],
        );
        context.state.kind = "rules".to_string();
        apply(&gh).run(&mut context).await.expect("applied");
        assert!(
            gh.writes()
                .contains(&Wrote::Label(15, labels::READY.to_string()))
        );
    }

    #[tokio::test]
    async fn an_owner_who_answered_while_the_session_thought_is_not_decided_over() {
        // The precheck read #15 with no owner answer; by the time the
        // harness writes, the owner has decided under ## Decision.
        let gh = Rc::new(FakeGitHub {
            issues: vec![Issue {
                number: 15,
                body: "## Scope\n\nbranch: feat/pjs\n\n## Decision\n\nkind: stop\nblocks: x\nround: 1\ndecided-by: owner\nverdict: wait".to_string(),
                state: "open".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        let mut context = ctx(
            r#"{"action":"answer","answer":"go on","summary":"go","confidence":4}"#,
            None,
            &[labels::NEEDS_DECISION],
        );
        apply(&gh).run(&mut context).await.expect("dropped");
        let writes = gh.writes();
        assert!(
            !writes
                .iter()
                .any(|w| matches!(w, Wrote::Body(..) | Wrote::Comment(..) | Wrote::Label(..))),
            "the owner's answer stands: {writes:?}"
        );
    }

    #[tokio::test]
    async fn an_owner_s_answer_or_a_dry_run_writes_no_decision() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![fifteen()],
            ..FakeGitHub::default()
        });
        let mut context = ctx(r#"{"action":"answer","summary":"x"}"#, None, &[]);
        context.state.owner_decided = true;
        apply(&gh).run(&mut context).await.expect("nothing");
        ConsumeRequest {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("taken");
        assert_eq!(
            gh.writes(),
            [Wrote::Unlabelled(15, labels::NEEDS_DECISION.to_string())]
        );
        let dry = Rc::new(FakeGitHub::default());
        let mut context = ctx(r#"{"action":"answer","summary":"x"}"#, None, &[]);
        context.settings.dry_run = true;
        ConsumeRequest {
            gh: Rc::clone(&dry) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("nothing");
        apply(&dry).run(&mut context).await.expect("nothing");
        let writes = dry.writes();
        assert!(writes.is_empty(), "{writes:?}");
    }

    #[derive(Default)]
    struct Capture(std::cell::RefCell<Vec<String>>);

    impl harness_core::traces::Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    #[tokio::test]
    async fn a_wait_blocks_the_task_on_what_it_waits_for_once_and_leaves_no_label_to_decide() {
        // ForgeDesMondes: 60 of a day's 82 decisions answered "keep waiting"
        // — each a body change that had the task taken again next tick.
        let gh = Rc::new(FakeGitHub {
            issues: vec![fifteen()],
            labels: vec![labels::AI_DECIDED.to_string()],
            ..FakeGitHub::default()
        });
        let mut context = ctx(
            r##"{"action":"wait","on":"#20","summary":"#20 has not landed its session crate","confidence":5}"##,
            None,
            &[],
        );
        context.state.kind = "stop".to_string();
        let capture = Rc::new(Capture::default());
        context.traces = Logbook::new(
            Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        apply(&gh).run(&mut context).await.expect("applied");
        let writes = gh.writes();
        let Wrote::Body(15, body) = &writes[0] else {
            panic!("the body first: {writes:?}");
        };
        assert!(
            body.contains("verdict: wait — #20 has not landed its session crate (5/5)"),
            "{body}"
        );
        assert_eq!(writes[1], Wrote::BlockedByLink(15, 20));
        assert!(
            matches!(&writes[2], Wrote::Comment(15, said) if said.contains("Blocked by #20 until it lands")),
            "{writes:?}"
        );
        assert_eq!(
            writes.last(),
            Some(&Wrote::Label(15, labels::AI_DECIDED.to_string()))
        );
        assert!(
            !writes.contains(&Wrote::Label(15, labels::NEEDS_DECISION.to_string())),
            "nothing calls decide again"
        );
        assert!(
            capture
                .0
                .borrow()
                .iter()
                .any(|line| line.contains("decide: #15 (stop) -> wait, 5/5")),
            "{:?}",
            capture.0.borrow()
        );

        // The blocker already there is not posted twice.
        let mut blocked = Issue {
            number: 15,
            state: "open".to_string(),
            ..Issue::default()
        };
        blocked.blocked_by = vec![Issue {
            number: 20,
            state: "open".to_string(),
            ..Issue::default()
        }];
        let again = Rc::new(FakeGitHub {
            issues: vec![fifteen()],
            labels: vec![labels::AI_DECIDED.to_string()],
            subs: vec![(4, vec![blocked])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(
            r#"{"action":"wait","on":20,"summary":"still #20"}"#,
            Some(pr()),
            &[labels::REVIEW_PENDING],
        );
        apply(&again).run(&mut context).await.expect("applied");
        let writes = again.writes();
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::BlockedByLink(..))),
            "{writes:?}"
        );
        assert!(
            !writes.contains(&Wrote::MergedPr("40".to_string()))
                && !writes.contains(&Wrote::Label(40, labels::PR_FIX.to_string())),
            "a wait touches no PR: {writes:?}"
        );
    }
}
