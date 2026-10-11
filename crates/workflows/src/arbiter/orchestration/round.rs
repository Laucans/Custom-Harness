//! The round: core's `Round<S>` over the table, nothing more.

use harness_core::execution::Round;

use crate::arbiter::config::Config;
use crate::arbiter::data::state::ArbiterState;
use crate::arbiter::orchestration::stages;
use crate::arbiter::ports::Ports;

/// Mounts the round: exactly the table, no tolerance.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<ArbiterState> {
    Round::plain(stages::table(ports, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arbiter::config::fake as config_fake;
    use crate::arbiter::ports::fake as ports_fake;

    #[test]
    fn the_round_is_exactly_the_table() {
        let round = build(&ports_fake::ports(), &config_fake::config());
        assert_eq!(round.stages.len(), 4);
        assert!(
            round.tolerance.is_none(),
            "an arbitration tolerates nothing"
        );
    }
}
