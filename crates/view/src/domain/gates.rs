//! Gate verdicts: from the events the runs told to what a station shows.
//!
//! A gate tells one `gate_checked` event per check, pass included, under the
//! run's source (`agent-loop/20261009-140427-50`). This module reads them
//! back: which gate said what in which run, the checks of its last pass, and
//! the state an arch takes from it. Pure: events in, views out.

use harness_core::ports::store::events::Stored;
use harness_core::traces::Event;

use crate::domain::blueprint::GateSpec;
use crate::domain::snapshot::{CheckView, GateVerdict, GateView, StationState};

/// One check's verdict, as a run told it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateCheck {
    /// The line: the first half of the event's source.
    pub workflow: String,
    /// The run: the second half.
    pub run_id: String,
    /// When.
    pub at: String,
    /// `code requires`.
    pub gate: String,
    /// `CodeHasASpec`.
    pub check: String,
    /// What it verifies.
    pub purpose: String,
    /// `pass`, `skip` or `halt`.
    pub verdict: String,
    /// Why, for a skip or a halt.
    pub reason: String,
}

/// A run's stop, as it told it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunHalt {
    /// The line.
    pub workflow: String,
    /// The run.
    pub run_id: String,
    /// `STOP`, `FAILED`, `QUOTA`.
    pub kind: String,
    /// Why.
    pub reason: String,
    /// When.
    pub at: String,
}

/// What the runs told about their gates and their stops, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Told {
    /// Every check's verdict.
    pub checks: Vec<GateCheck>,
    /// Every stop.
    pub halts: Vec<RunHalt>,
}

/// `agent-loop/20261009-140427-50` → the line and the run.
fn split_source(source: &str) -> Option<(String, String)> {
    let (workflow, run_id) = source.split_once('/')?;
    (!workflow.is_empty() && !run_id.is_empty()).then(|| (workflow.to_string(), run_id.to_string()))
}

/// Reads the gate verdicts and the stops out of `events`; the watch's own
/// events, told under no run, are left aside.
#[must_use]
pub fn told(events: &[Stored]) -> Told {
    let mut out = Told::default();
    for stored in events {
        let Some((workflow, run_id)) = split_source(&stored.source) else {
            continue;
        };
        match &stored.event {
            Event::GateChecked {
                gate,
                check,
                purpose,
                verdict,
                reason,
            } => out.checks.push(GateCheck {
                workflow,
                run_id,
                at: stored.at.clone(),
                gate: gate.clone(),
                check: check.clone(),
                purpose: purpose.clone(),
                verdict: verdict.clone(),
                reason: reason.clone(),
            }),
            Event::Halted { kind, reason, .. } => out.halts.push(RunHalt {
                workflow,
                run_id,
                kind: kind.clone(),
                reason: reason.clone(),
                at: stored.at.clone(),
            }),
            _ => {}
        }
    }
    out
}

/// The state an arch takes from a verdict.
#[must_use]
pub fn state_of(verdict: &str) -> StationState {
    match verdict {
        "pass" => StationState::Done,
        "skip" => StationState::Skipped,
        "halt" => StationState::Failed,
        _ => StationState::Idle,
    }
}

/// The checks of a gate's last pass through a run: a gate runs its checks in
/// a fixed order and stops at the first that does not pass, so a pass begins
/// wherever its first check speaks again.
fn last_pass<'a>(checks: &[&'a GateCheck]) -> Vec<&'a GateCheck> {
    let Some(first) = checks.first() else {
        return Vec::new();
    };
    let start = checks
        .iter()
        .rposition(|c| c.check == first.check)
        .unwrap_or(0);
    checks[start..].to_vec()
}

impl Told {
    /// The stop of `run_id` on `workflow`, if it told one.
    #[must_use]
    pub fn halt_of(&self, workflow: &str, run_id: &str) -> Option<&RunHalt> {
        self.halts
            .iter()
            .rev()
            .find(|h| h.workflow == workflow && h.run_id == run_id)
    }

    /// Whether a gate of `run_id` halted the round — then the stop is the
    /// gate's to show, not the stage's after it.
    #[must_use]
    pub fn a_gate_halted(&self, workflow: &str, run_id: &str) -> bool {
        self.checks
            .iter()
            .any(|c| c.workflow == workflow && c.run_id == run_id && c.verdict == "halt")
    }

    /// `spec` as `run_id` last saw it — idle when the run never reached it.
    #[must_use]
    pub fn gate_view(&self, spec: &GateSpec, workflow: &str, run_id: Option<&str>) -> GateView {
        let mine: Vec<&GateCheck> = run_id
            .map(|run| {
                self.checks
                    .iter()
                    .filter(|c| c.workflow == workflow && c.run_id == run && c.gate == spec.name)
                    .collect()
            })
            .unwrap_or_default();
        let pass = last_pass(&mine);
        let last = pass.last();
        GateView {
            id: spec.id.clone(),
            name: spec.name.clone(),
            purpose: spec.purpose.clone(),
            state: last.map_or(StationState::Idle, |c| state_of(&c.verdict)),
            verdict: last.map(|c| c.verdict.clone()),
            reason: last
                .map(|c| c.reason.clone())
                .filter(|reason| !reason.is_empty()),
            at: last.map(|c| c.at.clone()),
            run_id: last.map(|c| c.run_id.clone()),
            checks: pass
                .iter()
                .map(|c| CheckView {
                    name: c.check.clone(),
                    purpose: c.purpose.clone(),
                    verdict: c.verdict.clone(),
                    reason: c.reason.clone(),
                    at: c.at.clone(),
                })
                .collect(),
        }
    }

    /// What each gate last said in `run_id`, in the order they first spoke.
    #[must_use]
    pub fn verdicts_of_run(&self, workflow: &str, run_id: &str) -> Vec<GateVerdict> {
        let mut out: Vec<GateVerdict> = Vec::new();
        for c in self
            .checks
            .iter()
            .filter(|c| c.workflow == workflow && c.run_id == run_id)
        {
            let verdict = GateVerdict {
                gate: c.gate.clone(),
                verdict: c.verdict.clone(),
                reason: c.reason.clone(),
                at: c.at.clone(),
            };
            match out.iter_mut().find(|v| v.gate == c.gate) {
                Some(seen) => *seen = verdict,
                None => out.push(verdict),
            }
        }
        out
    }
}

/// The state an arch takes from its gates: a halt anywhere is a stop, a skip
/// anywhere means the stage behind it was skipped, otherwise it is done once
/// any gate spoke.
#[must_use]
pub fn arch_state(gates: &[GateView]) -> StationState {
    if gates.iter().any(|g| g.state == StationState::Failed) {
        StationState::Failed
    } else if gates.iter().any(|g| g.state == StationState::Skipped) {
        StationState::Skipped
    } else if gates.iter().any(|g| g.state == StationState::Done) {
        StationState::Done
    } else {
        StationState::Idle
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;

    /// A gate's verdict as a run would tell it, for the tests.
    #[must_use]
    pub fn checked(
        at: &str,
        source: &str,
        gate: &str,
        check: &str,
        verdict: &str,
        reason: &str,
    ) -> Stored {
        Stored {
            id: 0,
            at: at.to_string(),
            source: source.to_string(),
            event: Event::GateChecked {
                gate: gate.to_string(),
                check: check.to_string(),
                purpose: format!("{check} holds"),
                verdict: verdict.to_string(),
                reason: reason.to_string(),
            },
        }
    }

    /// A run's stop, as it would tell it.
    #[must_use]
    pub fn halted(at: &str, source: &str, kind: &str, reason: &str) -> Stored {
        Stored {
            id: 0,
            at: at.to_string(),
            source: source.to_string(),
            event: Event::Halted {
                workflow: source.to_string(),
                kind: kind.to_string(),
                reason: reason.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{checked, halted};
    use super::*;

    const RUN: &str = "agent-loop/20261009-140427-50";

    fn spec(name: &str) -> GateSpec {
        GateSpec {
            id: name.replace(' ', "-"),
            name: name.to_string(),
            purpose: "checks".to_string(),
        }
    }

    #[test]
    fn a_gate_s_view_is_its_last_pass_through_the_run() {
        // Round 1: the gate passed all three checks; round 2: it stopped at
        // the second one with a skip. The view is round 2's.
        let events = [
            checked(
                "2026-10-09T14:00:01Z",
                RUN,
                "code requires",
                "InThisRun",
                "pass",
                "",
            ),
            checked(
                "2026-10-09T14:00:02Z",
                RUN,
                "code requires",
                "StageAlreadyDone",
                "pass",
                "",
            ),
            checked(
                "2026-10-09T14:00:03Z",
                RUN,
                "code requires",
                "CodeHasASpec",
                "pass",
                "",
            ),
            checked(
                "2026-10-09T14:10:01Z",
                RUN,
                "code requires",
                "InThisRun",
                "pass",
                "",
            ),
            checked(
                "2026-10-09T14:10:02Z",
                RUN,
                "code requires",
                "StageAlreadyDone",
                "skip",
                "already done",
            ),
            checked(
                "2026-10-09T14:10:03Z",
                "watch",
                "code requires",
                "InThisRun",
                "halt",
                "not a run",
            ),
        ];
        let told = told(&events);
        assert_eq!(told.checks.len(), 5, "the watch's events are left aside");
        let view = told.gate_view(
            &spec("code requires"),
            "agent-loop",
            Some("20261009-140427-50"),
        );
        assert_eq!(view.state, StationState::Skipped);
        assert_eq!(view.verdict.as_deref(), Some("skip"));
        assert_eq!(view.reason.as_deref(), Some("already done"));
        assert_eq!(view.at.as_deref(), Some("2026-10-09T14:10:02Z"));
        let names: Vec<&str> = view.checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["InThisRun", "StageAlreadyDone"]);
        assert_eq!(view.checks[0].purpose, "InThisRun holds");
    }

    #[test]
    fn a_gate_the_run_never_reached_is_idle_and_a_passed_one_has_no_reason() {
        let events = [checked(
            "2026-10-09T14:00:01Z",
            RUN,
            "code requires",
            "InThisRun",
            "pass",
            "",
        )];
        let told = told(&events);
        let idle = told.gate_view(
            &spec("create-test requires"),
            "agent-loop",
            Some("20261009-140427-50"),
        );
        assert_eq!(idle.state, StationState::Idle);
        assert!(idle.checks.is_empty() && idle.verdict.is_none());
        let passed = told.gate_view(
            &spec("code requires"),
            "agent-loop",
            Some("20261009-140427-50"),
        );
        assert_eq!(passed.state, StationState::Done);
        assert_eq!(passed.reason, None);
        let nowhere = told.gate_view(&spec("code requires"), "agent-loop", None);
        assert_eq!(nowhere.state, StationState::Idle);
    }

    #[test]
    fn an_arch_takes_the_worst_of_its_gates() {
        let done = GateView {
            state: StationState::Done,
            ..told(&[]).gate_view(&spec("a"), "x", None)
        };
        let skipped = GateView {
            state: StationState::Skipped,
            ..done.clone()
        };
        let failed = GateView {
            state: StationState::Failed,
            ..done.clone()
        };
        let idle = GateView {
            state: StationState::Idle,
            ..done.clone()
        };
        assert_eq!(
            arch_state(&[idle.clone(), idle.clone()]),
            StationState::Idle
        );
        assert_eq!(arch_state(&[done.clone(), idle]), StationState::Done);
        assert_eq!(arch_state(&[done, skipped.clone()]), StationState::Skipped);
        assert_eq!(arch_state(&[skipped, failed]), StationState::Failed);
        assert_eq!(arch_state(&[]), StationState::Idle);
    }

    #[test]
    fn a_run_s_verdicts_keep_one_line_per_gate_in_first_spoken_order() {
        let events = [
            checked(
                "2026-10-09T14:00:01Z",
                RUN,
                "code requires",
                "InThisRun",
                "pass",
                "",
            ),
            checked(
                "2026-10-09T14:00:02Z",
                RUN,
                "code must keep the architecture",
                "ArchitectureHolds",
                "halt",
                "drifted",
            ),
            checked(
                "2026-10-09T14:10:01Z",
                RUN,
                "code requires",
                "InThisRun",
                "skip",
                "not in --stages",
            ),
            halted("2026-10-09T14:10:02Z", RUN, "STOP", "drifted"),
        ];
        let told = told(&events);
        let verdicts = told.verdicts_of_run("agent-loop", "20261009-140427-50");
        let seen: Vec<(&str, &str)> = verdicts
            .iter()
            .map(|v| (v.gate.as_str(), v.verdict.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                ("code requires", "skip"),
                ("code must keep the architecture", "halt")
            ]
        );
        assert!(told.a_gate_halted("agent-loop", "20261009-140427-50"));
        assert_eq!(
            told.halt_of("agent-loop", "20261009-140427-50")
                .map(|h| h.kind.as_str()),
            Some("STOP")
        );
        assert!(told.halt_of("agent-loop", "other").is_none());
    }
}
