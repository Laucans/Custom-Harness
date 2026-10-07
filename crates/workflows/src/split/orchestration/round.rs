//! Split's round: the three steps of the table, nothing prepended.
//!
//! A single round — [`SplitRun`](crate::split::orchestration::workflow::SplitRun)
//! carries `remaining = 1`, one round per run — so the generic `Round` of
//! the core serves it as is: nothing branches, and nothing is tolerated.
//!
//! Unlike `planner`, there is no shared repository map prepended here: see
//! the workflow's own module doc for why.

use harness_core::execution::Round;

use crate::split::config::Config;
use crate::split::data::state::SplitState;
use crate::split::orchestration::stages;
use crate::split::ports::Ports;

/// Mounts split's round: context, the slice request, then publishing.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<SplitState> {
    Round::plain(stages::table(ports, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::config::fake as config_fake;
    use crate::split::ports::fake as ports_fake;

    #[test]
    fn the_round_is_exactly_the_table() {
        let round = build(&ports_fake::ports(), &config_fake::config());
        assert_eq!(round.stages.len(), 3);
        assert!(round.tolerance.is_none(), "split tolerates nothing");
    }
}
