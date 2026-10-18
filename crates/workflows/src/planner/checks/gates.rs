//! What a planner stage requires before paying, and what it must obtain.
//!
//! One gate: the plan step's reply must parse, even after its own one retry
//! (`action::actions::AskForPlan`), and the plan it carries must have no
//! cycle — no milestone reads what a later milestone produces. A
//! `Verification` judges and does not write (decision #1) — the publish
//! action re-parses the same text to actually act on it, the acknowledged
//! price of that split.

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use harness_architecture::graph;

use crate::planner::data::plan;
use crate::planner::data::state::PlannerState;

/// The plan stage's reply parses as a JSON array of milestones, and the
/// plan has no cycle.
///
/// A milestone that reads what a later milestone produces was a warning
/// (`reads_ahead`): `ForgeDesMondes`' #19 and #20 read each other's
/// Capabilities, the train dropped the backward edge, and #19 could land
/// reading a Capability not built. Now that a Micro-UI reads any system and
/// the first reader builds what it reads, such a read is only a plan laid
/// out of order — refused, with the reads named, for the retry to reorder.
pub struct PlanParses {
    /// The stage whose reply this judges.
    pub stage: String,
}

#[async_trait(?Send)]
impl Verification<PlannerState> for PlanParses {
    fn purpose(&self) -> String {
        "the plan stage's reply parses as a JSON array of milestones".to_string()
    }

    async fn verify(&self, ctx: &Context<PlannerState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some(reply) = ctx.results.get(&self.stage) else {
            return Err(Halt::Failed(format!(
                "no reply recorded under {} — the plan stage did not run",
                self.stage
            )));
        };
        let items = match plan::parse(&reply.text) {
            Ok(items) => items,
            Err(e) => {
                return Err(Halt::Failed(format!(
                    "the plan did not parse as JSON, even after one retry: {e}"
                )));
            }
        };
        let ahead = reads_ahead(ctx, &items);
        if ahead.is_empty() {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Failed(format!(
            "the plan has a cycle — a milestone reads what a later one produces; order the plan so \
             that every milestone comes after what it reads from:\n  - {}",
            ahead.join("\n  - ")
        )))
    }
}

/// Every read of a later milestone the plan would make, said for the retry.
///
/// The milestones to open have no number yet: they take provisional ones
/// past the existing, in delivery order — what the graph reads of them is
/// their position in the plan, not their number.
fn reads_ahead(ctx: &Context<PlannerState>, items: &[plan::MilestoneItem]) -> Vec<String> {
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
    let mut next = milestones.iter().map(|m| m.number).max().unwrap_or(0);
    for item in items {
        next = next.saturating_add(1);
        milestones.push(plan::graph_input(
            next,
            &item.title,
            &item.body(),
            &ctx.state.components,
            &ctx.state.writes,
        ));
    }
    let found = graph::Graph::derive(&milestones, &ctx.state.components, &ctx.state.screens);
    let title = |number: u64| {
        milestones
            .iter()
            .find(|m| m.number == number)
            .map_or_else(|| format!("#{number}"), |m| format!("{:?}", m.title))
    };
    found
        .reads_ahead()
        .into_iter()
        .map(|ahead| {
            format!(
                "{} reads `{}`, which {} produces later",
                title(ahead.consumer),
                ahead.id,
                title(ahead.producer)
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool) -> Context<PlannerState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            PlannerState {
                roadmap: Some(Issue::default()),
                ..PlannerState::default()
            },
            Logbook::null(),
        )
    }

    fn reply(text: &str) -> Reply {
        Reply {
            text: text.to_string(),
            stop_line: None,
            spend: Spend::default(),
        }
    }

    #[tokio::test]
    async fn a_parseable_plan_passes() {
        let mut context = ctx(false);
        context.results.insert("plan".to_string(), reply("[]"));
        let gate = PlanParses {
            stage: "plan".to_string(),
        };
        assert_eq!(
            gate.verify(&context).await.expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_plan_where_a_milestone_reads_what_a_later_one_produces_is_refused() {
        // "Preparation" freezes a screen composing SceneBoard, which
        // "Sessions" builds after it: a cycle in the plan's order.
        let mut context = ctx(false);
        context.state.components = vec![graph::Component {
            name: "SceneBoard".to_string(),
            system: "session".to_string(),
            needs: vec!["session.listScenes".to_string()],
            actions: vec![],
        }];
        context.state.screens = vec![graph::Screen {
            name: "preparation-seance".to_string(),
            micro_uis: vec!["SceneBoard".to_string()],
        }];
        let out_of_order = r#"[{"title":"Preparation","goal":"freeze","systems":["session"],"screens":["preparation-seance"]},{"title":"Sessions","goal":"scenes","systems":["session"],"micro_uis":["SceneBoard"]}]"#;
        context
            .results
            .insert("plan".to_string(), reply(out_of_order));
        let gate = PlanParses {
            stage: "plan".to_string(),
        };
        let err = gate.verify(&context).await.expect_err("a cycle");
        assert!(matches!(err, Halt::Failed(_)));
        assert!(
            err.reason()
                .contains("\"Preparation\" reads `SceneBoard`, which \"Sessions\" produces later"),
            "{}",
            err.reason()
        );
        let in_order = r#"[{"title":"Sessions","goal":"scenes","systems":["session"],"micro_uis":["SceneBoard"]},{"title":"Preparation","goal":"freeze","systems":["session"],"screens":["preparation-seance"]}]"#;
        context.results.insert("plan".to_string(), reply(in_order));
        assert_eq!(
            gate.verify(&context).await.expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn an_unparseable_plan_fails_the_gate() {
        let mut context = ctx(false);
        context
            .results
            .insert("plan".to_string(), reply("not json"));
        let gate = PlanParses {
            stage: "plan".to_string(),
        };
        let err = gate.verify(&context).await.expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }

    #[tokio::test]
    async fn no_reply_at_all_fails_rather_than_reading_as_empty() {
        let context = ctx(false);
        let gate = PlanParses {
            stage: "plan".to_string(),
        };
        let err = gate.verify(&context).await.expect_err("must fail");
        assert!(err.reason().contains("did not run"));
    }

    #[tokio::test]
    async fn a_dry_run_skips_the_check_entirely() {
        let context = ctx(true);
        let gate = PlanParses {
            stage: "plan".to_string(),
        };
        assert_eq!(
            gate.verify(&context).await.expect("verdict"),
            Verdict::Continue
        );
    }
}
