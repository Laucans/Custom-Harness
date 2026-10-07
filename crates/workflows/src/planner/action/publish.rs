//! The deterministic write: the milestones opened from the parsed plan.
//!
//! The session decided *what* the milestones are — this is the only place
//! that decides *how* they land on GitHub: created, linked to the roadmap
//! item, chained by `blocked_by`, labelled for refinement.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;

use crate::common::labels;
use crate::planner::data::plan;
use crate::planner::data::state::PlannerState;

/// Opens one milestone per item of the parsed plan, in order.
pub struct Write {
    /// What creates the milestones, links them, and labels them.
    pub gh: Rc<dyn GitHub>,
    /// The stage whose reply carries the plan.
    pub plan_stage: String,
}

#[async_trait(?Send)]
impl Action<PlannerState> for Write {
    async fn run(&self, ctx: &mut Context<PlannerState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some(reply) = ctx.results.get(&self.plan_stage) else {
            return Err(Halt::Failed(
                "no plan reply to publish from — the plan stage must run first".to_string(),
            ));
        };
        // The post-gate on the plan stage (`checks::gates::PlanParses`)
        // already demanded this parses; a failure here would be that gate
        // not having run, not a reason to improvise.
        let items = plan::parse(&reply.text).map_err(|e| {
            Halt::Failed(format!(
                "the plan still did not parse ({e}) — PlanParses should have \
                 stopped the round before this stage ever ran"
            ))
        })?;
        if items.is_empty() {
            ctx.traces
                .say("the plan named no further milestone for this roadmap item");
            return Ok(Verdict::Continue);
        }

        let roadmap_number = ctx.state.roadmap().number;
        let mut previous = ctx.state.existing.last().map(|issue| issue.number);
        for item in &items {
            let number = self
                .gh
                .create_issue(&item.title, &item.goal, &[labels::MILESTONE])
                .await?;
            self.gh
                .create_sub_issue_link(roadmap_number, number)
                .await?;
            if let Some(blocker) = previous {
                self.gh.add_blocked_by(number, blocker).await?;
            }
            self.gh.add_label(number, labels::REFINEMENT).await?;
            ctx.traces
                .say(&format!("opened milestone #{number}: {}", item.title));
            previous = Some(number);
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool) -> Context<PlannerState> {
        let mut ctx = Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            PlannerState {
                roadmap: Some(Issue {
                    number: 4,
                    ..Issue::default()
                }),
                ..PlannerState::default()
            },
            Logbook::null(),
        );
        ctx.results.insert(
            "plan".to_string(),
            Reply {
                text: r#"[{"title":"A","goal":"do A"},{"title":"B","goal":"do B"}]"#.to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        ctx
    }

    #[tokio::test]
    async fn each_milestone_is_created_linked_and_chained_in_order() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false);
        Write {
            gh: gh.clone(),
            plan_stage: "plan".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        let writes = gh.writes();
        assert_eq!(
            writes,
            vec![
                Wrote::CreatedIssue(
                    "A".to_string(),
                    "do A".to_string(),
                    vec![labels::MILESTONE.to_string()]
                ),
                Wrote::SubIssueLink(4, 1),
                Wrote::Label(1, labels::REFINEMENT.to_string()),
                Wrote::CreatedIssue(
                    "B".to_string(),
                    "do B".to_string(),
                    vec![labels::MILESTONE.to_string()]
                ),
                Wrote::SubIssueLink(4, 2),
                Wrote::BlockedByLink(2, 1),
                Wrote::Label(2, labels::REFINEMENT.to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(true);
        Write {
            gh: gh.clone(),
            plan_stage: "plan".to_string(),
        }
        .run(&mut context)
        .await
        .expect("nothing to do");
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn the_first_new_milestone_chains_onto_the_last_existing_one() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false);
        context.state.existing = vec![Issue {
            number: 9,
            ..Issue::default()
        }];
        Write {
            gh: gh.clone(),
            plan_stage: "plan".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        assert!(
            gh.writes()
                .iter()
                .any(|w| matches!(w, Wrote::BlockedByLink(_, 9)))
        );
    }

    #[tokio::test]
    async fn an_empty_plan_writes_nothing() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false);
        context.results.insert(
            "plan".to_string(),
            Reply {
                text: "[]".to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        Write {
            gh: gh.clone(),
            plan_stage: "plan".to_string(),
        }
        .run(&mut context)
        .await
        .expect("nothing to do");
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn an_unparseable_reply_is_a_failure_not_an_improvisation() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false);
        context.results.insert(
            "plan".to_string(),
            Reply {
                text: "not json".to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        let err = Write {
            gh,
            plan_stage: "plan".to_string(),
        }
        .run(&mut context)
        .await
        .expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }
}
