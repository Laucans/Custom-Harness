//! The planner's round: the repo map, then the three steps of the table.
//!
//! A single round — [`PlannerRun`](crate::planner::orchestration::workflow::PlannerRun)
//! carries `remaining = 1`, one round per run — so the generic `Round` of
//! the core serves it as is: nothing branches, and nothing is tolerated.
//!
//! **This is where the map goes in front of the sequence.** `stages::table`
//! has nothing to read the repo with, and has no business having it; the
//! round is what owns the order, so the two shared entries are prepended
//! here.

use harness_core::execution::{Round, Stage};

use crate::common::explore;
use crate::planner::config::Config;
use crate::planner::data::state::PlannerState;
use crate::planner::orchestration::stages;
use crate::planner::ports::Ports;

/// Mounts the planner's round: the map, then context, the plan request, and
/// publishing.
#[must_use]
pub fn build(
    ports: &Ports,
    config: &Config,
    explore_ports: &explore::Ports,
    explore_config: &explore::Config,
) -> Round<PlannerState> {
    let mut sequence: Vec<Stage<PlannerState>> =
        explore::entries::<PlannerState>(explore_ports, explore_config).into();
    sequence.extend(stages::table(ports, config));
    Round::plain(sequence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::config::fake as config_fake;
    use crate::planner::ports::fake as ports_fake;

    #[test]
    fn the_map_comes_first_and_the_table_follows() {
        let config = config_fake::config();
        let round = build(
            &ports_fake::ports(),
            &config,
            &explore::fake::ports(),
            &explore::fake::config(config.artifacts_dir.clone()),
        );
        let names: Vec<&str> = round.stages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(&names[..2], [explore::GROUND, explore::SKILL]);
        assert_eq!(names.len(), 5, "the two entries, then the three steps");
        assert!(round.tolerance.is_none(), "the planner tolerates nothing");
    }
}
