//! Assemble an arbitration from ports and a config already built.
//!
//! Not the construction of concrete adapters — that remains the launcher's
//! work. This receives filled [`Ports`] and [`Config`] and assembles the
//! form [`orchestration::workflow`](crate::arbiter::orchestration::workflow)
//! carries.

use std::cell::Cell;
use std::rc::Rc;

use harness_core::execution::Gate;

use crate::arbiter::config::Config;
use crate::arbiter::data::state::ArbiterState;
use crate::arbiter::orchestration::round;
use crate::arbiter::orchestration::workflow::ArbiterRun;
use crate::arbiter::ports::Ports;

/// What an invocation requests.
pub struct Request {
    /// The task to arbitrate.
    pub issue: u64,
    /// Arbitrate even if the task does not carry `harness:needs-ai-decision`.
    pub force: bool,
}

/// Assemble an arbitration from its wiring.
#[must_use]
pub fn build(
    ports: &Ports,
    config: &Config,
    request: &Request,
    pre: Gate<ArbiterState>,
) -> ArbiterRun {
    ArbiterRun {
        pre,
        // One arbitration per run: a second one is a second label.
        remaining: Cell::new(1),
        gh: Rc::clone(&ports.gh),
        locks: Rc::clone(&ports.locks),
        arbiter_dir: config.arbiter_dir.clone(),
        issue: request.issue,
        lock_name: request.issue.to_string(),
        force: request.force,
        round: round::build(ports, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arbiter::config::fake as config_fake;
    use crate::arbiter::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_and_the_round_straight_through() {
        let built = build(
            &ports_fake::ports(),
            &config_fake::config(),
            &Request {
                issue: 7,
                force: true,
            },
            Gate::empty("tooling"),
        );
        assert_eq!(built.issue, 7);
        assert_eq!(built.lock_name, "7");
        assert!(built.force);
        assert_eq!(built.remaining.get(), 1, "one arbitration per run");
        assert_eq!(built.round.stages.len(), 4);
    }
}
