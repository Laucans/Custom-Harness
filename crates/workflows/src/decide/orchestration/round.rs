//! A decision's round: the three steps of the table, nothing prepended.

use harness_core::execution::Round;

use crate::decide::config::Config;
use crate::decide::data::state::DecideState;
use crate::decide::orchestration::stages;
use crate::decide::ports::Ports;

/// Mounts a decision's round: context, decide, apply.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<DecideState> {
    Round::plain(stages::table(ports, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decide::config::fake as config_fake;
    use crate::decide::ports::fake as ports_fake;

    #[test]
    fn the_round_is_exactly_the_table() {
        let round = build(&ports_fake::ports(), &config_fake::config());
        assert_eq!(round.stages.len(), 3);
        assert!(round.tolerance.is_none());
    }
}
