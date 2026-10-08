//! The issue body: assembled, saved to disk, then posted.
//!
//! Written before posting: if `gh` fails, the text still exists and the error
//! message says where — paid sections don't vanish because a network call failed.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;

use crate::common::labels;
use crate::refinement::data::state::RefinementState;
use crate::refinement::data::{advice, rounds, sections};

/// The publishing step: body, comment, labels.
pub struct Write {
    /// What rewrites the body and labels.
    pub gh: Rc<dyn GitHub>,
    /// The coherence stage name, where its output is stored in
    /// `ctx.results`. Received from the table, like every stage name.
    pub coherence: String,
    /// The advice stage name, when this phase has one (business only): its
    /// reply is posted as a comment before the round counter.
    pub advice: Option<String>,
    /// The artifacts folder for this issue.
    pub refinement_dir: PathBuf,
    /// What keeps the body on disk before it is posted.
    pub disk: Rc<dyn Disk>,
}

impl Write {
    /// Poses or clears `harness:needs-decision`, to match what the advice said.
    ///
    /// **Never fails the round.** By the time this runs, every session is paid
    /// for, the body is posted and the advice comment carries the same answer in
    /// words. A label is how the board is read at a glance, not where the
    /// information lives — so a `gh` refusal is said in the journal and the round
    /// still completes. The one setup it cannot survive is the label not existing
    /// on the repository, which is why it is in
    /// [`labels::ALL`](crate::common::labels::ALL) for `init-repo` to create.
    ///
    /// Both directions, because the flag is derived from the latest round: a
    /// re-run that now concludes `no` has to take it off, or the board keeps
    /// claiming a blocker that has been answered.
    async fn flag_decision(
        &self,
        num: u64,
        issue: &harness_core::domain::Issue,
        found: &advice::Read,
        ctx: &Context<RefinementState>,
    ) {
        let carried = issue.has(labels::NEEDS_DECISION);
        let wanted = found.wants_a_human();
        let done = if wanted && !carried {
            self.gh.add_label(num, labels::NEEDS_DECISION).await
        } else if !wanted && carried {
            self.gh.remove_label(num, labels::NEEDS_DECISION).await
        } else {
            return;
        };
        match done {
            Ok(()) if wanted => ctx.traces.say(&format!(
                "#{num} — {} posed: a human has a decision to make",
                labels::NEEDS_DECISION
            )),
            Ok(()) => ctx.traces.say(&format!(
                "#{num} — {} removed: this round needs no decision",
                labels::NEEDS_DECISION
            )),
            Err(why) => ctx.traces.warn(&format!(
                "#{num} — could not {} {}: {why}. The advice comment says it \
                 anyway; the round is published",
                if wanted { "pose" } else { "remove" },
                labels::NEEDS_DECISION
            )),
        }
    }
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
        // What the canonical sections do not cover stays: a session's
        // `## Assumptions (autonomous run)`, a human's `## Owner Decisions`.
        // A round rewrites the body; it must not erase what it never read.
        let extras = sections::unknown(&issue.body);
        let body = if extras.is_empty() {
            body
        } else {
            format!("{body}\n{extras}\n")
        };

        let path = self
            .refinement_dir
            .join(format!("{num}-r{:02}-body.md", ctx.state.round_no));
        if let Some(parent) = path.parent() {
            self.disk.create_dir_all(parent)?;
        }
        self.disk.write_to_string(&path, &body)?;

        self.gh.set_body(num, &body).await?;

        let phase = ctx.state.phase;

        // Only if it was there: `gh` returns 404 removing a missing label,
        // which is true for every run with `--force`.
        if issue.has(phase.requested_by()) {
            self.gh.remove_label(num, phase.requested_by()).await?;
        }

        // One request writes the whole phase: the label it leaves says so.
        self.gh.add_label(num, phase.leaves()).await?;

        if let Some(reply) = self
            .advice
            .as_ref()
            .and_then(|stage| ctx.results.get(stage))
        {
            let text = reply.text.trim();
            if !text.is_empty() {
                let found = advice::read(text);
                // The headline goes in the journal too: the whole point of the
                // score is to be read without opening the issue.
                ctx.traces
                    .say(&format!("#{num} — {}", advice::headline(&found)));
                self.gh
                    .post_issue_comment(num, &advice::comment(num, text))
                    .await?;
                self.flag_decision(num, &issue, &found, ctx).await;
            }
        }

        // Counter last: it's what says this round happened. Posted before a
        // failing label, it would restart resumption at the next round.
        self.gh
            .post_issue_comment(num, &rounds::comment(ctx.state.round_no, phase))
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
    use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::refinement::data::phase::Phase;
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    /// The name the table gives to the coherence stage.
    const COHERENCE: &str = "coherence";
    const ADVICE: &str = "human-advice";

    /// Where the body lands on the fake disk — never a real folder.
    fn dir(name: &str) -> PathBuf {
        PathBuf::from(format!("/refinement-{name}"))
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
            phase: Phase::Business,
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
    async fn round_one_writes_the_body_and_marks_spec_written() {
        let review_dir = dir("round-one");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            advice: Some(ADVICE.to_string()),
            refinement_dir: review_dir.clone(),
            disk: Rc::new(FakeDisk::default()),
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
        assert!(writes.contains(&Wrote::Label(25, labels::SPEC_WRITTEN.to_string())));
    }

    #[tokio::test]
    async fn round_two_marks_spec_written_for_an_agent_task() {
        let review_dir = dir("round-two");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            advice: Some(ADVICE.to_string()),
            refinement_dir: review_dir.clone(),
            disk: Rc::new(FakeDisk::default()),
        };
        let mut context = ctx(2, &[]);
        write.run(&mut context).await.expect("written");
        assert!(
            gh.writes()
                .contains(&Wrote::Label(25, labels::SPEC_WRITTEN.to_string()))
        );
    }

    #[tokio::test]
    async fn a_human_task_only_needs_round_one_to_be_spec_written() {
        let review_dir = dir("human-task");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            advice: Some(ADVICE.to_string()),
            refinement_dir: review_dir.clone(),
            disk: Rc::new(FakeDisk::default()),
        };
        let mut context = ctx(1, &[labels::HUMAN]);
        write.run(&mut context).await.expect("written");
        assert!(
            gh.writes()
                .contains(&Wrote::Label(25, labels::SPEC_WRITTEN.to_string()))
        );
    }

    #[tokio::test]
    async fn the_advice_is_posted_as_a_comment_before_the_counter() {
        let review_dir = dir("advice");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            advice: Some(ADVICE.to_string()),
            refinement_dir: review_dir.clone(),
            disk: Rc::new(FakeDisk::default()),
        };
        let mut context = ctx(1, &[]);
        context.results.insert(
            ADVICE.to_string(),
            Reply {
                text: "technical-refinement: 5/5\nhuman-in-the-loop: yes\n- a paid service"
                    .to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        write.run(&mut context).await.expect("written");
        let writes = gh.writes();
        let advice = writes
            .iter()
            .position(|w| matches!(w, Wrote::Comment(25, b) if b.contains("necessity 5/5")))
            .expect("advice posted");
        // The relation, not a position: the decision label now lands between
        // the two, and an index would break on every write added here while
        // saying nothing about the invariant, which is the order.
        let counter = writes
            .iter()
            .position(|w| matches!(w, Wrote::Comment(25, b) if b == "refinement round: 1"))
            .expect("the counter");
        assert!(advice < counter, "the advice comes before the counter");
        // The score is the headline, the advice is kept whole under it, and the
        // gesture a `yes` implies is spelled out.
        let Some(Wrote::Comment(_, body)) = writes.get(advice) else {
            panic!("a comment");
        };
        assert!(
            body.starts_with("Technical refinement — necessity 5/5"),
            "{body}"
        );
        assert!(body.contains("a paid service"));
        assert!(body.contains(labels::TECH_REFINEMENT));
    }

    /// A business round whose advice says `text`, on an issue carrying `labels`.
    async fn advised(name: &str, text: &str, issue_labels: &[&str]) -> Vec<Wrote> {
        let review_dir = dir(name);
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            advice: Some(ADVICE.to_string()),
            refinement_dir: review_dir.clone(),
            disk: Rc::new(FakeDisk::default()),
        };
        let mut context = ctx(1, issue_labels);
        context.results.insert(
            ADVICE.to_string(),
            Reply {
                text: text.to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        write.run(&mut context).await.expect("written");
        gh.writes()
    }

    #[tokio::test]
    async fn an_advice_that_wants_a_human_poses_the_decision_label() {
        let writes = advised(
            "needs-decision",
            "technical-refinement: 4/5\nhuman-in-the-loop: yes\n\n- Q1: which port?",
            &[],
        )
        .await;
        assert!(writes.contains(&Wrote::Label(25, labels::NEEDS_DECISION.to_string())));
    }

    #[tokio::test]
    async fn an_advice_that_wants_no_human_poses_nothing() {
        let writes = advised(
            "no-decision",
            "technical-refinement: 2/5\nhuman-in-the-loop: no\n\n- mechanical",
            &[],
        )
        .await;
        assert!(!writes.contains(&Wrote::Label(25, labels::NEEDS_DECISION.to_string())));
        // And nothing is removed either: it was not there to begin with, and
        // `gh` answers 404 on a label an issue does not carry.
        assert!(!writes.contains(&Wrote::Unlabelled(25, labels::NEEDS_DECISION.to_string())));
    }

    #[tokio::test]
    async fn a_later_round_that_needs_no_decision_clears_the_label() {
        // Derived state: leaving it would keep the board claiming a blocker
        // that this round just concluded is gone.
        let writes = advised(
            "decision-cleared",
            "technical-refinement: 1/5\nhuman-in-the-loop: no",
            &[labels::NEEDS_DECISION],
        )
        .await;
        assert!(writes.contains(&Wrote::Unlabelled(25, labels::NEEDS_DECISION.to_string())));
    }

    #[tokio::test]
    async fn a_label_already_there_is_not_posed_twice() {
        let writes = advised(
            "decision-kept",
            "technical-refinement: 5/5\nhuman-in-the-loop: yes",
            &[labels::NEEDS_DECISION],
        )
        .await;
        assert!(!writes.contains(&Wrote::Label(25, labels::NEEDS_DECISION.to_string())));
    }

    #[tokio::test]
    async fn an_unreadable_advice_still_asks_for_a_human() {
        // Nothing was understood, so nobody can say this is safe unattended —
        // and the one who reads an unreadable answer has to be a human.
        let writes = advised("decision-unstated", "a paragraph with no shape", &[]).await;
        assert!(writes.contains(&Wrote::Label(25, labels::NEEDS_DECISION.to_string())));
    }

    #[tokio::test]
    async fn the_round_survives_a_decision_label_gh_refuses() {
        // Every session is paid by now and the advice comment carries the same
        // answer in words. A board flag is not worth losing the round over.
        let review_dir = dir("decision-refused");
        let gh = Rc::new(FakeGitHub::default());
        gh.refuse_label(labels::NEEDS_DECISION);
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            advice: Some(ADVICE.to_string()),
            refinement_dir: review_dir.clone(),
            disk: Rc::new(FakeDisk::default()),
        };
        let mut context = ctx(1, &[]);
        context.results.insert(
            ADVICE.to_string(),
            Reply {
                text: "technical-refinement: 4/5\nhuman-in-the-loop: yes".to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        write.run(&mut context).await.expect("published anyway");
        // And the counter still landed: the round really did complete.
        assert!(
            gh.writes()
                .contains(&Wrote::Comment(25, "refinement round: 1".to_string()))
        );
    }

    #[tokio::test]
    async fn a_technical_round_leaves_tech_written_and_its_own_counter() {
        let review_dir = dir("technical");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            advice: None,
            refinement_dir: review_dir.clone(),
            disk: Rc::new(FakeDisk::default()),
        };
        let mut context = ctx(1, &[labels::TECH_REFINEMENT]);
        context.state.phase = Phase::Technical;
        write.run(&mut context).await.expect("written");
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::Unlabelled(25, labels::TECH_REFINEMENT.to_string())));
        assert!(writes.contains(&Wrote::Label(25, labels::TECH_WRITTEN.to_string())));
        assert!(!writes.contains(&Wrote::Label(25, labels::SPEC_WRITTEN.to_string())));
        assert!(writes.contains(&Wrote::Comment(
            25,
            "technical refinement round: 1".to_string()
        )));
    }

    #[tokio::test]
    async fn the_round_counter_comment_is_posted_last() {
        let review_dir = dir("counter-last");
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            advice: Some(ADVICE.to_string()),
            refinement_dir: review_dir.clone(),
            disk: Rc::new(FakeDisk::default()),
        };
        let mut context = ctx(1, &[]);
        write.run(&mut context).await.expect("written");
        let writes = gh.writes();
        let comment = writes
            .iter()
            .position(|w| matches!(w, Wrote::Comment(25, b) if b == "refinement round: 1"));
        assert!(comment.is_some());
        assert_eq!(comment.unwrap(), writes.len() - 1, "posted last");
    }

    #[tokio::test]
    async fn a_coherence_pass_that_drops_a_section_is_ignored_rather_than_applied() {
        let review_dir = dir("coherence-drops");
        let gh = Rc::new(FakeGitHub::default());
        let disk = Rc::new(FakeDisk::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            advice: Some(ADVICE.to_string()),
            refinement_dir: review_dir.clone(),
            disk: Rc::clone(&disk) as Rc<dyn Disk>,
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
        let body = disk
            .written_to(&review_dir.join("25-r01-body.md"))
            .expect("written");
        assert!(body.contains("the lot goal"), "original merge held");
    }

    #[tokio::test]
    async fn a_section_under_a_heading_no_stage_owns_survives_the_round() {
        // The loss this prevents: `## Assumptions (autonomous run)` written by
        // the dev loop vanished the next time a refinement rewrote the body.
        let gh = Rc::new(FakeGitHub::default());
        let write = Write {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            coherence: COHERENCE.to_string(),
            advice: None,
            refinement_dir: dir("extras"),
            disk: Rc::new(FakeDisk::default()),
        };
        let mut context = ctx(1, &[]);
        if let Some(issue) = context.state.issue.as_mut() {
            issue.body = "## Business Goal\n\nold goal\n\n\
                          ## Assumptions (autonomous run)\n\n- the API is v2\n"
                .to_string();
        }
        write.run(&mut context).await.expect("written");
        let body = gh
            .writes()
            .into_iter()
            .find_map(|w| match w {
                Wrote::Body(25, b) => Some(b),
                _ => None,
            })
            .expect("a body");
        assert!(
            body.starts_with("## Business Goal\n\nthe lot goal"),
            "{body}"
        );
        assert!(
            body.contains("## Assumptions (autonomous run)\n\n- the API is v2"),
            "{body}"
        );
    }
}
