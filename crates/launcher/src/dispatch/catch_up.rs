//! The wiring of a milestone branch's catch-up with the integration branch
//! (`Route::CatchUp`, run in the tick): GitHub's merge of the head the watch
//! read first — unless the merge stales the mock-up's bundles, then
//! straight — and on a refusal, the same merge in a checkout of
//! the harness's own (`catch-up`), its generated files regenerated
//! (`branching::catch_up_locally`) with the cargo the **merged** tree pins
//! (`crate::toolchain::by_the_tree`: rustup's proxies ahead, so the merge's
//! `rust-toolchain.toml` decides, never the branch's older pin nor
//! Homebrew's cargo). What it decides — when, how often, what a refusal or a
//! residual conflict becomes — is `common::catch_up` in
//! `harness-workflows`, tested there; this builds a `GitCli`, `Processes`
//! and `RealDisk` on the mounted checkout and hands them over, and reports
//! the outcome under the watch's `catch_up` tag.

use std::path::Path;
use std::rc::Rc;

use harness_adapters::shell::disk::RealDisk;
use harness_adapters::shell::git::GitCli;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Logbook;
use harness_workflows::common::branching::{self, BaseHead, CaughtUp, Checkout};
use harness_workflows::common::catch_up::{self, Busy, CatchUps, Target};

use crate::adapters::spending;
use crate::dispatch::shared;

/// The checkout a refused catch-up is tried again in: only the harness
/// works in it.
const WORKSPACE_ID: &str = "catch-up";

/// Brings `milestone`'s branch up to `integration`'s head, remembered in
/// `catch_ups`: `common::catch_up::catch_up`, with the harness's checkout
/// as its local merge, told as `watch: catch_up -> …`. `busy`: what lanes
/// work on, whose PRs are left alone.
#[allow(clippy::too_many_arguments)] // the tick's state, handed as it is
pub async fn milestone(
    here: &Path,
    target_repo_url: &str,
    integration: &str,
    dry_run: bool,
    gh: &dyn GitHub,
    (milestone, branch): (u64, &str),
    catch_ups: &mut CatchUps,
    busy: &Busy,
    log: &Logbook,
) -> CaughtUp {
    let target = Target {
        milestone,
        branch,
        integration,
    };
    let caught = catch_up::catch_up(
        gh,
        target,
        catch_ups,
        busy,
        dry_run,
        crate::dispatch::dev_loop::now_seconds(),
        async |base: BaseHead| {
            locally(here, target_repo_url, (branch, integration), &base, log).await
        },
        log,
    )
    .await;
    let (line, warned) = catch_up::told(milestone, &caught);
    if warned {
        log.warn(&format!("watch: catch_up -> {line}"));
    } else {
        log.say(&format!("watch: catch_up -> {line}"));
    }
    caught
}

/// The merge of `base` (a commit of `integration`) into `branch` in the
/// harness's own checkout, mounted for it and released: a milestone's
/// catch-up, or a task PR the merge train refreshes before it lands
/// (`milestone_merge::Attempt::Stale`).
pub async fn locally(
    here: &Path,
    target_repo_url: &str,
    (branch, integration): (&str, &str),
    base: &BaseHead,
    log: &Logbook,
) -> CaughtUp {
    let mount = match shared::mount_bare(
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
            return CaughtUp::Failed(format!(
                "{branch} cannot be checked out to catch up: {}",
                halt.reason()
            ));
        }
    };
    let root = mount.workspace.root().to_path_buf();
    let checkout = Checkout {
        repo: Rc::new(GitCli::new(&root)),
        process: Rc::new(crate::toolchain::by_the_tree(here).await),
        disk: Rc::new(RealDisk),
        root,
    };
    let caught = branching::catch_up_locally(&checkout, branch, integration, base, log).await;
    shared::release_checkout(&mount, log).await;
    caught
}
