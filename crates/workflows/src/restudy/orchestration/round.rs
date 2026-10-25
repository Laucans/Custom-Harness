//! A re-study's round: the three steps of the table, nothing prepended.

use harness_core::execution::Round;

use crate::restudy::config::Config;
use crate::restudy::data::state::StudyState;
use crate::restudy::orchestration::stages;
use crate::restudy::ports::Ports;

/// Mounts a re-study's round: context, study, publish.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<StudyState> {
    Round::plain(stages::table(ports, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::restudy::config::fake as config_fake;
    use crate::restudy::ports::fake as ports_fake;

    #[test]
    fn the_round_is_exactly_the_table() {
        let round = build(&ports_fake::ports(), &config_fake::config());
        assert_eq!(round.stages.len(), 3);
        assert!(round.tolerance.is_none());
    }
}
