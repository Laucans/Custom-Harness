//! The wiring of a milestone branch's catch-up with the integration branch:
//! GitHub's merge first; on a refusal, the same merge in a checkout of the
//! harness's own (`catch-up`), where a conflict limited to `Cargo.lock` and
//! the registry's `Cargo.toml` is regenerated (`branching::catch_up_locally`).
//! What it decides is in `harness-workflows`, tested there; this builds a
//! `GitCli` and `Processes` on the mounted checkout and gives it back.

use std::path::Path;
use std::rc::Rc;

use harness_adapters::shell::git::GitCli;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Logbook;
use harness_workflows::common::branching::{self, Checkout};

use crate::adapters::spending;
use crate::dispatch::shared;

/// The checkout a refused catch-up is tried again in: only the harness
/// works in it.
const WORKSPACE_ID: &str = "catch-up";

/// Whether `branch` holds `integration` now: merged on GitHub, or locally
/// once the generated files are regenerated. Nothing local under a dry run.
pub async fn run(
    here: &Path,
    target_repo_url: &str,
    branch: &str,
    integration: &str,
    dry_run: bool,
    gh: &dyn GitHub,
    log: &Logbook,
) -> bool {
    if branching::catch_up(gh, branch, integration, log).await {
        return true;
    }
    if dry_run {
        return false;
    }
    let mount = match shared::mount_named(
        here,
        target_repo_url,
        branch,
        WORKSPACE_ID,
        true,
        &spending::run_id(),
        false,
        log,
    )
    .await
    {
        Ok(mount) => mount,
        Err(halt) => {
            log.warn(&format!(
                "{branch} cannot be checked out to catch up: {}",
                halt.reason()
            ));
            return false;
        }
    };
    let root = mount.workspace.root().to_path_buf();
    let checkout = Checkout {
        repo: Rc::new(GitCli::new(&root)),
        process: Rc::new(crate::toolchain::processes()),
        root,
    };
    let caught_up = branching::catch_up_locally(&checkout, branch, integration, log).await;
    shared::release_checkout(&mount, log).await;
    caught_up
}
