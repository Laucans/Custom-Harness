//! What the re-study's session must leave: a reply that reads, and its
//! checkout as it found it.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};
use harness_core::ports::shell::git::Repo;

use crate::restudy::data::reply;
use crate::restudy::data::state::StudyState;

/// The session's reply ends on its two lines, read.
pub struct StudyParses {
    /// The stage whose reply this judges.
    pub stage: String,
}

#[async_trait(?Send)]
impl Verification<StudyState> for StudyParses {
    fn purpose(&self) -> String {
        "the reply ends on `RESTUDY: tasks [...]` that reads, and its contests".to_string()
    }

    async fn verify(&self, ctx: &Context<StudyState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some(answer) = ctx.results.get(&self.stage) else {
            return Err(Halt::Failed(format!(
                "no reply recorded under {} — the study stage did not run",
                self.stage
            )));
        };
        reply::parse(&answer.text)
            .map(|_| Verdict::Continue)
            .map_err(|why| {
                Halt::Failed(format!(
                    "the re-study did not read, even after one retry: {why}"
                ))
            })
    }
}

/// The checkout is on the commit it was mounted on.
///
/// A read-only session committed nothing — so it pushed nothing either.
/// The milestone's branch on GitHub may move meanwhile (a task merged into
/// it); the checkout does not.
pub struct TheCheckoutIsUntouched {
    /// The re-study's checkout.
    pub git: Rc<dyn Repo>,
}

#[async_trait(?Send)]
impl Verification<StudyState> for TheCheckoutIsUntouched {
    fn purpose(&self) -> String {
        "the checkout is still on the commit it was mounted on: nothing committed".to_string()
    }

    async fn verify(&self, ctx: &Context<StudyState>) -> Outcome<Verdict> {
        let Some(before) = ctx.state.head.as_deref().filter(|_| !ctx.settings.dry_run) else {
            return Ok(Verdict::Continue);
        };
        let after = self.git.head_sha().await?;
        if after.trim() == before.trim() {
            Ok(Verdict::Continue)
        } else {
            Err(Halt::Failed(format!(
                "the checkout moved from {} to {} during the re-study: it committed, and a \
                 re-study is read-only — nothing it found is opened",
                before.trim(),
                after.trim()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_git::FakeRepos;
    use harness_core::domain::Spend;
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::ports::shell::git::Repos;
    use harness_core::traces::Logbook;

    fn ctx(answer: Option<&str>, head: Option<&str>, dry_run: bool) -> Context<StudyState> {
        let mut ctx = Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            StudyState {
                head: head.map(ToString::to_string),
                ..StudyState::default()
            },
            Logbook::null(),
        );
        if let Some(text) = answer {
            ctx.results.insert(
                "study".to_string(),
                Reply {
                    text: text.to_string(),
                    stop_line: None,
                    spend: Spend::default(),
                },
            );
        }
        ctx
    }

    #[tokio::test]
    async fn a_reply_that_is_no_re_study_stops_the_round() {
        let gate = StudyParses {
            stage: "study".to_string(),
        };
        assert!(gate.purpose().contains("RESTUDY: tasks"));
        let good = ctx(Some("RESTUDY: tasks []"), None, false);
        assert_eq!(gate.verify(&good).await.expect("reads"), Verdict::Continue);
        let prose = ctx(Some("I found three gaps"), None, false);
        assert!(
            gate.verify(&prose)
                .await
                .expect_err("prose")
                .reason()
                .contains("even after one retry")
        );
        assert!(
            gate.verify(&ctx(None, None, false)).await.is_err(),
            "no reply"
        );
        assert_eq!(
            gate.verify(&ctx(None, None, true)).await.expect("dry"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_checkout_that_moved_fails_and_one_that_did_not_passes() {
        let at = |heads: &[&str]| TheCheckoutIsUntouched {
            git: FakeRepos::default()
                .with_heads(heads)
                .at(std::path::Path::new("/w")),
        };
        assert!(at(&[]).purpose().contains("nothing committed"));
        assert_eq!(
            at(&["f00d\n"])
                .verify(&ctx(None, Some("f00d"), false))
                .await
                .expect("untouched"),
            Verdict::Continue
        );
        let moved = at(&["beef"])
            .verify(&ctx(None, Some("f00d"), false))
            .await
            .expect_err("moved");
        assert!(
            moved.reason().contains("moved from f00d to beef"),
            "{}",
            moved.reason()
        );
        // No head read (a dry run): nothing to compare.
        assert_eq!(
            at(&["beef"])
                .verify(&ctx(None, None, false))
                .await
                .expect("none"),
            Verdict::Continue
        );
        assert_eq!(
            at(&["beef"])
                .verify(&ctx(None, Some("f00d"), true))
                .await
                .expect("dry"),
            Verdict::Continue
        );
    }
}
