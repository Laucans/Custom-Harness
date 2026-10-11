//! Assemble an entire test-debt attempt from ports and a config already
//! built.
//!
//! **Not the construction of concrete adapters** — that remains the
//! launcher's work, the only place with the right to name a `GhCli` or
//! `ClaudeCliFactory`. This module receives filled [`Ports`] and [`Config`]
//! and assembles the form that
//! [`orchestration::workflow`](crate::test_debt::orchestration::workflow)
//! carries.

use std::cell::Cell;
use std::rc::Rc;

use harness_core::execution::Gate;

use crate::test_debt::config::Config;
use crate::test_debt::data::state::DebtState;
use crate::test_debt::orchestration::round;
use crate::test_debt::orchestration::workflow::DebtRun;
use crate::test_debt::ports::Ports;

/// What an invocation requests — distinct from [`Ports`] and [`Config`],
/// which are infrastructure rather than a request.
pub struct Request {
    /// The test-debt issue's number.
    pub issue: u64,
}

/// Assemble an entire test-debt attempt from its wiring.
#[must_use]
pub fn build(ports: &Ports, config: &Config, request: &Request, pre: Gate<DebtState>) -> DebtRun {
    DebtRun {
        pre,
        // One attempt per run: a second one is a second label.
        remaining: Cell::new(1),
        gh: Rc::clone(&ports.gh),
        locks: Rc::clone(&ports.locks),
        debt_dir: config.debt_dir.clone(),
        issue: request.issue,
        lock_name: request.issue.to_string(),
        round: round::build(ports, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_debt::config::fake as config_fake;
    use crate::test_debt::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_and_the_round_straight_through() {
        let built = build(
            &ports_fake::ports(),
            &config_fake::config(),
            &Request { issue: 7 },
            Gate::empty("tooling"),
        );
        assert_eq!(built.issue, 7);
        assert_eq!(built.lock_name, "7");
        assert_eq!(built.remaining.get(), 1, "one attempt per run");
        assert_eq!(built.round.stages.len(), 2, "context then cover");
    }
}
