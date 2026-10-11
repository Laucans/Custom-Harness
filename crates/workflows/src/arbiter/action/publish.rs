//! The judgement, written: the task's `needs` and `blocked_by`, a
//! producer's `produces` where an id was nobody's, the verdict under
//! `## Decision`, a comment — and the human's label for what is theirs.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;

use crate::arbiter::data::board::{self, Judgement};
use crate::arbiter::data::state::ArbiterState;
use crate::arbiter::data::verdict;
use crate::common::{labels, sections};

/// Writes the judgement.
pub struct Apply {
    /// What rewrites bodies, links and labels.
    pub gh: Rc<dyn GitHub>,
    /// The stage whose reply carries the verdict, when the session ran.
    pub arbitrate: String,
}

#[async_trait(?Send)]
impl Action<ArbiterState> for Apply {
    async fn run(&self, ctx: &mut Context<ArbiterState>) -> Outcome<Verdict> {
        let issue = ctx.state.issue().clone();
        let num = issue.number;
        let question = ctx.state.question().clone();
        let entries = board::entries(num, &ctx.state.siblings);
        // `VerdictParses` already demanded this parses when the stage ran;
        // a reply that does not is a gate that did not run, not a case.
        let read = match ctx.results.get(&self.arbitrate) {
            Some(reply) => Some(verdict::parse(&reply.text).map_err(|e| {
                Halt::Failed(format!(
                    "the verdict still did not parse ({e}) — VerdictParses should have \
                     stopped the round before this stage ever ran"
                ))
            })?),
            None => None,
        };
        let judged = board::judge(&question, &ctx.state.resolution, &entries, read.as_ref());
        let said = judged.verdict_line();
        ctx.state.outcome = Some(judged.by.map_or_else(
            || format!("#{num}: {said}"),
            |by| format!("#{num}: settled by {by} — {said}"),
        ));
        if ctx.settings.dry_run {
            ctx.traces.say(&format!("dry run — would write: {said}"));
            return Ok(Verdict::Continue);
        }

        self.write_task(num, &issue.body, &question, &judged)
            .await?;
        for link in &judged.links {
            if !issue.blocked_by.iter().any(|b| b.number == link.producer) {
                self.gh.add_blocked_by(num, link.producer).await?;
            }
            if link.reassigned {
                self.widen_producer(ctx, link.producer, &link.id).await?;
            }
        }
        let comment = match (&judged.human, judged.by) {
            (Some(why), _) => format!(
                "Arbiter — for a human: {why}\n\nWhat was settled anyway: {}\n\nAnswer under \
                 `## Decision` (or edit `## Architecture`), then remove `{}`.",
                settled_line(&judged),
                labels::NEEDS_DECISION
            ),
            (None, Some(by)) => format!("Arbiter — settled by {by}: {said}"),
            (None, None) => format!("Arbiter — {said}"),
        };
        self.gh.post_issue_comment(num, &comment).await?;
        if judged.human.is_some() && !issue.has(labels::NEEDS_DECISION) {
            self.gh.add_label(num, labels::NEEDS_DECISION).await?;
        }
        ctx.traces.say(&format!(
            "#{num} — {}",
            ctx.state.outcome.as_deref().unwrap_or_default()
        ));
        Ok(Verdict::Continue)
    }
}

/// The mechanical part of a judgement a human receives: what was chained
/// before the question reached them.
fn settled_line(judged: &Judgement) -> String {
    let kept = Judgement {
        human: None,
        ..judged.clone()
    };
    kept.verdict_line()
}

impl Apply {
    /// The task's body: `needs` widened by every chained or present id, the
    /// verdict under `## Decision`, everything else as it was.
    async fn write_task(
        &self,
        num: u64,
        body: &str,
        question: &crate::common::decision::Question,
        judged: &Judgement,
    ) -> Outcome<()> {
        let found = sections::parse(body);
        let Some(mut declaration) = found
            .get(sections::ARCHITECTURE)
            .and_then(|text| harness_architecture::vocabulary::Declaration::parse(text))
        else {
            return Err(Halt::Failed(format!(
                "#{num} has no parsable ## Architecture section — nothing to chain on"
            )));
        };
        for id in judged
            .links
            .iter()
            .map(|link| link.id.as_str())
            .chain(judged.present.iter().map(String::as_str))
        {
            if !declaration.needs.iter().any(|n| n.eq_ignore_ascii_case(id)) {
                declaration.needs.push(id.to_string());
            }
        }
        let mut decided = question.clone();
        decided.decided_by = judged.by.map(str::to_string);
        decided.verdict = Some(judged.verdict_line());
        let rewritten = sections::replace(
            body,
            &[
                (sections::ARCHITECTURE, &declaration.render()),
                (sections::DECISION, &decided.render()),
            ],
        );
        self.gh.set_body(num, &rewritten).await
    }

    /// A sibling now produces the id: its `## Architecture` says so.
    async fn widen_producer(
        &self,
        ctx: &Context<ArbiterState>,
        producer: u64,
        id: &str,
    ) -> Outcome<()> {
        let Some(sibling) = ctx.state.siblings.iter().find(|s| s.number == producer) else {
            return Ok(());
        };
        let Some(mut declaration) = board::declaration_of(sibling) else {
            return Ok(());
        };
        if declaration
            .produces
            .iter()
            .any(|p| p.eq_ignore_ascii_case(id))
        {
            return Ok(());
        }
        declaration.produces.push(id.to_string());
        let rewritten = sections::replace(
            &sibling.body,
            &[(sections::ARCHITECTURE, &declaration.render())],
        );
        self.gh.set_body(producer, &rewritten).await?;
        ctx.traces.say(&format!("#{producer} now produces {id}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::decision::Question;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    fn issue(number: u64, labels: &[&str], architecture: &str) -> Issue {
        Issue {
            number,
            title: format!("task {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: format!(
                "## Scope\n\nbranch: x\n\n## Architecture\n\n{architecture}\n\n## Owner Decisions\n\nkeep\n"
            ),
            ..Issue::default()
        }
    }

    fn ctx(dry_run: bool, needs: &[&str], verdict: Option<&str>) -> Context<ArbiterState> {
        let mut task = issue(
            20,
            &[],
            "unit: micro-ui\nsystem: c\nproduces: Pjs\nneeds: Campagne",
        );
        task.body = format!(
            "{}\n## Decision\n\nblocks: lists PJs\nneeds: {}\nround: 1\n",
            task.body,
            needs.join(", ")
        );
        let siblings = vec![
            issue(
                12,
                &[labels::READY],
                "unit: aggregate\nsystem: c\nproduces: PJ",
            ),
            issue(
                14,
                &[],
                "unit: data-capability\nsystem: c\nproduces: c.ajouterPj\nneeds: PJ",
            ),
        ];
        let entries = board::entries(20, &siblings);
        let asked: Vec<String> = needs.iter().map(|s| (*s).to_string()).collect();
        let resolution = board::resolve(&asked, &entries, &["Risk@3".to_string()]);
        let mut context = Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            ArbiterState {
                issue: Some(task),
                question: Some(Question {
                    blocks: "lists PJs".to_string(),
                    needs: asked,
                    round: 1,
                    ..Question::default()
                }),
                milestone: Some(5),
                siblings,
                resolution,
                ..ArbiterState::default()
            },
            Logbook::null(),
        );
        if let Some(text) = verdict {
            context.results.insert(
                "arbitrate".to_string(),
                Reply {
                    text: text.to_string(),
                    stop_line: None,
                    spend: Spend::default(),
                },
            );
        }
        context
    }

    fn apply(gh: &Rc<FakeGitHub>) -> Apply {
        Apply {
            gh: Rc::clone(gh) as Rc<dyn GitHub>,
            arbitrate: "arbitrate".to_string(),
        }
    }

    fn body_of(writes: &[Wrote], number: u64) -> String {
        writes
            .iter()
            .find_map(|w| match w {
                Wrote::Body(n, body) if *n == number => Some(body.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("a body for #{number}"))
    }

    #[tokio::test]
    async fn a_question_settled_by_the_harness_chains_needs_links_and_says_so() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, &["PJ", "Risk@3"], None);
        apply(&gh).run(&mut context).await.expect("applied");
        let writes = gh.writes();
        let body = body_of(&writes, 20);
        assert!(body.contains("needs: Campagne, PJ, Risk@3"), "{body}");
        assert!(body.contains("## Decision\n\nblocks: lists PJs\nneeds: PJ, Risk@3\nround: 1\ndecided-by: harness\nverdict: PJ ← #12; Risk@3 (already there)"), "{body}");
        assert!(
            body.contains("## Owner Decisions\n\nkeep"),
            "extras kept: {body}"
        );
        assert!(writes.contains(&Wrote::BlockedByLink(20, 12)));
        assert!(writes.iter().any(
            |w| matches!(w, Wrote::Comment(20, c) if c.starts_with("Arbiter — settled by harness"))
        ));
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::Label(..))),
            "no human needed"
        );
        assert!(
            context
                .state
                .outcome
                .as_deref()
                .is_some_and(|o| o.contains("settled by harness"))
        );
    }

    #[tokio::test]
    async fn a_verdict_applied_widens_the_producer_and_links_both() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(
            false,
            &["PJ", "Dice"],
            Some(
                r#"{"decision":"resolve","confidence":5,"needs":[{"id":"Dice","producer":14}],"reason":"same command"}"#,
            ),
        );
        apply(&gh).run(&mut context).await.expect("applied");
        let writes = gh.writes();
        assert!(body_of(&writes, 20).contains("needs: Campagne, PJ, Dice"));
        assert!(body_of(&writes, 20).contains("decided-by: ai"));
        assert!(body_of(&writes, 14).contains("produces: c.ajouterPj, Dice"));
        assert!(writes.contains(&Wrote::BlockedByLink(20, 12)));
        assert!(writes.contains(&Wrote::BlockedByLink(20, 14)));
    }

    #[tokio::test]
    async fn what_goes_to_a_human_keeps_the_mechanical_chains_and_poses_the_label() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(
            false,
            &["PJ", "Dice"],
            Some(r#"{"decision":"human","confidence":5,"reason":"Dice is another milestone's"}"#),
        );
        apply(&gh).run(&mut context).await.expect("applied");
        let writes = gh.writes();
        let body = body_of(&writes, 20);
        assert!(body.contains("needs: Campagne, PJ\n"), "{body}");
        assert!(body.contains("verdict: for a human — the arbiter did not settle it (human, confidence 5/5): Dice is another milestone's"), "{body}");
        assert!(!body.contains("decided-by"), "{body}");
        assert!(writes.contains(&Wrote::BlockedByLink(20, 12)));
        assert!(writes.contains(&Wrote::Label(20, labels::NEEDS_DECISION.to_string())));
        assert!(writes.iter().any(|w| matches!(w, Wrote::Comment(20, c) if c.contains("for a human") && c.contains("PJ ← #12"))));
    }

    #[tokio::test]
    async fn an_existing_link_is_not_drawn_twice_and_a_dry_run_writes_nothing() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, &["PJ"], None);
        context.state.issue.as_mut().expect("issue").blocked_by = vec![Issue {
            number: 12,
            ..Issue::default()
        }];
        apply(&gh).run(&mut context).await.expect("applied");
        assert!(
            !gh.writes()
                .iter()
                .any(|w| matches!(w, Wrote::BlockedByLink(..)))
        );

        let gh = Rc::new(FakeGitHub::default());
        let mut dry = ctx(true, &["PJ"], None);
        apply(&gh).run(&mut dry).await.expect("dry");
        assert!(gh.writes().is_empty());
        assert!(
            dry.state.outcome.is_some(),
            "the summary still says what would be written"
        );
    }

    #[tokio::test]
    async fn an_unparsable_verdict_is_a_failure_not_an_improvisation() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, &["Dice"], Some("no json"));
        assert!(apply(&gh).run(&mut context).await.is_err());
        assert!(gh.writes().is_empty());
    }
}
