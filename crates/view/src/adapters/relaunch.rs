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

/// Starts the view again in `factory`'s folder, with its repository and
/// branch. Returns only when that failed — a successful `exec` never comes back.
pub fn into(factory: &Factory) -> std::io::Error {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return e,
    };
    let original: Vec<_> = std::env::args_os().skip(1).collect();
    Command::new(exe)
        .args(switch::args_for(&original, factory))
        .current_dir(&factory.root)
        // The children the view starts inherit its environment: a stale
        // repository there would outvote the factory's own.
        .env("TARGET_REPO_URL", &factory.slug)
        .env("INTEGRATION_BRANCH", &factory.integration_branch)
        .exec()
}
