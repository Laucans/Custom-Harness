//! Launcher wiring for the `data_snapshot` deterministic command: the
//! product's data model archived, version by version, on the target's
//! `harness/data-model` branch.
//!
//! Reached by the router on every tick that read the board: a quiet pass is
//! one `gh` read per branch, and nothing is cloned.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use harness_adapters::shell::disk::RealDisk;
use harness_adapters::shell::git::GitRepos;
use harness_adapters::shell::process::Processes;
use harness_core::domain::{Outcome, Slug};
use harness_core::ports::shell::github::GitHub;
use harness_workflows::data_snapshot::config::{ARCHIVE_BRANCH, Config, SCRIPT};
use harness_workflows::data_snapshot::data::report::Report;
use harness_workflows::data_snapshot::ports::Ports;
use harness_workflows::data_snapshot::run as workflow;

use crate::adapters::spending;

/// The longest a dump may take: a scratch database, every migration, one
/// `pg_dump`.
const TIMEOUT: Duration = Duration::from_secs(300);

/// Archives the data model of every branch whose head moved.
///
/// # Errors
/// Whatever the command propagates: GitHub, `git` or the disk unreachable.
pub async fn run(
    gh: Rc<dyn GitHub>,
    slug: &Slug,
    here: &Path,
    branches: Vec<String>,
) -> Outcome<Report> {
    let ports = Ports {
        gh,
        repos: Rc::new(GitRepos),
        disk: Rc::new(RealDisk),
        process: Rc::new(Processes),
        now: spending::now,
    };
    let config = Config {
        slug: slug.clone(),
        workdir: here.join(".llocal").join("data-snapshot"),
        archive_branch: ARCHIVE_BRANCH.to_string(),
        script: SCRIPT.to_string(),
        timeout: TIMEOUT,
    };
    workflow::build(&ports, &config, workflow::Request { branches })
        .run()
        .await
}
