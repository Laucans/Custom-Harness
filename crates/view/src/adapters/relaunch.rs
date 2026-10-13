//! The view started again in another factory's folder, in place.
//!
//! `exec` replaces the process: same pid, same port once the listener is
//! dropped, and the watches the view started — each in a process group of its
//! own — are not touched. The new image finds its factory's watch by looking
//! for one whose working directory is its own (`watch_proc`).

use std::os::unix::process::CommandExt as _;
use std::process::Command;

use harness_core::ports::store::factories::Factory;

use crate::domain::switch;

/// The variable that tells the new view which folder the old one left
/// behind (`Removal::finish`).
pub const LEAVING: &str = "HARNESS_VIEW_LEAVING";

/// Starts the view again in `factory`'s folder, with its repository and
/// branch, and what its `.env.local` sets — the watch command, the harness
/// binary — over what the factory it leaves had. `leaving` is the folder of
/// that factory when it was taken down: the new view deals with it, as the
/// old one cannot remove the folder it runs in. Returns only when that
/// failed — a successful `exec` never comes back.
pub fn into(factory: &Factory, leaving: Option<&str>) -> std::io::Error {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return e,
    };
    let original: Vec<_> = std::env::args_os().skip(1).collect();
    let own = std::fs::read_to_string(factory.root.join(".env.local")).unwrap_or_default();
    Command::new(exe)
        .args(switch::args_for(&original, factory))
        .current_dir(&factory.root)
        .envs(switch::env_of(&own))
        // What a view left is dealt with once: a later switch must not repeat it.
        .env_remove(LEAVING)
        .envs(leaving.map(|spec| (LEAVING, spec)))
        // The children the view starts inherit its environment: a stale
        // repository there would outvote the factory's own.
        .env("TARGET_REPO_URL", &factory.slug)
        .env("INTEGRATION_BRANCH", &factory.integration_branch)
        .exec()
}
