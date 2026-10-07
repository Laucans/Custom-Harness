//! Assemble an entire repair attempt from ports and a config already built.
//!
//! **Not the construction of concrete adapters** — that remains the
//! launcher's work, the only place with the right to name a `GhCli` or
//! `ClaudeCliFactory`. This module receives filled [`Ports`] and [`Config`]
//! and assembles the form that
//! [`orchestration::workflow`](crate::pr_fix::orchestration::workflow)
//! carries.

use std::cell::Cell;
use std::rc::Rc;

use harness_core::execution::Gate;

use crate::pr_fix::config::Config;
use crate::pr_fix::data::state::FixState;
use crate::pr_fix::orchestration::round;
use crate::pr_fix::orchestration::workflow::FixRun;
use crate::pr_fix::ports::Ports;

/// What an invocation requests — distinct from [`Ports`] and [`Config`],
/// which are infrastructure rather than a request.
pub struct Request {
    /// The PR to repair, number or URL.
    pub pr_ref: String,
}

/// Assemble an entire repair attempt from its wiring.
#[must_use]
pub fn build(ports: &Ports, config: &Config, request: &Request, pre: Gate<FixState>) -> FixRun {
    FixRun {
        pre,
        // One attempt per run: a second one is a second label.
        remaining: Cell::new(1),
        gh: Rc::clone(&ports.gh),
        locks: Rc::clone(&ports.locks),
        fix_dir: config.fix_dir.clone(),
        pr_ref: request.pr_ref.clone(),
        round: round::build(ports, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr_fix::config::fake as config_fake;
    use crate::pr_fix::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_and_the_round_straight_through() {
        let built = build(
            &ports_fake::ports(),
            &config_fake::config(),
            &Request {
                pr_ref: "32".to_string(),
            },
            Gate::empty("outillage"),
        );
        assert_eq!(built.pr_ref, "32");
        assert_eq!(built.remaining.get(), 1, "one attempt per run");
        assert_eq!(built.round.stages.len(), 2, "context then fix");
    }
}
