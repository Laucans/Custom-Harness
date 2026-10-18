//! Assemble an entire evolution from ports and a config already built.

use std::cell::Cell;
use std::rc::Rc;

use harness_core::execution::Gate;

use crate::contract_manager::config::Config;
use crate::contract_manager::data::state::ManagerState;
use crate::contract_manager::orchestration::round;
use crate::contract_manager::orchestration::workflow::ManagerRun;
use crate::contract_manager::ports::Ports;

/// What an invocation requests.
pub struct Request {
    /// The `harness:contract-change` issue to answer.
    pub issue: u64,
}

/// Assemble an entire evolution from its wiring.
#[must_use]
pub fn build(
    ports: &Ports,
    config: &Config,
    request: &Request,
    pre: Gate<ManagerState>,
) -> ManagerRun {
    ManagerRun {
        pre,
        // One evolution per run: a second one is a second request.
        remaining: Cell::new(1),
        gh: Rc::clone(&ports.gh),
        locks: Rc::clone(&ports.locks),
        lock_dir: config.lock_dir.clone(),
        issue: request.issue,
        lock_name: request.issue.to_string(),
        round: round::build(ports, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_manager::config::fake as config_fake;
    use crate::contract_manager::ports::fake as ports_fake;

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
