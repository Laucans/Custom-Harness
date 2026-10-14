//! The deterministic write: the milestones opened from the parsed plan.
//!
//! The session decided *what* the milestones are — this is the only place
//! that decides *how* they land on GitHub: created, linked to the roadmap
//! item, chained by `blocked_by` on every milestone that touches one of
//! its aggregates — already open or opened just before — so two
//! milestones on different aggregates open together and a milestone with
//! none waits on nothing; `harness:ready` at once, so `split` takes each
//! when its blockers are delivered — a human holds one by taking the label
//! off. The side of the
//! architecture is stated in the body's `## Architecture` only: no label
//! carries it at this level — `split` decides it task by task, and a
//! milestone label would contradict the tasks it holds.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Event;

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
        // Who touches which aggregate: the milestones already open, by their
        // body, then those opened here, in order. A new milestone waits on
        // every one before it that shares an aggregate with it.
        let mut touched: Vec<(u64, Vec<String>)> = ctx
            .state
            .existing
            .iter()
            .map(|issue| (issue.number, plan::aggregates_of(&issue.body)))
            .collect();
        let mut opened: Vec<u64> = Vec::new();
        for item in &items {
            let number = self
                .gh
                .create_issue(&item.title, &item.body(), &[labels::MILESTONE])
                .await?;
            self.gh
                .create_sub_issue_link(roadmap_number, number)
                .await?;
            let mine = item.aggregate_keys();
            let blockers: Vec<u64> = touched
                .iter()
                .filter(|(_, theirs)| theirs.iter().any(|a| mine.contains(a)))
                .map(|(n, _)| *n)
                .collect();
            for blocker in &blockers {
                self.gh.add_blocked_by(number, *blocker).await?;
            }
            // The tap opened: `split` takes it once its blockers are delivered,
            // unless a human takes the label off to hold it.
            self.gh.add_label(number, labels::READY).await?;
            ctx.traces.say(&format!(
                "opened milestone #{number}: {} ({})",
                item.title,
                if blockers.is_empty() {
                    "waits on nothing".to_string()
                } else {
                    format!(
                        "after {}, on {}",
                        blockers
                            .iter()
                            .map(|b| format!("#{b}"))
                            .collect::<Vec<_>>()
                            .join(", "),
                        mine.join(", ")
                    )
                }
            ));
            touched.push((number, mine));
            opened.push(number);
        }
        ctx.traces.event(&Event::MilestonesPlanned {
            roadmap: roadmap_number,
            milestones: opened,
        });
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
                text: r#"[{"title":"A","goal":"do A","aggregates":["PJ"]},{"title":"B","goal":"do B","aggregates":["Campagne"]},{"title":"C","goal":"do C","aggregates":["pj","Campagne"]},{"title":"D","goal":"do D"}]"#.to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        ctx
    }

    #[tokio::test]
    async fn milestones_on_different_aggregates_open_together_and_one_on_both_waits_on_both() {
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
        // A (PJ) and B (Campagne) share nothing: both wait on nothing. C
        // touches both, whatever the case: it waits on A and B. D names no
        // aggregate: it waits on nothing.
        assert_eq!(
            writes,
            vec![
                Wrote::CreatedIssue(
                    "A".to_string(),
                    "do A\n\n## Architecture\n\nsystems: -\nconcepts: -\naggregates: PJ\nside: harness:read-side"
                        .to_string(),
                    vec![labels::MILESTONE.to_string()]
                ),
                Wrote::SubIssueLink(4, 1),
                Wrote::Label(1, labels::READY.to_string()),
                Wrote::CreatedIssue(
                    "B".to_string(),
                    "do B\n\n## Architecture\n\nsystems: -\nconcepts: -\naggregates: Campagne\nside: harness:read-side"
                        .to_string(),
                    vec![labels::MILESTONE.to_string()]
                ),
                Wrote::SubIssueLink(4, 2),
                Wrote::Label(2, labels::READY.to_string()),
                Wrote::CreatedIssue(
                    "C".to_string(),
                    "do C\n\n## Architecture\n\nsystems: -\nconcepts: -\naggregates: pj, Campagne\nside: harness:read-side"
                        .to_string(),
                    vec![labels::MILESTONE.to_string()]
                ),
                Wrote::SubIssueLink(4, 3),
                Wrote::BlockedByLink(3, 1),
                Wrote::BlockedByLink(3, 2),
                Wrote::Label(3, labels::READY.to_string()),
                Wrote::CreatedIssue(
                    "D".to_string(),
                    "do D\n\n## Architecture\n\nsystems: -\nconcepts: -\naggregates: -\nside: harness:read-side"
                        .to_string(),
                    vec![labels::MILESTONE.to_string()]
                ),
                Wrote::SubIssueLink(4, 4),
                Wrote::Label(4, labels::READY.to_string()),
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
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
    }

    #[tokio::test]
    async fn a_new_milestone_waits_on_an_open_one_only_when_they_share_an_aggregate() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false);
        context.state.existing = vec![
            Issue {
                number: 8,
                body: "old\n\n## Architecture\n\nsystems: -\nconcepts: -\naggregates: Campagne\nside: harness:write-side".to_string(),
                ..Issue::default()
            },
            Issue {
                number: 9,
                body: "older, on nothing".to_string(),
                ..Issue::default()
            },
        ];
        Write {
            gh: gh.clone(),
            plan_stage: "plan".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        let links: Vec<(u64, u64)> = gh
            .writes()
            .into_iter()
            .filter_map(|w| match w {
                Wrote::BlockedByLink(task, dependency) => Some((task, dependency)),
                _ => None,
            })
            .collect();
        // A (PJ) waits on nothing already open; B (Campagne) on #8; C (both) on #8, A and B; D on nothing.
        assert_eq!(links, vec![(2, 8), (3, 8), (3, 1), (3, 2)]);
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
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
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
