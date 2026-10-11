//! The test debt's round: the two steps of the table, nothing prepended.
//!
//! A single round — [`DebtRun`](crate::test_debt::orchestration::workflow::DebtRun)
//! carries `remaining = 1`, one attempt per run — so the generic `Round` of
//! the core serves it as is: nothing branches, and nothing is tolerated.
//!
//! No shared repository map prepended (`common::explore`): the attempt
//! starts from one named file and the mutants listed against it, not from a
//! question about the whole repo.

use harness_core::execution::Round;

use crate::test_debt::config::Config;
use crate::test_debt::data::state::DebtState;
use crate::test_debt::orchestration::stages;
use crate::test_debt::ports::Ports;

/// Mounts the test debt's round: context, then the attempt.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<DebtState> {
    Round::plain(stages::table(ports, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_debt::config::fake as config_fake;
    use crate::test_debt::ports::fake as ports_fake;

    #[test]
    fn the_round_is_exactly_the_table() {
        let round = build(&ports_fake::ports(), &config_fake::config());
        assert_eq!(round.stages.len(), 2);
        assert!(round.tolerance.is_none(), "an attempt tolerates nothing");
    }
}
