//! What the round does when its session stops on an ambiguity, or the
//! breaker refuses to pay for it again: it asks for a decision instead of
//! waiting for a human.
//!
//! The stop is written under the task's `## Decision` (`kind: stop` or
//! `breaker`, `blocks:` the reason, `round:` how many times) and the task is
//! labelled `harness:needs-decision`: the `decide` workflow answers at the
//! next tick, the body changes — so the breaker's fingerprint moves — and the
//! loop takes the task again. Asked a third time, it is no longer the
//! harness's to settle: a `harness:human` task is opened, blocking it.
//!
//! A halt of any other kind — a gate that refused, an unreadable input — is
//! not a decision to take, and goes on as it was.

use std::rc::Rc;

use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::Context;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Event;

use crate::common::decision::Question;
use crate::common::{labels, sections};
use crate::dev_loop::data::state::Loop;

/// The round a stop on one task goes to a human instead of a decision.
pub const HUMAN_AT_ROUND: u32 = 3;

/// The most of a stop's reason the question keeps, in characters.
const REASON_MAX: usize = 600;

/// What kind of decision a halt asks for, if any: a session's own stop, or
/// the breaker's refusal.
#[must_use]
pub fn kind_of(halt: &Halt) -> Option<&'static str> {
    let Halt::Halted(reason) = halt else {
        return None;
    };
    if reason.contains("AGENT_LOOP_STOP") {
        Some("stop")
    } else if reason.starts_with("refusing to pay for") {
        Some("breaker")
    } else {
        None
    }
}

/// A stop's reason, on one line, cut to what a question holds.
fn one_line(reason: &str) -> String {
    let line = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    line.chars().take(REASON_MAX).collect()
}

/// Hands a stopped task over: a decision asked, or a human task opened.
pub struct RaiseDecision {
    /// What reads the task and writes its decision.
    pub gh: Rc<dyn GitHub>,
}

impl RaiseDecision {
    /// What the round does with `halt`: a stop or a refusal is handed over,
    /// and the round goes on; anything else is propagated.
    ///
    /// # Errors
    /// `halt` itself when it is no decision to take, or the GitHub writes'.
    pub async fn on_halt(&self, ctx: &mut Context<Loop>, halt: Halt) -> Outcome<Verdict> {
        let Some(kind) = kind_of(&halt) else {
            return Err(halt);
        };
        if ctx.settings.dry_run || !ctx.state.has_task() {
            return Err(halt);
        }
        let Ok(number) = ctx.state.task.number.parse::<u64>() else {
            return Err(halt);
        };
        let reason = one_line(halt.reason());
        let issue = self.gh.issue(number).await?;
        let asked = sections::parse(&issue.body)
            .get(sections::DECISION)
            .and_then(|text| Question::parse(text));
        let round = asked
            .filter(|q| matches!(q.kind.as_deref(), Some("stop" | "breaker")))
            .map_or(1, |q| q.round.saturating_add(1));
        if round >= HUMAN_AT_ROUND {
            self.to_a_human(ctx, number, &issue.title, &reason).await?;
        } else {
            let question = Question {
                kind: Some(kind.to_string()),
                blocks: reason.clone(),
                round,
                ..Question::default()
            };
            let body = sections::replace(&issue.body, &[(sections::DECISION, &question.render())]);
            self.gh.set_body(number, &body).await?;
            self.gh.add_label(number, labels::NEEDS_DECISION).await?;
            ctx.traces.say(&format!(
                "#{number}: the session stopped ({kind}, round {round}) — a decision is asked: {reason}"
            ));
        }
        ctx.state.handed_over = true;
        Ok(Verdict::Continue)
    }

    /// A stop the harness already decided twice: a task only a human can
    /// settle, blocking the stopped one.
    async fn to_a_human(
        &self,
        ctx: &Context<Loop>,
        number: u64,
        title: &str,
        reason: &str,
    ) -> Outcome<()> {
        let body = format!(
            "The session building #{number} stopped again after the harness decided twice:\n\n\
             > {reason}\n\nOnly a person can settle it: answer under #{number}'s `## Decision` \
             with `decided-by: owner`, then close this issue — #{number} waits on it."
        );
        let human = self
            .gh
            .create_issue(
                &format!("Decide for #{number}: {title}"),
                &body,
                &[labels::HUMAN],
            )
            .await?;
        if let Ok(milestone) = ctx.state.milestone.number.parse::<u64>() {
            self.gh.create_sub_issue_link(milestone, human).await?;
        }
        self.gh.add_blocked_by(number, human).await?;
        ctx.traces.event(&Event::HumanTaskOpened {
            task: human,
            blocks: number,
            why: format!("#{number} stopped a third time: {reason}"),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Issue;
    use harness_core::domain::Named;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            Loop {
                milestone: Named {
                    number: "4".to_string(),
                    ..Named::default()
                },
                task: Named {
                    number: "11".to_string(),
                    title: "Pay".to_string(),
                    body: String::new(),
                },
                task_key: "11".to_string(),
                ..Loop::default()
            },
            Logbook::null(),
        )
    }

    fn gh(body: &str) -> Rc<FakeGitHub> {
        Rc::new(FakeGitHub {
            issues: vec![Issue {
                number: 11,
                title: "Pay".to_string(),
                state: "open".to_string(),
                body: body.to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        })
    }

    fn stop() -> Halt {
        Halt::Halted("AGENT_LOOP_STOP: which payment provider? (/code)".to_string())
    }

    #[test]
    fn only_a_stop_or_a_refusal_is_a_decision_to_take() {
        assert_eq!(kind_of(&stop()), Some("stop"));
        assert_eq!(
            kind_of(&Halt::Halted(
                "refusing to pay for /code on #11: …".to_string()
            )),
            Some("breaker")
        );
        assert_eq!(
            kind_of(&Halt::Halted("issue #11 carries no SPEC".to_string())),
            None
        );
        assert_eq!(kind_of(&Halt::Failed("AGENT_LOOP_STOP".to_string())), None);
    }

    #[tokio::test]
    async fn a_stop_asks_for_a_decision_and_the_round_goes_on() {
        let gh = gh("## Scope\n\nbranch: feat/pay\n\npay");
        let mut context = ctx(false);
        let verdict = RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .on_halt(&mut context, stop())
        .await
        .expect("handed over");
        assert_eq!(verdict, Verdict::Continue);
        assert!(context.state.handed_over);
        let writes = gh.writes();
        let Wrote::Body(11, body) = &writes[0] else {
            panic!("{writes:?}");
        };
        let asked = Question::parse(
            sections::parse(body)
                .get(sections::DECISION)
                .map_or("", String::as_str),
        )
        .expect("a question");
        assert_eq!(asked.kind.as_deref(), Some("stop"));
        assert_eq!(asked.round, 1);
        assert!(asked.blocks.contains("which payment provider"));
        assert!(body.contains("branch: feat/pay"));
        assert_eq!(
            writes[1],
            Wrote::Label(11, labels::NEEDS_DECISION.to_string())
        );
    }

    #[tokio::test]
    async fn a_third_stop_opens_a_task_for_a_human_that_blocks_it() {
        let gh = gh(
            "## Decision\n\nkind: stop\nblocks: which provider\nround: 2\ndecided-by: ai\nverdict: answer — Stripe (3/5)",
        );
        let mut context = ctx(false);
        RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .on_halt(&mut context, stop())
        .await
        .expect("handed over");
        let writes = gh.writes();
        assert!(
            matches!(&writes[0], Wrote::CreatedIssue(title, _, labels) if title == "Decide for #11: Pay" && labels == &[labels::HUMAN.to_string()])
        );
        assert!(
            writes
                .iter()
                .any(|w| matches!(w, Wrote::SubIssueLink(4, _)))
        );
        assert!(
            writes
                .iter()
                .any(|w| matches!(w, Wrote::BlockedByLink(11, _)))
        );
        assert!(
            !writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())),
            "no third decision"
        );
        assert!(context.state.handed_over);
    }

    #[tokio::test]
    async fn any_other_halt_or_a_dry_run_goes_on_as_it_was() {
        let gh = gh("");
        let raise = RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        };
        let gate = Halt::Halted("issue #11 carries no SPEC".to_string());
        assert!(raise.on_halt(&mut ctx(false), gate).await.is_err());
        assert!(raise.on_halt(&mut ctx(true), stop()).await.is_err());
        assert!(gh.writes().is_empty());
    }
}
