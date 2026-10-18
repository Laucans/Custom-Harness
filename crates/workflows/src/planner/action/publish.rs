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

use harness_architecture::graph;

use crate::common::labels;
use crate::planner::data::plan;
use crate::planner::data::state::PlannerState;

/// Opens one milestone per item of the parsed plan, in order.
pub struct Write {
    /// What creates the milestones, links them, and labels them.
    pub gh: Rc<dyn GitHub>,
    /// The stage whose reply carries the plan.
    pub plan_stage: String,
    /// The integration branch the plan's graph is written on.
    pub branch: String,
}

impl Write {
    /// Writes the plan's graph (contract R) from every milestone of the
    /// roadmap item — open before, opened here — and says what it found: a
    /// read of a later milestone, an id nobody produces. Never fails the
    /// publish: the milestones are the deliverable, the graph their trace.
    async fn write_graph(
        &self,
        ctx: &Context<PlannerState>,
        opened: &[(u64, &plan::MilestoneItem)],
    ) {
        let mut milestones: Vec<graph::Milestone> = ctx
            .state
            .existing
            .iter()
            .map(|issue| {
                plan::graph_input(
                    issue.number,
                    &issue.title,
                    &issue.body,
                    &ctx.state.components,
                    &ctx.state.writes,
                )
            })
            .collect();
        for (number, item) in opened {
            milestones.push(plan::graph_input(
                *number,
                &item.title,
                &item.body(),
                &ctx.state.components,
                &ctx.state.writes,
            ));
        }
        let found = graph::Graph::derive(&milestones, &ctx.state.components);
        for ahead in found.reads_ahead() {
            ctx.traces.warn(&format!(
                "plan graph: #{} reads `{}`, which #{} produces later — built in this order, its tasks find it missing",
                ahead.consumer, ahead.id, ahead.producer
            ));
        }
        let unproduced = found.unproduced();
        if !unproduced.is_empty() {
            ctx.traces.say(&format!(
                "plan graph: no milestone produces {} — already in the repository, or left out of the plan",
                unproduced.join(", ")
            ));
        }
        let Ok(text) = found.to_json() else {
            return;
        };
        match self
            .gh
            .put_file(
                graph::PATH,
                &self.branch,
                &text,
                "chore(plan): the graph of the milestones (contract R)",
            )
            .await
        {
            Ok(()) => ctx.traces.say(&format!(
                "plan graph: {} milestone(s), {} id(s) — written to {} on {}",
                found.milestones.len(),
                found.ids.len(),
                graph::PATH,
                self.branch
            )),
            Err(halt) => ctx.traces.warn(&format!(
                "plan graph: not written to {} — {}",
                graph::PATH,
                halt.reason()
            )),
        }
    }
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
        let mut placed: Vec<(u64, &plan::MilestoneItem)> = Vec::new();
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
            placed.push((number, item));
        }
        self.write_graph(ctx, &placed).await;
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
            branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        let mut writes = gh.writes();
        // Last, the plan's graph, on the integration branch: A produces PJ,
        // C consumes it and depends on A and B.
        let Some(Wrote::PutFile(path, branch, text)) = writes.pop() else {
            panic!("the graph is written last: {writes:?}");
        };
        assert_eq!(
            (path.as_str(), branch.as_str()),
            (graph::PATH, "main_agent")
        );
        let written = graph::Graph::parse(&text).expect("a graph");
        assert_eq!(
            written
                .milestones
                .iter()
                .map(|m| m.number)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        assert_eq!(written.ids["PJ"].producer, Some(1));
        assert_eq!(written.milestones[2].depends_on, vec![1, 2]);
        // A (PJ) and B (Campagne) share nothing: both wait on nothing. C
        // touches both, whatever the case: it waits on A and B. D names no
        // aggregate: it waits on nothing.
        assert_eq!(
            writes,
            vec![
                Wrote::CreatedIssue(
                    "A".to_string(),
                    "do A\n\n## Architecture\n\nsystems: -\nconcepts: -\naggregates: PJ\nmicro-uis: -\ncommands: -\nside: harness:read-side"
                        .to_string(),
                    vec![labels::MILESTONE.to_string()]
                ),
                Wrote::SubIssueLink(4, 1),
                Wrote::Label(1, labels::READY.to_string()),
                Wrote::CreatedIssue(
                    "B".to_string(),
                    "do B\n\n## Architecture\n\nsystems: -\nconcepts: -\naggregates: Campagne\nmicro-uis: -\ncommands: -\nside: harness:read-side"
                        .to_string(),
                    vec![labels::MILESTONE.to_string()]
                ),
                Wrote::SubIssueLink(4, 2),
                Wrote::Label(2, labels::READY.to_string()),
                Wrote::CreatedIssue(
                    "C".to_string(),
                    "do C\n\n## Architecture\n\nsystems: -\nconcepts: -\naggregates: pj, Campagne\nmicro-uis: -\ncommands: -\nside: harness:read-side"
                        .to_string(),
                    vec![labels::MILESTONE.to_string()]
                ),
                Wrote::SubIssueLink(4, 3),
                Wrote::BlockedByLink(3, 1),
                Wrote::BlockedByLink(3, 2),
                Wrote::Label(3, labels::READY.to_string()),
                Wrote::CreatedIssue(
                    "D".to_string(),
                    "do D\n\n## Architecture\n\nsystems: -\nconcepts: -\naggregates: -\nmicro-uis: -\ncommands: -\nside: harness:read-side"
                        .to_string(),
                    vec![labels::MILESTONE.to_string()]
                ),
                Wrote::SubIssueLink(4, 4),
                Wrote::Label(4, labels::READY.to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn a_milestone_reading_a_later_one_is_in_the_graph_and_an_older_body_is_read_from_its_text()
     {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false);
        context.state.components = vec![
            graph::Component {
                name: "MediaLibrary".to_string(),
                system: "media".to_string(),
                needs: vec![
                    "media.listMedia".to_string(),
                    "session.listScenes".to_string(),
                ],
                actions: vec![],
            },
            graph::Component {
                name: "SceneBoard".to_string(),
                system: "session".to_string(),
                needs: vec!["session.listScenes".to_string()],
                actions: vec!["session.addScene".to_string()],
            },
        ];
        context.state.writes = vec!["session.addScene".to_string()];
        // Open before the plan, written by an older planner: no `micro-uis:`
        // line, its text names the component and quotes the write.
        context.state.existing = vec![Issue {
            number: 9,
            title: "Sessions".to_string(),
            body: "Add `session.addScene` and the SceneBoard.\n\n## Architecture\n\nsystems: session\naggregates: Session\nside: harness:write-side".to_string(),
            ..Issue::default()
        }];
        context.results.insert(
            "plan".to_string(),
            Reply {
                text: r#"[{"title":"Media","goal":"the library","systems":["media"],"micro_uis":["MediaLibrary"]}]"#.to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        Write {
            gh: gh.clone(),
            plan_stage: "plan".to_string(),
            branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("published");
        let Some(Wrote::PutFile(_, _, text)) = gh.writes().pop() else {
            panic!("a graph");
        };
        let written = graph::Graph::parse(&text).expect("a graph");
        assert_eq!(
            written.ids["session.addScene"].producer,
            Some(9),
            "read from the text"
        );
        assert_eq!(written.ids["SceneBoard"].producer, Some(9));
        assert_eq!(written.ids["session.listScenes"].producer, Some(9));
        assert_eq!(
            written.milestones[1].consumes,
            vec!["session.listScenes".to_string()]
        );
        assert!(
            written.reads_ahead().is_empty(),
            "#9 comes first: no read ahead"
        );
        assert!(
            plan::graph_input(
                1,
                "t",
                "the SceneBoardWrapper only",
                &context.state.components,
                &[]
            )
            .micro_uis
            .is_empty(),
            "a longer word is not the component"
        );
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(true);
        Write {
            gh: gh.clone(),
            plan_stage: "plan".to_string(),
            branch: "main_agent".to_string(),
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
            branch: "main_agent".to_string(),
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
            branch: "main_agent".to_string(),
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
            branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }
}
