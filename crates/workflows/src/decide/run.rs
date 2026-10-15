//! Assemble an entire decision from ports and a config already built.

use std::cell::Cell;
use std::rc::Rc;

use harness_core::execution::Gate;

use crate::decide::config::Config;
use crate::decide::data::state::DecideState;
use crate::decide::orchestration::round;
use crate::decide::orchestration::workflow::DecideRun;
use crate::decide::ports::Ports;

/// What an invocation requests.
pub struct Request {
    /// The issue to decide on.
    pub issue: u64,
}

/// Assemble an entire decision from its wiring.
#[must_use]
pub fn build(
    ports: &Ports,
    config: &Config,
    request: &Request,
    pre: Gate<DecideState>,
) -> DecideRun {
    DecideRun {
        pre,
        // One decision per run: a second one is a second label.
        remaining: Cell::new(1),
        gh: Rc::clone(&ports.gh),
        locks: Rc::clone(&ports.locks),
        decide_dir: config.decide_dir.clone(),
        issue: request.issue,
        lock_name: request.issue.to_string(),
        round: round::build(ports, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decide::config::fake as config_fake;
    use crate::decide::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_and_the_round_straight_through() {
        let run = build(
            &ports_fake::ports(),
            &config_fake::config(),
            &Request { issue: 12 },
            Gate::empty("tooling"),
        );
        assert_eq!((run.issue, run.lock_name.as_str()), (12, "12"));
        assert_eq!(run.round.stages.len(), 3);
    }
}
