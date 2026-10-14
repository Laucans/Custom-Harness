//! Assemble an entire rules' task from ports and a config already built.
//!
//! **Not the construction of concrete adapters** — that remains the
//! launcher's work.

use std::cell::Cell;
use std::rc::Rc;

use harness_core::execution::Gate;

use crate::rules::config::Config;
use crate::rules::data::state::RulesState;
use crate::rules::orchestration::round;
use crate::rules::orchestration::workflow::RulesRun;
use crate::rules::ports::Ports;

/// What an invocation requests.
pub struct Request {
    /// The rules' issue number.
    pub issue: u64,
}

/// Assemble an entire rules' task from its wiring.
#[must_use]
pub fn build(ports: &Ports, config: &Config, request: &Request, pre: Gate<RulesState>) -> RulesRun {
    RulesRun {
        pre,
        // One attempt per run: a second one is a second `harness:ready`.
        remaining: Cell::new(1),
        gh: Rc::clone(&ports.gh),
        locks: Rc::clone(&ports.locks),
        rules_dir: config.rules_dir.clone(),
        issue: request.issue,
        lock_name: request.issue.to_string(),
        round: round::build(ports, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::config::fake as config_fake;
    use crate::rules::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_and_the_round_straight_through() {
        let run = build(
            &ports_fake::ports(),
            &config_fake::config(),
            &Request { issue: 12 },
            Gate::empty("tooling"),
        );
        assert_eq!(run.issue, 12);
        assert_eq!(run.lock_name, "12");
        assert_eq!(run.rules_dir, config_fake::config().rules_dir);
        assert_eq!(run.round.stages.len(), 3);
    }
}
