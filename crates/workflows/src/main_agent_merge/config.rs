//! What a merge attempt needs to know, by value.

/// The settings for a merge attempt.
pub struct Config {
    /// The branch a milestone's PR targets — `main_agent`.
    pub base_branch: String,
    /// The PRs a lane repairs right now, by number — the lane holds the
    /// repair's lock: a landing PR whose repair still runs is not escalated
    /// (its last repair's marker is posted before its session, while the
    /// checks are still red from the push before).
    pub repairing: Vec<u64>,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config: `main_agent`.

    use super::Config;

    /// A test config.
    pub fn config() -> Config {
        Config {
            base_branch: "main_agent".to_string(),
            repairing: Vec::new(),
        }
    }
}
