//! The diagnosis's round: its one stage, nothing forgiven.

use harness_core::execution::Round;

use crate::data_diagnosis::config::Config;
use crate::data_diagnosis::data::state::DiagnosisState;
use crate::data_diagnosis::orchestration::stages;
use crate::data_diagnosis::ports::Ports;

/// Mounts the round.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<DiagnosisState> {
    Round::plain(stages::table(ports, config))
}
