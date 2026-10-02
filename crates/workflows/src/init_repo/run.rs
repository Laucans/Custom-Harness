//! Assemble one `init-repo` invocation from ports and a config already built.
//!
//! **Not the construction of concrete adapters** — that remains the
//! launcher's work, the only place with the right to name a `GhCli` or a
//! `RealDisk`.
//!
//! **No `Context`, no stage table** (D1): `init-repo` is a deterministic
//! command, not a workflow. `execute()` is an inherent method, not
//! [`harness_core::execution::Executable`] — there is no round, no
//! `Verdict` in the orchestration sense, for it to implement that trait
//! against.

use std::path::PathBuf;
use std::rc::Rc;

use harness_core::adapters::shell::disk::Disk;
use harness_core::adapters::shell::github::GitHub;
use harness_core::domain::{Outcome, Slug};

use crate::init_repo::action::apply;
use crate::init_repo::config::Config;
use crate::init_repo::data::report::{Report, Verdict};
use crate::init_repo::ports::Ports;

/// What an invocation requests.
///
/// Empty on purpose: unlike `pr_review`/`refinement`, every per-invocation
/// value (`slug`, `branch`, `env_file`, the flags) is already [`Config`] —
/// `init-repo` takes no paid decision distinct from its settings. Kept as a
/// type, not dropped, so [`build`] keeps the same shape as the other
/// workflows' `run::build(ports, config, request)`.
pub struct Request;

/// Assemble the command from its wiring. Builds no concrete adapter.
#[must_use]
pub fn build(ports: &Ports, config: &Config, _request: Request) -> InitRepoRun {
    InitRepoRun {
        gh: Rc::clone(&ports.gh),
        disk: Rc::clone(&ports.disk),
        slug: config.slug.clone(),
        branch: config.branch.clone(),
        env_file: config.env_file.clone(),
        write_env: config.write_env,
        force: config.force,
        dry_run: config.dry_run,
    }
}

/// One `init-repo` invocation, wired and ready to run.
pub struct InitRepoRun {
    gh: Rc<dyn GitHub>,
    disk: Rc<dyn Disk>,
    slug: Slug,
    branch: String,
    env_file: PathBuf,
    write_env: bool,
    force: bool,
    dry_run: bool,
}

impl InitRepoRun {
    /// Runs it: preconditions, labels, the branch, the read-only audit, the
    /// link, the report.
    ///
    /// # Errors
    /// A [`harness_core::domain::Halt`] from an unreadable precondition, or
    /// an env-file conflict without `--force`. A remaining BLOCKING audit
    /// line is **not** an error — it comes back as `Ok((report,
    /// Verdict::StillBlocking))`; the caller maps that to its own exit code.
    pub async fn execute(&self) -> Outcome<(Report, Verdict)> {
        let ports = Ports {
            gh: Rc::clone(&self.gh),
            disk: Rc::clone(&self.disk),
        };
        let config = Config {
            slug: self.slug.clone(),
            branch: self.branch.clone(),
            env_file: self.env_file.clone(),
            write_env: self.write_env,
            force: self.force,
            dry_run: self.dry_run,
        };
        apply::run(&ports, &config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_repo::config::fake as config_fake;
    use crate::init_repo::ports::fake as ports_fake;

    #[test]
    fn build_wires_config_straight_through() {
        let built = build(&ports_fake::ports(), &config_fake::config(), Request);
        assert_eq!(built.slug.to_string(), "o/r");
        assert_eq!(built.branch, "main_agent");
        assert!(!built.dry_run);
    }
}
