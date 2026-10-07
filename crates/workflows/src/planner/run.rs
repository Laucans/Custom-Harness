//! Assemble an entire planner run from ports and a config already built.
//!
//! **Not the construction of concrete adapters** — that remains the
//! launcher's work, the only place with the right to name a `GhCli` or
//! `ClaudeCliFactory`. This module receives filled [`Ports`] and [`Config`]
//! — its own and those of the repository map, which is shared — and
//! assembles the form that
//! [`orchestration::workflow`](crate::planner::orchestration::workflow)
//! carries.
//!
//! The map at the head of the sequence is assembled by
//! [`orchestration::round`](crate::planner::orchestration::round): it's the
//! round that carries the order, not the assembly.

use std::cell::Cell;
use std::rc::Rc;

use harness_core::execution::Gate;

use crate::common::explore;
use crate::planner::config::Config;
use crate::planner::data::state::PlannerState;
use crate::planner::orchestration::round;
use crate::planner::orchestration::workflow::PlannerRun;
use crate::planner::ports::Ports;

/// What an invocation requests — distinct from [`Ports`] and [`Config`],
/// which are infrastructure rather than a request.
pub struct Request {
    /// The roadmap item to plan.
    pub roadmap: u64,
}

/// Assemble an entire planner run from its wiring.
#[must_use]
pub fn build(
    ports: &Ports,
    config: &Config,
    explore_ports: &explore::Ports,
    explore_config: &explore::Config,
    request: &Request,
    pre: Gate<PlannerState>,
) -> PlannerRun {
    PlannerRun {
        pre,
        // A run plans one roadmap item, and only one.
        remaining: Cell::new(1),
        gh: Rc::clone(&ports.gh),
        locks: Rc::clone(&ports.locks),
        planner_dir: config.planner_dir.clone(),
        roadmap_key: request.roadmap.to_string(),
        roadmap: request.roadmap,
        round: round::build(ports, config, explore_ports, explore_config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::config::fake as config_fake;
    use crate::planner::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_and_the_round_straight_through() {
        let config = config_fake::config();
        let built = build(
            &ports_fake::ports(),
            &config,
            &explore::fake::ports(),
            &explore::fake::config(config.artifacts_dir.clone()),
            &Request { roadmap: 4 },
            Gate::empty("outillage"),
        );
        assert_eq!(built.roadmap_key, "4");
        assert_eq!(built.remaining.get(), 1, "a run is one round");
        assert_eq!(built.round.stages.len(), 5, "the map, then the table");
    }
}
