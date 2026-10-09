//! Assemble an entire review from ports and a config already built.
//!
//! **Not the construction of concrete adapters** — that remains the launcher's
//! work, the only place with the right to name a `GhCli` or
//! `ClaudeCliFactory`. This module receives filled [`Ports`] and [`Config`],
//! and assembles the form that
//! [`orchestration::workflow`](crate::pr_review::orchestration::workflow)
//! carries.

use std::cell::Cell;

use harness_core::execution::Gate;

use crate::pr_review::config::Config;
use crate::pr_review::data::state::ReviewState;
use crate::pr_review::orchestration::round;
use crate::pr_review::orchestration::workflow::ReviewRun;
use crate::pr_review::ports::Ports;

/// What an invocation requests — distinct from [`Ports`] and [`Config`],
/// which are infrastructure rather than a request.
pub struct Request {
    /// The PR to review, number or URL.
    pub pr_ref: String,
    /// The branch the PR must target to be reviewed.
    pub base: String,
    /// Review even if a rule says to skip.
    pub force: bool,
}

/// Assemble an entire review from its wiring.
#[must_use]
pub fn build(
    ports: &Ports,
    config: &Config,
    request: Request,
    pre: Gate<ReviewState>,
) -> ReviewRun {
    ReviewRun {
        pre,
        // A review is one round, and only one: there's only one PR to review.
        remaining: Cell::new(1),
        gh: std::rc::Rc::clone(&ports.gh),
        locks: std::rc::Rc::clone(&ports.locks),
        review_dir: config.review_dir.clone(),
        pr_ref: request.pr_ref,
        base: request.base,
        force: request.force,
        round: round::build(ports, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr_review::config::fake as config_fake;
    use crate::pr_review::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_and_the_table_straight_through() {
        let built = build(
            &ports_fake::ports(),
            &config_fake::config(),
            Request {
                pr_ref: "32".to_string(),
                base: "main_agent".to_string(),
                force: true,
            },
            Gate::empty("tooling"),
        );
        assert_eq!(built.pr_ref, "32");
        assert_eq!(built.base, "main_agent");
        assert!(built.force);
        assert_eq!(built.remaining.get(), 1, "a review is one round");
        assert_eq!(built.round.stages.len(), 3, "two passes and publishing");
    }
}
