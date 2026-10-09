//! The draft's round: its three stages, nothing forgiven.

use harness_core::execution::Round;

use crate::data_model_draft::config::Config;
use crate::data_model_draft::data::state::DraftState;
use crate::data_model_draft::orchestration::stages;
use crate::data_model_draft::ports::Ports;

/// Mounts the round.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<DraftState> {
    Round::plain(stages::table(ports, config))
}
