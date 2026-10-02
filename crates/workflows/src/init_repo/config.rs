//! What this run of `init-repo` changes, by value.
//!
//! Separated from [`crate::init_repo::ports::Ports`]: this is only values,
//! never an injected capability.

use std::path::PathBuf;

use harness_core::domain::Slug;

/// The settings for one `init-repo` invocation.
pub struct Config {
    /// The target repository.
    pub slug: Slug,
    /// The integration branch to create and to audit.
    pub branch: String,
    /// Where the link (`TARGET_REPO_URL`, `INTEGRATION_BRANCH`) is written.
    pub env_file: PathBuf,
    /// Write the env file at all — `false` under `--no-env`.
    pub write_env: bool,
    /// Overwrite a `TARGET_REPO_URL` already set to another repo.
    pub force: bool,
    /// Read everything, write nothing.
    pub dry_run: bool,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config: `o/r`, branch `main_agent`, a throwaway env file.

    use std::path::PathBuf;

    use harness_core::domain::Slug;

    use super::Config;

    /// A test config.
    pub fn config() -> Config {
        Config {
            slug: Slug {
                owner: "o".to_string(),
                name: "r".to_string(),
            },
            branch: "main_agent".to_string(),
            env_file: PathBuf::from("/tmp/.env.local"),
            write_env: true,
            force: false,
            dry_run: false,
        }
    }
}
