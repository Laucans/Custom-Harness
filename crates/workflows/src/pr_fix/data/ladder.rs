//! Which model a repair runs on, and why — the line the run's journal says.
//!
//! The ladder: sonnet for the first [`REPAIRS_ON_SONNET`] repairs of a PR,
//! opus once they failed (the owner's rule, 2026-10-20: most repairs are a
//! conflict or tests for survivors). One rung earlier for the data layer
//! (2026-10-25): a `harness:data-layer` task's PR whose checks are still red
//! after its first repair gets its second on opus — the blind review's
//! advice for keeping sonnet on the data layer. Every other PR keeps the
//! ladder.
//!
//! [`choose`] is the pure decision; [`situation`] reads what it needs through
//! the `GitHub` port, and only what the decision can use.

use harness_core::ports::agent::SessionSpec;
use harness_core::ports::shell::github::GitHub;

use crate::common::{labels, review};
use crate::dev_loop::data::tasks;

/// How many repairs a PR gets on the cheaper model before the stronger one
/// takes over — one fewer for a data-layer PR still red.
const REPAIRS_ON_SONNET: usize = 2;

/// What the choice of a repair's model depends on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Situation {
    /// How many repairs the PR already had, decisions to repair once more
    /// included ([`review::repairs_in_all`]).
    pub repairs_before: usize,
    /// The PR delivers a `harness:data-layer` task.
    pub data_layer: bool,
    /// One of the PR's checks is red.
    pub gate_red: bool,
}

/// The session a repair runs, and why, in the journal's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// Model and effort.
    pub spec: SessionSpec,
    /// Why this model.
    pub why: String,
}

/// The repair's model and why: see the module.
#[must_use]
pub fn choose(situation: Situation) -> Choice {
    let (model, why) = if situation.repairs_before >= REPAIRS_ON_SONNET {
        (
            "opus",
            format!("the {REPAIRS_ON_SONNET} repairs on sonnet did not clear it"),
        )
    } else if situation.data_layer && situation.repairs_before >= 1 && situation.gate_red {
        (
            "opus",
            format!(
                "a {} task's PR with a check still red after its first repair",
                labels::DATA_LAYER
            ),
        )
    } else {
        ("sonnet", "the ladder's first rungs".to_string())
    };
    Choice {
        spec: SessionSpec {
            model: model.to_string(),
            effort: "high".to_string(),
        },
        why,
    }
}

/// The journal's line for this repair: which one it is on the PR, the
/// session it runs and why — `#12: repair 2 on this PR — opus/high (…)`.
#[must_use]
pub fn line(pr_ref: &str, situation: Situation, choice: &Choice) -> String {
    format!(
        "#{}: repair {} on this PR — {}/{} ({})",
        pr_ref.trim_start_matches('#'),
        situation.repairs_before.saturating_add(1),
        choice.spec.model,
        choice.spec.effort,
        choice.why
    )
}

/// True when the rung the data layer moves is the one this repair stands on.
const fn data_layer_rung(repairs_before: usize) -> bool {
    repairs_before >= 1 && repairs_before < REPAIRS_ON_SONNET
}

/// What [`choose`] needs of the PR `pr_ref`, read through `gh`.
///
/// Its repairs so far and — only on the rung the data layer moves — whether it delivers
/// a data-layer task and a check is red. An unreadable read counts as
/// nothing: a first repair, no data layer, nothing red.
pub async fn situation(gh: &dyn GitHub, pr_ref: &str) -> Situation {
    let num = pr_ref.trim_start_matches('#');
    let repairs_before = gh
        .pr_comments(num)
        .await
        .map_or(0, |comments| review::repairs_in_all(&comments));
    if !data_layer_rung(repairs_before) {
        return Situation {
            repairs_before,
            ..Situation::default()
        };
    }
    let Ok(pr) = gh.pr(num).await else {
        return Situation {
            repairs_before,
            ..Situation::default()
        };
    };
    let data_layer = gh
        .issues_labelled(labels::DATA_LAYER, "open")
        .await
        .is_ok_and(|found| {
            found
                .iter()
                .any(|task| tasks::declared_branch(&task.body).as_deref() == Some(pr.head.as_str()))
        });
    let gate_red = data_layer
        && gh
            .pr_failing_checks(num)
            .await
            .is_ok_and(|failing| !failing.is_empty());
    Situation {
        repairs_before,
        data_layer,
        gate_red,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use harness_core::domain::{Issue, Pr};
    use std::collections::HashMap;

    fn at(repairs_before: usize, data_layer: bool, gate_red: bool) -> Situation {
        Situation {
            repairs_before,
            data_layer,
            gate_red,
        }
    }

    fn model(situation: Situation) -> String {
        choose(situation).spec.model
    }

    #[test]
    fn a_plain_pr_keeps_two_repairs_on_sonnet_then_opus() {
        for red in [false, true] {
            assert_eq!(model(at(0, false, red)), "sonnet");
            assert_eq!(model(at(1, false, red)), "sonnet");
            assert_eq!(model(at(2, false, red)), "opus");
            assert_eq!(model(at(5, false, red)), "opus");
        }
        assert_eq!(choose(at(0, false, false)).spec.effort, "high");
        assert!(
            choose(at(2, false, false))
                .why
                .contains("2 repairs on sonnet")
        );
    }

    #[test]
    fn a_data_layer_pr_still_red_after_its_first_repair_gets_the_second_on_opus() {
        assert_eq!(model(at(0, true, true)), "sonnet", "the first stays");
        let second = choose(at(1, true, true));
        assert_eq!(second.spec.model, "opus");
        assert!(second.why.contains("harness:data-layer"), "{}", second.why);
        assert!(second.why.contains("still red after its first repair"));
        assert_eq!(
            model(at(1, true, false)),
            "sonnet",
            "nothing red: a review asked it"
        );
        assert_eq!(model(at(1, false, true)), "sonnet", "not the data layer");
    }

    #[test]
    fn the_journal_line_says_which_repair_its_session_and_why() {
        let situation = at(1, true, true);
        let choice = choose(situation);
        assert_eq!(
            line("#12", situation, &choice),
            "#12: repair 2 on this PR — opus/high (a harness:data-layer task's PR with a \
             check still red after its first repair)"
        );
    }

    #[test]
    fn only_the_second_repair_stands_on_the_rung_the_data_layer_moves() {
        assert!(!data_layer_rung(0));
        assert!(data_layer_rung(1));
        assert!(!data_layer_rung(REPAIRS_ON_SONNET));
    }

    fn fix_marks(count: usize) -> String {
        review::FIX_MARKER.repeat(count)
    }

    fn github(repairs: usize, data_layer: bool, failing: &[&str]) -> FakeGitHub {
        let label = if data_layer {
            labels::DATA_LAYER
        } else {
            labels::AGENT
        };
        FakeGitHub {
            pr_comment_bodies: vec![("12".to_string(), fix_marks(repairs))],
            prs: vec![(
                "12".to_string(),
                Pr {
                    num: "12".to_string(),
                    head: "feat/entries".to_string(),
                    ..Pr::default()
                },
            )],
            issues: vec![
                Issue {
                    number: 40,
                    body: "## Scope\n\nbranch: feat/other\n".to_string(),
                    labels: vec![labels::DATA_LAYER.to_string()],
                    ..Issue::default()
                },
                Issue {
                    number: 41,
                    body: "## Scope\n\nbranch: feat/entries\n".to_string(),
                    labels: vec![label.to_string()],
                    ..Issue::default()
                },
            ],
            failing_checks: HashMap::from([(
                "12".to_string(),
                failing.iter().map(ToString::to_string).collect(),
            )]),
            ..FakeGitHub::default()
        }
    }

    #[tokio::test]
    async fn the_situation_is_read_off_the_pr_its_task_and_its_checks() {
        let read = situation(&github(1, true, &["gates / tests"]), "#12").await;
        assert_eq!(read, at(1, true, true));
        let green = situation(&github(1, true, &[]), "12").await;
        assert_eq!(green, at(1, true, false));
        let plain = situation(&github(1, false, &["gates / tests"]), "12").await;
        assert_eq!(
            plain,
            at(1, false, false),
            "a plain task's checks are not read"
        );
    }

    #[tokio::test]
    async fn off_the_data_layer_s_rung_only_the_repairs_are_read() {
        for repairs in [0, REPAIRS_ON_SONNET, 4] {
            let mut gh = github(repairs, true, &["gates / tests"]);
            gh.prs.clear();
            let read = situation(&gh, "12").await;
            assert_eq!(read, at(repairs, false, false));
        }
    }

    #[tokio::test]
    async fn an_unreadable_pr_or_board_counts_as_nothing() {
        let mut gh = github(1, true, &["gates / tests"]);
        gh.prs.clear();
        assert_eq!(situation(&gh, "12").await, at(1, false, false));
        let broken = FakeGitHub {
            broken: Some(harness_core::domain::Halt::Failed("down".to_string())),
            ..FakeGitHub::default()
        };
        assert_eq!(situation(&broken, "12").await, at(0, false, false));
        let mut unknown_checks = github(1, true, &[]);
        unknown_checks.failing_checks.clear();
        assert_eq!(situation(&unknown_checks, "12").await, at(1, true, false));
    }
}
