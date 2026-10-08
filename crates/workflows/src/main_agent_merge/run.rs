//! Assembles one merge attempt from ports and a config already built.
//!
//! **Not the construction of concrete adapters** — that stays the
//! launcher's work. Mirrors `init_repo::run`: no `Context`, no stage table,
//! since this is a deterministic command, not a workflow (see `mod.rs`).

use std::rc::Rc;

use harness_core::domain::Outcome;
use harness_core::ports::shell::github::GitHub;

use crate::main_agent_merge::action::apply;
use crate::main_agent_merge::config::Config;
use crate::main_agent_merge::data::report;
use crate::main_agent_merge::ports::Ports;

/// What an invocation requests.
pub struct Request {
    /// The milestone to attempt merging.
    pub milestone: u64,
}

/// One merge attempt, assembled from its wiring.
pub struct MilestoneMergeRun {
    gh: Rc<dyn GitHub>,
    base_branch: String,
    milestone: u64,
}

/// Assembles one merge attempt.
#[must_use]
pub fn build(ports: &Ports, config: &Config, request: &Request) -> MilestoneMergeRun {
    MilestoneMergeRun {
        gh: Rc::clone(&ports.gh),
        base_branch: config.base_branch.clone(),
        milestone: request.milestone,
    }
}

impl MilestoneMergeRun {
    /// Runs the attempt.
    ///
    /// # Errors
    /// Whatever [`apply::run`] propagates.
    pub async fn execute(&self) -> Outcome<report::Outcome> {
        apply::run(
            &Ports {
                gh: Rc::clone(&self.gh),
            },
            &Config {
                base_branch: self.base_branch.clone(),
            },
            self.milestone,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::main_agent_merge::config::fake as config_fake;
    use crate::main_agent_merge::ports::fake as ports_fake;

    #[test]
    fn build_wires_the_request_straight_through() {
        let built = build(
            &ports_fake::ports(),
            &config_fake::config(),
            &Request { milestone: 4 },
        );
        assert_eq!(built.milestone, 4);
        assert_eq!(built.base_branch, "main_agent");
    }
}
