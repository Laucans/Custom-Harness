//! What an arbitration does before judging: reads the board, consumes its
//! label, asks the session.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_architecture::tree::Tree;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::arbiter::data::board;
use crate::arbiter::data::state::ArbiterState;
use crate::common::{labels, sections};

/// Settles mechanically what needs no judgement: a producer on the board,
/// an id the repository holds. Free, and before the label is consumed, so
/// a read that fails leaves the request for the next poll.
pub struct Resolve {
    /// What reads the checkout's manifests.
    pub disk: Rc<dyn Disk>,
    /// The checkout's root.
    pub root: PathBuf,
}

#[async_trait(?Send)]
impl Action<ArbiterState> for Resolve {
    async fn run(&self, ctx: &mut Context<ArbiterState>) -> Outcome<Verdict> {
        let task = ctx.state.issue().number;
        let needs = ctx.state.question().needs.clone();
        let existing = board::existing_ids(&Tree::read(self.disk.as_ref(), &self.root));
        let entries = board::entries(task, &ctx.state.siblings);
        let resolution = board::resolve(&needs, &entries, &existing);
        ctx.traces.say(&format!(
            "#{task}: {} id(s) asked — {} produced on the board, {} already there, {} open",
            needs.len(),
            resolution.chained.len(),
            resolution.present.len(),
            resolution.unresolved.len()
        ));
        ctx.state.existing = existing;
        ctx.state.resolution = resolution;
        Ok(Verdict::Continue)
    }
}

/// Consumes the request: one label, one arbitration, so a question nobody
/// can settle does not buy one session per poll.
pub struct Consume {
    /// What removes the label.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<ArbiterState> for Consume {
    async fn run(&self, ctx: &mut Context<ArbiterState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let issue = ctx.state.issue();
        // Only if it was there: `gh` returns 404 removing a missing label,
        // which is true for every run with `--force`.
        if !issue.has(labels::NEEDS_AI_DECISION) {
            return Ok(Verdict::Continue);
        }
        let number = issue.number;
        self.gh
            .remove_label(number, labels::NEEDS_AI_DECISION)
            .await?;
        ctx.traces.say(&format!(
            "#{number}: {} consumed — one arbitration follows",
            labels::NEEDS_AI_DECISION
        ));
        Ok(Verdict::Continue)
    }
}

/// Asks the session which task produces each id the harness could not
/// settle — over the board's index, never a refined SPEC.
pub struct AskArbiter {
    /// The stage name, under which the reply is recorded.
    pub stage: String,
    /// The prompt template, with its placeholders.
    pub template: String,
    /// Where the session's spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<ArbiterState> for AskArbiter {
    async fn run(&self, open: &mut Open<'_, ArbiterState>) -> Outcome<Verdict> {
        let issue = open.state.issue().clone();
        let question = open.state.question().clone();
        let entries = board::entries(issue.number, &open.state.siblings);
        let architecture = sections::parse(&issue.body)
            .get(sections::ARCHITECTURE)
            .cloned()
            .unwrap_or_default();
        let settled: Vec<String> = open
            .state
            .resolution
            .chained
            .iter()
            .map(|(id, producer)| format!("- {id}: produced by #{producer}, chained"))
            .chain(
                open.state
                    .resolution
                    .present
                    .iter()
                    .map(|id| format!("- {id}: already in the repository")),
            )
            .collect();
        let existing = if open.state.existing.is_empty() {
            "- (none)".to_string()
        } else {
            open.state
                .existing
                .iter()
                .map(|id| format!("- {id}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let prompt = splice(
            &self.template,
            &[
                ("num", &issue.number.to_string()),
                ("title", &issue.title),
                (
                    "milestone",
                    &open
                        .state
                        .milestone
                        .map_or_else(|| "?".to_string(), |m| m.to_string()),
                ),
                ("decision", &question.render()),
                ("architecture", &architecture),
                ("tasks", &board::brief(&entries)),
                ("existing", &existing),
                (
                    "settled",
                    &if settled.is_empty() {
                        "- (nothing)".to_string()
                    } else {
                        settled.join("\n")
                    },
                ),
                (
                    "open",
                    &open
                        .state
                        .resolution
                        .unresolved
                        .iter()
                        .map(|id| format!("- {id}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ],
        );
        let task = issue.reference();
        ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::decision::Question;
    use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn issue(number: u64, labels: &[&str], architecture: &str) -> Issue {
        Issue {
            number,
            title: format!("task {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: format!("## Architecture\n\n{architecture}\n"),
            ..Issue::default()
        }
    }

    fn ctx(dry_run: bool, task_labels: &[&str]) -> Context<ArbiterState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            ArbiterState {
                issue: Some(issue(
                    20,
                    task_labels,
                    "unit: micro-ui\nsystem: c\nproduces: Pjs",
                )),
                question: Some(Question {
                    blocks: "x".to_string(),
                    needs: vec!["PJ".to_string(), "Risk@3".to_string(), "Dice".to_string()],
                    round: 1,
                    ..Question::default()
                }),
                milestone: Some(5),
                siblings: vec![issue(12, &[], "unit: aggregate\nsystem: c\nproduces: PJ")],
                ..ArbiterState::default()
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn resolve_reads_the_board_and_the_repository_before_anything_is_paid() {
        let disk = FakeDisk {
            existing: std::iter::once((
                PathBuf::from("/repo/crates/c/aggregates/risk/aggregate.json"),
                r#"{"aggregate":"Risk@3"}"#.to_string(),
            ))
            .collect(),
            ..FakeDisk::default()
        };
        let action = Resolve {
            disk: Rc::new(disk),
            root: PathBuf::from("/repo"),
        };
        let mut context = ctx(false, &[]);
        action.run(&mut context).await.expect("resolved");
        assert_eq!(context.state.existing, ["Risk@3"]);
        assert_eq!(context.state.resolution.chained, [("PJ".to_string(), 12)]);
        assert_eq!(context.state.resolution.present, ["Risk@3"]);
        assert_eq!(context.state.resolution.unresolved, ["Dice"]);
    }

    #[tokio::test]
    async fn consume_removes_the_label_once_and_never_dry() {
        let gh = Rc::new(FakeGitHub::default());
        let action = Consume {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        };
        action
            .run(&mut ctx(false, &[labels::NEEDS_AI_DECISION]))
            .await
            .expect("consumed");
        assert_eq!(
            gh.writes(),
            [Wrote::Unlabelled(20, labels::NEEDS_AI_DECISION.to_string())]
        );
        let gh = Rc::new(FakeGitHub::default());
        let action = Consume {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        };
        action
            .run(&mut ctx(true, &[labels::NEEDS_AI_DECISION]))
            .await
            .expect("dry");
        action.run(&mut ctx(false, &[])).await.expect("forced");
        assert!(
            gh.writes().is_empty(),
            "dry, or not carried: nothing removed"
        );
    }
}
