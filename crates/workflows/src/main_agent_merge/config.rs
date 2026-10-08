//! What a merge attempt needs to know, by value.

/// The settings for a merge attempt.
pub struct Config {
    /// The branch a milestone's PR targets — `main_agent`.
    pub base_branch: String,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config: `main_agent`.

    use super::Config;

    /// A test config.
    pub fn config() -> Config {
        Config {
            base_branch: "main_agent".to_string(),
        }
    }
}
