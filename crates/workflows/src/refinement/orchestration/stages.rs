//! Refinement: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** The order of entries
//! *is* the execution order: the router, the sections of the phase in
//! canonical body order, then coherence, then — for the business phase — the
//! advice on the technical refinement, then publishing, a table stage but not
//! a session.
//!
//! **Stage names are written here, nowhere else.** Gates and actions receive
//! them as fields: two literals in two layers would silently desynchronize —
//! a gate looking for a stage's reply under a name nobody wrote thinks it
//! skipped.
//!
//! The repo map (`ground` + `explore`) is **not** wired here: it comes from
//! [`crate::common::explore::entries`], which [`crate::refinement::run::build`]
//! puts at the head of the sequence — this table itself has no way to read
//! the repo, and doesn't need to.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody, Unpaid};
use harness_core::ports::agent::SessionSpec;

use crate::refinement::action::actions::{AskRefine, RecordWantedSections};
use crate::refinement::action::publish::Write;
use crate::refinement::checks::gates::{
    IssueIsATask, NothingIsWritten, RouterIsOff, RouterNamedSections, SectionIsWanted,
    SectionSitsInTheLayout,
};
use crate::refinement::config::Config;
use crate::refinement::data::phase::Phase;
use crate::refinement::data::state::RefinementState;
use crate::refinement::orchestration::prompts as text;
use crate::refinement::ports::Ports;

/// The router stage name.
pub const ROUTER: &str = "router";
/// The coherence stage name.
pub const COHERENCE: &str = "coherence";
/// The advice stage name: should the technical refinement have a human?
pub const ADVICE: &str = "human-advice";
/// The publish local stage name.
pub const PUBLISH: &str = "publish";

/// The template for each stage, by stage key.
fn template_of(key: &str) -> &'static str {
    match key {
        ROUTER => text::ROUTER_PROMPT,
        "business-goal" => text::BUSINESS_GOAL_PROMPT,
        "technical" => text::TECHNICAL_PROMPT,
        "acceptance-criteria" => text::ACCEPTANCE_CRITERIA_PROMPT,
        "business-rules" => text::BUSINESS_RULES_PROMPT,
        "technical-plan" => text::TECHNICAL_PLAN_PROMPT,
        COHERENCE => text::COHERENCE_PROMPT,
        ADVICE => text::ADVICE_PROMPT,
        other => unreachable!("unknown stage: {other}"),
    }
}

fn spec_of(config: &Config, key: &str) -> SessionSpec {
    match key {
        "business-goal" => config.goal.clone(),
        "technical" => config.technical.clone(),
        "acceptance-criteria" => config.criteria.clone(),
        "business-rules" => config.rules.clone(),
        "technical-plan" => config.plan.clone(),
        other => unreachable!("unknown section key: {other}"),
    }
}

fn paid(
    ports: &Ports,
    config: &Config,
    phase: Phase,
    stage: &str,
    spec: SessionSpec,
) -> Stage<RefinementState> {
    Stage {
        name: stage.to_string(),
        pre: None,
        post: None,
        body: StageBody::Session {
            spec,
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskRefine {
                stage: stage.to_string(),
                template: template_of(stage),
                merged_body: stage == COHERENCE || stage == ADVICE,
                phase,
                context: config.context.clone(),
                artifacts_dir: config.artifacts_dir.clone(),
                explore: config.explore,
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// The router, skipped before round 3 or without `--context`.
#[must_use]
pub fn router(ports: &Ports, config: &Config, phase: Phase) -> Stage<RefinementState> {
    let mut stage = paid(ports, config, phase, ROUTER, config.router.clone());
    stage.pre = Some(Gate {
        name: "router requires",
        checks: vec![Box::new(RouterIsOff {
            router: ROUTER.to_string(),
            has_context: !config.context.is_empty(),
        })],
    });
    stage.post = Some(Gate {
        name: "router must achieve",
        checks: vec![Box::new(RouterNamedSections {
            router: ROUTER.to_string(),
        })],
    });
    if let StageBody::Session { actions, .. } = &mut stage.body {
        actions.push(Box::new(Unpaid(RecordWantedSections {
            router: ROUTER.to_string(),
        })));
    }
    stage
}

/// A section, skipped when this round doesn't write it.
#[must_use]
pub fn section(
    ports: &Ports,
    config: &Config,
    phase: Phase,
    key: &'static str,
) -> Stage<RefinementState> {
    let mut stage = paid(ports, config, phase, key, spec_of(config, key));
    stage.pre = Some(Gate {
        name: "section requires",
        checks: vec![Box::new(SectionIsWanted {
            key: key.to_string(),
        })],
    });
    stage.post = Some(Gate {
        name: "section must sit in the layout",
        checks: vec![Box::new(SectionSitsInTheLayout {
            key: key.to_string(),
        })],
    });
    stage
}

/// Coherence: reads this round's sections together, refines them.
#[must_use]
pub fn coherence(ports: &Ports, config: &Config, phase: Phase) -> Stage<RefinementState> {
    paid(ports, config, phase, COHERENCE, config.coherence.clone())
}

/// Advice: reads the finished business half, says whether the technical half
/// wants a human on the issue. Posted as a comment by [`publish`]. Skipped
/// on a milestone, which has no technical half.
#[must_use]
pub fn advice(ports: &Ports, config: &Config, phase: Phase) -> Stage<RefinementState> {
    let mut stage = paid(ports, config, phase, ADVICE, config.advice.clone());
    stage.pre = Some(Gate {
        name: "advice requires",
        checks: vec![Box::new(IssueIsATask)],
    });
    stage
}

/// Publishing: a local stage, which costs nothing.
#[must_use]
pub fn publish(ports: &Ports, config: &Config, phase: Phase) -> Stage<RefinementState> {
    Stage {
        name: PUBLISH.to_string(),
        pre: Some(Gate {
            name: "publish requires",
            checks: vec![Box::new(NothingIsWritten)],
        }),
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Write {
                gh: Rc::clone(&ports.gh),
                coherence: COHERENCE.to_string(),
                advice: (phase == Phase::Business).then(|| ADVICE.to_string()),
                refinement_dir: config.refinement_dir.clone(),
            })],
        },
    }
}

/// The router, the phase's sections, coherence, the advice (business phase
/// only), publishing — in order. The map (`ground`/`explore`) is not here:
/// see the module.
///
/// Coherence has no `skip`: by the time the sequence reaches it,
/// `state.wanted` is never empty (a plain round sets it, and a routed round
/// that named nothing would already fail in `RouterNamedSections`) — nothing
/// is ever skipped.
#[must_use]
pub fn table(ports: &Ports, config: &Config, phase: Phase) -> Vec<Stage<RefinementState>> {
    let mut built = Vec::with_capacity(7);
    built.push(router(ports, config, phase));
    for key in phase.keys() {
        built.push(section(ports, config, phase, key));
    }
    built.push(coherence(ports, config, phase));
    if phase == Phase::Business {
        built.push(advice(ports, config, phase));
    }
    built.push(publish(ports, config, phase));
    built
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refinement::config::fake as config_fake;
    use crate::refinement::ports::fake as ports_fake;

    fn names(phase: Phase) -> Vec<String> {
        table(&ports_fake::ports(), &config_fake::config(), phase)
            .iter()
            .map(|stage| stage.name.clone())
            .collect()
    }

    #[test]
    fn the_business_table_ends_with_the_advice_then_publishing() {
        assert_eq!(
            names(Phase::Business),
            [
                ROUTER,
                "business-goal",
                "acceptance-criteria",
                "business-rules",
                COHERENCE,
                ADVICE,
                PUBLISH,
            ]
        );
    }

    #[test]
    fn the_technical_table_has_no_advice() {
        assert_eq!(
            names(Phase::Technical),
            [ROUTER, "technical", "technical-plan", COHERENCE, PUBLISH]
        );
    }

    #[test]
    fn every_section_key_has_a_template_and_a_spec() {
        // `template_of` and `spec_of` are two hand-written exhaustive matches:
        // a key added to a phase without text or model would panic during
        // table assembly, not a test.
        let config = config_fake::config();
        for phase in [Phase::Business, Phase::Technical] {
            for key in phase.keys() {
                assert!(!template_of(key).is_empty());
                assert!(!spec_of(&config, key).model.is_empty());
            }
        }
        assert!(!template_of(ADVICE).is_empty());
    }

    #[test]
    fn no_template_holds_a_placeholder_nobody_fills() {
        // An unspliced placeholder does not fail anything: it ships into a
        // paid prompt, literally, and the session reads it as part of the
        // request. Named without braces, so this list does not read as
        // formatting arguments itself.
        let filled = [
            "num",
            "title",
            "round",
            "keys",
            "drags",
            "additional_context",
            "body",
        ];
        let mut keys: Vec<&str> = Phase::Business.keys().to_vec();
        keys.extend(Phase::Technical.keys());
        keys.extend([ROUTER, COHERENCE, ADVICE]);
        for key in keys {
            let text = template_of(key);
            for (at, _) in text.match_indices('{') {
                let Some(end) = text[at..].find('}') else {
                    continue;
                };
                let found = &text[at + 1..at + end];
                assert!(
                    filled.contains(&found),
                    "{key}'s template asks for {found}, which nothing fills"
                );
            }
        }
    }
}
