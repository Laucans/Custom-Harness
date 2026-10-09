//! Assemble an entire refinement round from ports and a config already
//! built.
//!
//! **Not the construction of concrete adapters** — that remains the launcher's
//! work, the only place with the right to name a `GhCli` or
//! `ClaudeCliFactory`. This module receives filled [`Ports`] and [`Config`] —
//! its own and those of the repository map, which is shared — and assembles
//! the form that
//! [`orchestration::workflow`](crate::refinement::orchestration::workflow)
//! carries.
//!
//! The map at the head of the sequence is assembled by
//! [`orchestration::round`](crate::refinement::orchestration::round): it's
//! the round that carries the order, not the assembly.

use std::cell::Cell;
use std::rc::Rc;

use harness_core::execution::Gate;

use crate::common::explore;
use crate::refinement::config::Config;
use crate::refinement::data::phase::Phase;
use crate::refinement::data::state::RefinementState;
use crate::refinement::orchestration::round;
use crate::refinement::orchestration::workflow::RefinementRun;
use crate::refinement::ports::Ports;

/// What an invocation requests — distinct from [`Ports`] and [`Config`],
/// which are infrastructure rather than a request.
pub struct Request {
    /// Which half of the refinement to run.
    pub phase: Phase,
    /// The issue to refine.
    pub issue: u64,
    /// What a human asked for this round, verbatim.
    pub context: String,
    /// Refine even if the issue doesn't carry `harness:refinement`.
    pub force: bool,
}

/// Assemble an entire refinement round from its wiring.
#[must_use]
pub fn build(
    ports: &Ports,
    config: &Config,
    explore_ports: &explore::Ports,
    explore_config: &explore::Config,
    request: Request,
    pre: Gate<RefinementState>,
) -> RefinementRun {
    RefinementRun {
        pre,
        // A run refines one round, and only one: the round counter lives in
        // the issue's comments, not in this process.
        remaining: Cell::new(1),
        gh: Rc::clone(&ports.gh),
        locks: Rc::clone(&ports.locks),
        refinement_dir: config.refinement_dir.clone(),
        issue_key: request.issue.to_string(),
        issue: request.issue,
        context: request.context,
        force: request.force,
        phase: request.phase,
        round: round::build(ports, config, explore_ports, explore_config, request.phase),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refinement::config::fake as config_fake;
    use crate::refinement::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_and_the_round_straight_through() {
        let config = config_fake::config();
        let built = build(
            &ports_fake::ports(),
            &config,
            &explore::fake::ports(),
            &explore::fake::config(config.artifacts_dir.clone()),
            Request {
                phase: Phase::Business,
                issue: 25,
                context: String::new(),
                force: false,
            },
            Gate::empty("tooling"),
        );
        assert_eq!(built.issue_key, "25");
        assert_eq!(built.remaining.get(), 1, "a run is one round");
        assert_eq!(built.round.stages.len(), 9, "the map, then the table");
    }
}
