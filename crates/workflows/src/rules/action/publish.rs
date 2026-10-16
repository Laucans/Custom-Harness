//! What the rules' task writes on GitHub itself: the request it was asked
//! by, taken, and why it cannot answer one it cannot read.
//!
//! **Taken before the sessions are paid for, not after they succeed** — the
//! reasoning of the test debt's own `ConsumeRequest`, and more so here: the
//! route outranks the dev loop, so a request left in place after a failed
//! attempt would be asked again every poll. The issue stays open and
//! labelled `harness:rules` — development still waits on it — until a
//! human, or the attempt itself, settles it.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;

use crate::common::labels;
use crate::rules::data::state::RulesState;

/// Removes `harness:ready` from the issue: the request has been taken. An
/// issue whose request cannot be read is told why, and waits on a human.
pub struct ConsumeRequest {
    /// What removes the label and comments.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<RulesState> for ConsumeRequest {
    async fn run(&self, ctx: &mut Context<RulesState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let number = ctx.state.issue().number;
        self.gh.remove_label(number, labels::READY).await?;
        ctx.traces.say(&format!(
            "#{number}: {} consumed — one attempt follows",
            labels::READY
        ));
        if let Some(why) = ctx.state.unreadable.clone() {
            self.gh
                .post_issue_comment(
                    number,
                    &format!(
                        "The rules' task cannot read what this issue asks: {why}.\n\n\
                         Fix the body, then put `{}` back.",
                        labels::READY
                    ),
                )
                .await?;
            self.gh.add_label(number, labels::HUMAN).await?;
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::rules::data::request::Ask;
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool, unreadable: Option<&str>) -> Context<RulesState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            RulesState {
                issue: Some(Issue {
                    number: 7,
                    ..Issue::default()
                }),
                ask: unreadable.is_none().then(|| Ask::Contest {
                    said: String::new(),
                }),
                unreadable: unreadable.map(ToString::to_string),
                comments: String::new(),
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn the_request_is_taken_before_anything_is_paid() {
        let gh = Rc::new(FakeGitHub::default());
        ConsumeRequest {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut ctx(false, None))
        .await
        .expect("consumed");
        assert_eq!(
            gh.writes(),
            [Wrote::Unlabelled(7, labels::READY.to_string())]
        );
    }

    #[tokio::test]
    async fn an_unreadable_request_is_taken_said_and_handed_to_a_human() {
        let gh = Rc::new(FakeGitHub::default());
        ConsumeRequest {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut ctx(false, Some("no json block")))
        .await
        .expect("consumed");
        let writes = gh.writes();
        assert_eq!(writes.len(), 3);
        assert!(matches!(&writes[1], Wrote::Comment(7, said) if said.contains("no json block")));
        assert_eq!(writes[2], Wrote::Label(7, labels::HUMAN.to_string()));
    }

    #[tokio::test]
    async fn a_dry_run_takes_nothing() {
        let gh = Rc::new(FakeGitHub::default());
        ConsumeRequest {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut ctx(true, Some("x")))
        .await
        .expect("nothing");
        let seen = &gh.writes();
        assert!(seen.is_empty(), "{seen:?}");
    }
}
