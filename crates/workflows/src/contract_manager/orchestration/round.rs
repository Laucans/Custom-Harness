//! An evolution's round: the three steps of the table, nothing prepended.

use harness_core::execution::Round;

use crate::contract_manager::config::Config;
use crate::contract_manager::data::state::ManagerState;
use crate::contract_manager::orchestration::stages;
use crate::contract_manager::ports::Ports;

/// Mounts an evolution's round: context, evolve, apply.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<ManagerState> {
    Round::plain(stages::table(ports, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_manager::config::fake as config_fake;
    use crate::contract_manager::ports::fake as ports_fake;

    #[test]
    fn the_round_is_exactly_the_table() {
        let round = build(&ports_fake::ports(), &config_fake::config());
        let names: Vec<&str> = round.stages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, [stages::CONTEXT, stages::EVOLVE, stages::APPLY]);
    }
}
