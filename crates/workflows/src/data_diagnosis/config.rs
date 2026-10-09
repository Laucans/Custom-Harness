//! What a diagnosis is worth, by value.

use harness_core::ports::agent::SessionSpec;

/// The settings for a diagnosis.
pub struct Config {
    /// The model and effort the session opens with.
    pub spec: SessionSpec,
}
