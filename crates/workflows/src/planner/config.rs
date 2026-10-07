//! What this run changes for a planning round, by value.
//!
//! Separated from the core's `Settings` on purpose: `Settings` carries what
//! **every** workflow has — `--dry-run`, `--stages` — and the core reads it
//! from the `Context`. What is here belongs only to the planner.
//!
//! Also separated from [`crate::planner::ports::Ports`]: this is only
//! values, never an injected capability.

use std::path::PathBuf;

use harness_core::ports::agent::SessionSpec;

/// The settings for a planning round.
pub struct Config {
    /// This roadmap item's lock directory — one planning run at a time.
    pub planner_dir: PathBuf,
    /// Where `business-digest.md`/`technical-digest.md` are read from, if
    /// either exists: `<state_root>/.llocal/grill/<owner>/<name>`.
    pub grill_dir: PathBuf,
    /// This roadmap item's artifacts folder — the repo map the shared
    /// `explore` entries leave behind.
    pub artifacts_dir: PathBuf,
    /// `--explore`: no map, the plan step re-reads the repository itself.
    pub explore: bool,
    /// Model and effort for the plan step.
    pub plan: SessionSpec,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config, disposable folders.

    use std::path::PathBuf;

    use harness_core::ports::agent::SessionSpec;

    use super::Config;

    /// A test config, artifacts under this folder.
    pub fn in_dir(planner_dir: PathBuf) -> Config {
        Config {
            artifacts_dir: planner_dir.join("7"),
            grill_dir: planner_dir.join("grill"),
            planner_dir,
            explore: false,
            plan: SessionSpec {
                model: "sonnet".to_string(),
                effort: "high".to_string(),
            },
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/planner"))
    }
}
