//! The repair's round: the two steps of the table, nothing prepended.
//!
//! A single round — [`FixRun`](crate::pr_fix::orchestration::workflow::FixRun)
//! carries `remaining = 1`, one attempt per run — so the generic `Round` of
//! the core serves it as is: nothing branches, and nothing is tolerated.
//!
//! No shared repository map prepended (`common::explore`), unlike
//! `planner`: a repair starts from a named failure in a known diff, not from
//! a question about the whole repo.

use harness_core::execution::Round;

use crate::pr_fix::config::Config;
use crate::pr_fix::data::state::FixState;
use crate::pr_fix::orchestration::stages;
use crate::pr_fix::ports::Ports;

/// Mounts the repair's round: context, then the attempt.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<FixState> {
    Round::plain(stages::table(ports, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr_fix::config::fake as config_fake;
    use crate::pr_fix::ports::fake as ports_fake;

    #[test]
    fn the_round_is_exactly_the_table() {
        let round = build(&ports_fake::ports(), &config_fake::config());
        assert_eq!(round.stages.len(), 2);
        assert!(round.tolerance.is_none(), "a repair tolerates nothing");
    }
}
