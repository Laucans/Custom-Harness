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
        if let Some(why) = data_layer_not_first(ctx, &items) {
            return Err(Halt::Failed(why));
        }
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

/// Why the plan does not lay the Data layer first, when the dossier has
/// aggregates the checkout has not built yet.
///
/// The database is designed whole at the creation (`mock/data/model.json`,
/// one `aggregate.json` per aggregate); laid one aggregate per feature
/// milestone, every feature waits on the one before it for its tables —
/// `ForgeDesMondes` had 53 of 63 open tasks queued behind one table at a
/// time, with five lanes. So: one milestone produces every aggregate not
/// built yet, and no milestone before it builds a Micro-UI or a write — the
/// skeleton (tooling) alone may come first. Refused with the gap named, for
/// the retry to lay the Data layer whole.
fn data_layer_not_first(
    ctx: &Context<PlannerState>,
    items: &[plan::MilestoneItem],
) -> Option<String> {
    let built = &ctx.state.standing.built;
    let unbuilt: Vec<&String> = ctx
        .state
        .aggregates
        .iter()
        .filter(|a| !built.iter().any(|b| b.eq_ignore_ascii_case(a)))
        .collect();
    if unbuilt.is_empty() {
        return None;
    }
    // The existing milestones already produce what they produce: only a
    // plan that opens the first aggregate milestone is judged.
    let existing_produce_aggregates = ctx.state.existing.iter().any(|issue| {
        !plan::graph_input(issue.number, &issue.title, &issue.body, &[], &[])
            .aggregates
            .is_empty()
    });
    if existing_produce_aggregates {
        return None;
    }
    let producers: Vec<(usize, &plan::MilestoneItem)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| !item.aggregates.is_empty())
        .collect();
    let Some((at, data_layer)) = producers.first().copied() else {
        return Some(format!(
            "the plan lays no aggregate, and the dossier has {} the checkout has not built ({}): \
             one milestone, right after the skeleton, lays them all — their tables, migrations \
             and aggregate.json — before any milestone reads or writes them",
            unbuilt.len(),
            unbuilt
                .iter()
                .map(|a| a.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    if producers.len() > 1 {
        return Some(format!(
            "the Data layer is split over {} milestones ({}): one milestone, right after the \
             skeleton, lays every aggregate of the dossier whole; the later ones change a \
             table by a new migration, never produce it",
            producers.len(),
            producers
                .iter()
                .map(|(_, m)| format!("{:?}", m.title))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let missing: Vec<&str> = unbuilt
        .iter()
        .filter(|a| {
            !data_layer
                .aggregates
                .iter()
                .any(|m| m.eq_ignore_ascii_case(a))
        })
        .map(|a| a.as_str())
        .collect();
    if !missing.is_empty() {
        return Some(format!(
            "{:?} lays the Data layer but leaves out {}: every aggregate of the dossier is laid \
             there, whole, in one milestone",
            data_layer.title,
            missing.join(", ")
        ));
    }
    let consumer_before: Vec<&str> = items[..at]
        .iter()
        .filter(|m| !m.micro_uis.is_empty() || !m.commands.is_empty() || !m.screens.is_empty())
        .map(|m| m.title.as_str())
        .collect();
    if !consumer_before.is_empty() {
        return Some(format!(
            "{} come(s) before the Data layer ({:?}) and build(s) a Micro-UI, a write or a \
             screen: only the skeleton (tooling) may come before the aggregates",
            consumer_before
                .iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join(", "),
            data_layer.title
        ));
    }
    None
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
    async fn the_data_layer_is_laid_whole_in_one_milestone_right_after_the_skeleton() {
        let mut context = ctx(false);
        context.state.aggregates = vec!["Campaign".to_string(), "Media".to_string()];
        let gate = PlanParses {
            stage: "plan".to_string(),
        };
        // Per feature: refused, the two aggregates are laid by two milestones.
        let per_feature = r#"[{"title":"Skeleton","goal":"s","systems":["platform"]},{"title":"Campaigns","goal":"c","systems":["campaign"],"aggregates":["Campaign"],"micro_uis":["CampaignList"]},{"title":"Media","goal":"m","systems":["media"],"aggregates":["Media"]}]"#;
        context
            .results
            .insert("plan".to_string(), reply(per_feature));
        let err = gate.verify(&context).await.expect_err("split data layer");
        assert!(
            err.reason().contains("split over 2 milestones"),
            "{}",
            err.reason()
        );
        // One milestone, but a consumer before it: refused.
        let consumer_first = r#"[{"title":"Campaign list","goal":"c","systems":["campaign"],"micro_uis":["CampaignList"]},{"title":"Data layer","goal":"d","systems":["campaign","media"],"aggregates":["Campaign","Media"]}]"#;
        context
            .results
            .insert("plan".to_string(), reply(consumer_first));
        let err = gate.verify(&context).await.expect_err("consumer first");
        assert!(
            err.reason().contains("come(s) before the Data layer"),
            "{}",
            err.reason()
        );
        // One milestone, an aggregate left out: refused.
        let partial = r#"[{"title":"Data layer","goal":"d","systems":["campaign"],"aggregates":["Campaign"]}]"#;
        context.results.insert("plan".to_string(), reply(partial));
        let err = gate.verify(&context).await.expect_err("partial");
        assert!(
            err.reason().contains("leaves out Media"),
            "{}",
            err.reason()
        );
        // Skeleton, then the Data layer whole, then the features: passes.
        let whole = r#"[{"title":"Skeleton","goal":"s","systems":["platform"]},{"title":"Data layer","goal":"d","systems":["campaign","media"],"aggregates":["Campaign","Media"]},{"title":"Campaigns","goal":"c","systems":["campaign"],"micro_uis":["CampaignList"]}]"#;
        context.results.insert("plan".to_string(), reply(whole));
        assert_eq!(
            gate.verify(&context).await.expect("verdict"),
            Verdict::Continue
        );
        // A consumer before it that only writes, or only freezes a screen:
        // refused the same.
        let writer_first = r#"[{"title":"Approvals","goal":"w","systems":["campaign"],"commands":["campaign.approve"]},{"title":"Data layer","goal":"d","systems":["campaign","media"],"aggregates":["Campaign","Media"]}]"#;
        context
            .results
            .insert("plan".to_string(), reply(writer_first));
        let err = gate.verify(&context).await.expect_err("writer first");
        assert!(
            err.reason().contains("\"Approvals\" come(s) before"),
            "{}",
            err.reason()
        );
        let screen_first = r#"[{"title":"Home","goal":"h","systems":["campaign"],"screens":["home"]},{"title":"Data layer","goal":"d","systems":["campaign","media"],"aggregates":["Campaign","Media"]}]"#;
        context
            .results
            .insert("plan".to_string(), reply(screen_first));
        let err = gate.verify(&context).await.expect_err("screen first");
        assert!(
            err.reason().contains("\"Home\" come(s) before"),
            "{}",
            err.reason()
        );
        // An existing milestone already lays an aggregate: the Data layer was
        // judged when it was opened; a later plan is not held to it.
        context.state.existing = vec![Issue {
            number: 18,
            title: "Campaigns".to_string(),
            body: "x\n\n## Architecture\n\nsystems: campaign\naggregates: Campaign\nside: harness:write-side".to_string(),
            ..Issue::default()
        }];
        context
            .results
            .insert("plan".to_string(), reply(per_feature));
        assert_eq!(
            gate.verify(&context).await.expect("verdict"),
            Verdict::Continue
        );
        // An existing milestone that lays nothing (the skeleton) keeps the
        // new plan under judgement.
        context.state.existing = vec![Issue {
            number: 16,
            title: "Skeleton".to_string(),
            body:
                "x\n\n## Architecture\n\nsystems: platform\naggregates: -\nside: harness:read-side"
                    .to_string(),
            ..Issue::default()
        }];
        context
            .results
            .insert("plan".to_string(), reply(per_feature));
        let err = gate.verify(&context).await.expect_err("still judged");
        assert!(
            err.reason().contains("split over 2 milestones"),
            "{}",
            err.reason()
        );
        // Already built in the checkout: nothing to lay, any plan passes.
        context.state.existing = Vec::new();
        context.state.standing.built = vec!["Campaign".to_string(), "Media".to_string()];
        context
            .results
            .insert("plan".to_string(), reply(per_feature));
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
