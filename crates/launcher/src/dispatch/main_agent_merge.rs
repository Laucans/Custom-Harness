//! Launcher wiring for the `main_agent_merge` deterministic command.
//!
//! No checkout mounted at all: every read and write it makes is a `gh` API
//! call (`main_agent_merge::ports::Ports` carries only a `GitHub`), so this
//! is the simplest of the launcher's wiring modules.

use std::path::Path;
use std::rc::Rc;

use harness_core::adapters::shell::github::GhCli;
use harness_core::domain::{Halt, Outcome, Slug};
use harness_core::ports::shell::github::GitHub;
use harness_workflows::main_agent_merge::config::Config;
use harness_workflows::main_agent_merge::data::report;
use harness_workflows::main_agent_merge::ports::Ports;
use harness_workflows::main_agent_merge::run as workflow;

/// Attempts to merge one milestone.
///
/// A `GhCli` built directly from `target_repo_url`/`here`, not through the
/// shared checkout `planner`/`split`/`refinement` use — this command never
/// touches git or the repository's own files.
///
/// # Errors
/// Whatever the command propagates.
pub async fn run(
    milestone: u64,
    here: &Path,
    target_repo_url: &str,
    base_branch: &str,
) -> Outcome<report::Outcome> {
    let gh: Rc<dyn GitHub> = if target_repo_url.is_empty() {
        Rc::new(GhCli::new(here))
    } else {
        let slug = Slug::parse(target_repo_url).ok_or_else(|| {
            Halt::Failed(format!(
                "{target_repo_url:?} is not a usable repository URL"
            ))
        })?;
        Rc::new(GhCli::for_slug(&slug))
    };
    let ports = Ports { gh };
    let config = Config {
        base_branch: base_branch.to_string(),
    };
    workflow::build(&ports, &config, &workflow::Request { milestone })
        .execute()
        .await
}
