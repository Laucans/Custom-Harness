//! The issue body: assembled, saved to disk, then posted.
//!
//! Written before posting: if `gh` fails, the text still exists and the error
//! message says where — paid sections don't vanish because a network call failed.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::github::GitHub;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};

use crate::common::labels;
use crate::refinement::data::state::RefinementState;
use crate::refinement::data::{rounds, sections};

/// The publishing step: body, comment, labels.
pub struct Write {
    /// What rewrites the body and labels.
    pub gh: Rc<dyn GitHub>,
    /// The coherence stage name, where its output is stored in
    /// `ctx.results`. Received from the table, like every stage name.
    pub coherence: String,
    /// The artifacts folder for this issue.
    pub refinement_dir: PathBuf,
}

#[async_trait(?Send)]
impl Action<RefinementState> for Write {
    async fn run(&self, ctx: &mut Context<RefinementState>) -> Outcome<Verdict> {
        let issue = ctx.state.issue().clone();
        let num = issue.number;

        let mut found = sections::merge(&ctx.state.found, &ctx.state.wanted, &ctx.results);

        // Coherence read this merge and may have retouched it. `None` covers
        // dry-run and any resumption that skipped it — nothing to apply. Output
        // that doesn't keep all keys is ignored entirely rather than partially
        // applied: retouching three sections then losing the fourth creates
        // one more inconsistency instead of fixing one. Sessions are already
        // paid, so this round publishes anyway — worst case without retouches,
        // never failing on them.
        if let Some(reconciled) = ctx.results.get(&self.coherence) {
            let retouched = sections::parse(&reconciled.text);
            let dropped: Vec<&str> = found
                .keys()
                .filter(|key| {
                    retouched
                        .get(*key)
                        .map(|t| t.trim())
                        .unwrap_or_default()
                        .is_empty()
                })
                .map(String::as_str)
                .collect();
            if dropped.is_empty() {
                found = retouched;
            } else {
                ctx.traces.warn(&format!(
                    "the coherence pass dropped {} from the body — publishing \
                     without its retouches",
                    dropped.join(" ")
                ));
            }
        }

        let body = sections::render(&found);
        if body.trim().is_empty() {
            return Err(Halt::Failed(format!(
                "nothing to write into #{num} — no stage produced a section \
                 and the body carried none"
            )));
        }

        let path = self
            .refinement_dir
            .join(format!("{num}-r{:02}-body.md", ctx.state.round_no));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Halt::Failed(format!("impossible de créer {} : {e}", parent.display()))
            })?;
        }
        std::fs::write(&path, &body)
            .map_err(|e| Halt::Failed(format!("failed to write {}: {e}", path.display())))?;

        self.gh.set_body(num, &body).await?;

        // Only if it was there: `gh` returns 404 removing a missing label,
        // which is true for every run with `--force`.
        if issue.has(labels::REFINEMENT) {
            self.gh.remove_label(num, labels::REFINEMENT).await?;
        }

        // Three sections are enough for a human to act: their task is
        // specified by round 1.
        let due = if issue.has(labels::HUMAN) { 1 } else { 2 };
        if ctx.state.round_no >= due {
            self.gh.add_label(num, labels::SPEC_WRITTEN).await?;
        }

        // Counter last: it's what says this round happened. Posted before a
        // failing label, it would restart resumption at the next round.
        self.gh
            .post_issue_comment(num, &rounds::comment(ctx.state.round_no))
            .await?;

        ctx.traces.say(&format!(
            "#{num} — round {} written ({})",
            ctx.state.round_no,
            ctx.state.wanted.join(" ")
        ));
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::adapters::agent::Reply;
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    /// The name the table gives to the coherence stage.
    const COHERENCE: &str = "coherence";

    fn dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "harness-refinement-publish-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn state(round_no: u32, issue_labels: &[&str]) -> RefinementState {
        RefinementState {
            issue: Some(Issue {
                number: 25,
                labels: issue_labels.iter().map(|l| (*l).to_string()).collect(),
                ..Issue::default()
            }),
            round_no,
            wanted: vec!["business-goal".to_string()],
            ..RefinementState::default()
        }
    }

    fn ctx(round_no: u32, issue_labels: &[&str]) -> Context<RefinementState> {
        let mut context = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            state(round_no, issue_labels),
            Logbook::null(),
        );
        context.results.insert(
            "business-goal".to_string(),
            Reply {
                text: "the lot goal".to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        context
    }

    #[tokio::test]
    async fn round_one_writes_the_body_but_not_spec_written_yet() {
        let review_dir = dir("round-one");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            refinement_dir: review_dir.clone(),
        };
        let mut context = ctx(1, &[labels::REFINEMENT]);
        write.run(&mut context).await.expect("written");
        let writes = gh.writes();
        assert!(
            writes
                .iter()
                .any(|w| matches!(w, Wrote::Body(25, b) if b.contains("the lot goal")))
        );
        assert!(writes.contains(&Wrote::Unlabelled(25, labels::REFINEMENT.to_string())));
        assert!(!writes.contains(&Wrote::Label(25, labels::SPEC_WRITTEN.to_string())));
        let _ = std::fs::remove_dir_all(&review_dir);
    }

    #[tokio::test]
    async fn round_two_marks_spec_written_for_an_agent_task() {
        let review_dir = dir("round-two");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            refinement_dir: review_dir.clone(),
        };
        let mut context = ctx(2, &[]);
        write.run(&mut context).await.expect("written");
        assert!(
            gh.writes()
                .contains(&Wrote::Label(25, labels::SPEC_WRITTEN.to_string()))
        );
        let _ = std::fs::remove_dir_all(&review_dir);
    }

    #[tokio::test]
    async fn a_human_task_only_needs_round_one_to_be_spec_written() {
        let review_dir = dir("human-task");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            refinement_dir: review_dir.clone(),
        };
        let mut context = ctx(1, &[labels::HUMAN]);
        write.run(&mut context).await.expect("written");
        assert!(
            gh.writes()
                .contains(&Wrote::Label(25, labels::SPEC_WRITTEN.to_string()))
        );
        let _ = std::fs::remove_dir_all(&review_dir);
    }

    #[tokio::test]
    async fn the_round_counter_comment_is_posted_last() {
        let review_dir = dir("counter-last");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            refinement_dir: review_dir.clone(),
        };
        let mut context = ctx(1, &[]);
        write.run(&mut context).await.expect("written");
        let writes = gh.writes();
        let comment = writes
            .iter()
            .position(|w| matches!(w, Wrote::Comment(25, b) if b == "refinement round: 1"));
        assert!(comment.is_some());
        assert_eq!(comment.unwrap(), writes.len() - 1, "posted last");
        let _ = std::fs::remove_dir_all(&review_dir);
    }

    #[tokio::test]
    async fn a_coherence_pass_that_drops_a_section_is_ignored_rather_than_applied() {
        let review_dir = dir("coherence-drops");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            refinement_dir: review_dir.clone(),
        };
        let mut context = ctx(1, &[]);
        context.results.insert(
            COHERENCE.to_string(),
            Reply {
                // Doesn't keep "## Business Goal": broken output.
                text: "nothing recognizable".to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        write.run(&mut context).await.expect("published anyway");
        let body = std::fs::read_to_string(review_dir.join("25-r01-body.md")).expect("written");
        assert!(body.contains("the lot goal"), "original merge held");
        let _ = std::fs::remove_dir_all(&review_dir);
    }
}
