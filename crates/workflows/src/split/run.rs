//! Assemble an entire split run from ports and a config already built.
//!
//! **Not the construction of concrete adapters** — that remains the
//! launcher's work, the only place with the right to name a `GhCli` or
//! `ClaudeCliFactory`. This module receives filled [`Ports`] and [`Config`]
//! and assembles the form that
//! [`orchestration::workflow`](crate::split::orchestration::workflow)
//! carries.

use std::cell::Cell;
use std::rc::Rc;

use harness_core::execution::Gate;

use crate::split::config::Config;
use crate::split::data::state::SplitState;
use crate::split::orchestration::round;
use crate::split::orchestration::workflow::SplitRun;
use crate::split::ports::Ports;

/// What an invocation requests — distinct from [`Ports`] and [`Config`],
/// which are infrastructure rather than a request.
pub struct Request {
    /// The milestone to split.
    pub milestone: u64,
}

/// Assemble an entire split run from its wiring.
#[must_use]
pub fn build(ports: &Ports, config: &Config, request: &Request, pre: Gate<SplitState>) -> SplitRun {
    SplitRun {
        pre,
        // A run splits one milestone, and only one.
        remaining: Cell::new(1),
        gh: Rc::clone(&ports.gh),
        locks: Rc::clone(&ports.locks),
        split_dir: config.split_dir.clone(),
        milestone_key: request.milestone.to_string(),
        milestone: request.milestone,
        round: round::build(ports, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::config::fake as config_fake;
    use crate::split::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_and_the_round_straight_through() {
        let built = build(
            &ports_fake::ports(),
            &config_fake::config(),
            &Request { milestone: 4 },
            Gate::empty("outillage"),
        );
        assert_eq!(built.milestone_key, "4");
        assert_eq!(built.remaining.get(), 1, "a run is one round");
        assert_eq!(built.round.stages.len(), 3);
    }
}
