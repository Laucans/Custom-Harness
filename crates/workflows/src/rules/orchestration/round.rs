//! The rules' round: the three steps of the table, nothing prepended.
//!
//! A single round — one attempt per run — so the core's generic `Round`
//! serves it as is: nothing branches, nothing is tolerated.

use harness_core::execution::Round;

use crate::rules::config::Config;
use crate::rules::data::state::RulesState;
use crate::rules::orchestration::stages;
use crate::rules::ports::Ports;

/// Mounts the rules' round: context, write, fidelity.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<RulesState> {
    Round::plain(stages::table(ports, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::config::fake as config_fake;
    use crate::rules::ports::fake as ports_fake;

    #[test]
    fn the_round_is_exactly_the_table() {
        let round = build(&ports_fake::ports(), &config_fake::config());
        assert_eq!(round.stages.len(), 3);
        assert!(round.tolerance.is_none(), "an attempt tolerates nothing");
    }
}
