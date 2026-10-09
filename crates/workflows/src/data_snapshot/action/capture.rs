//! One head read: the snapshot clone brought to the commit, the target's dump
//! script run on it, the agents' model read beside.

use std::path::{Path, PathBuf};

use harness_core::domain::Outcome;

use super::said;
use crate::data_snapshot::config::Config;
use crate::data_snapshot::data::index::Head;
use crate::data_snapshot::ports::Ports;

/// The script the harness falls back on when the target keeps none.
pub const DEFAULT_SCRIPT: &str = include_str!("../assets/dump-schema.sh");

/// The agents' model, relative to the target's root.
pub const MODEL: &str = "data/model.json";

/// The clone's own working branch: moved to every head read, never pushed.
const WORK: &str = "harness-data-snapshot";

/// What one head holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// The branch.
    pub branch: String,
    /// The commit.
    pub sha: String,
    /// The dump; `None` when the script printed nothing (no database yet).
    pub schema: Option<String>,
    /// `data/model.json` at that commit, when there is one.
    pub model: Option<String>,
}

/// Where the snapshot clone lives: one folder per repository.
#[must_use]
pub fn clone_dir(config: &Config) -> PathBuf {
    config
        .workdir
        .join(format!("{}-{}", config.slug.owner, config.slug.name))
}

/// The snapshot clone, cloned the first time and fetched after.
///
/// # Errors
///
/// `git` could not run, or refused to clone or fetch.
pub async fn mount(ports: &Ports, config: &Config) -> Outcome<Result<PathBuf, String>> {
    let clone = clone_dir(config);
    if ports.disk.exists(&clone) {
        if let Err(why) = said("fetch", &ports.repos.at(&clone).fetch().await?) {
            return Ok(Err(why));
        }
        return Ok(Ok(clone));
    }
    ports.disk.create_dir_all(&config.workdir)?;
    let name = clone
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let url = format!(
        "https://github.com/{}/{}.git",
        config.slug.owner, config.slug.name
    );
    let ran = ports
        .repos
        .at(&config.workdir)
        .clone_repo(&url, &name)
        .await?;
    Ok(said("clone", &ran).map(|()| clone))
}

/// Brings the clone to `head` and reads its data model.
///
/// # Errors
///
/// `git` or the script could not run at all; a refusal of either is the
/// inner `Err`, a sentence.
pub async fn capture(
    ports: &Ports,
    config: &Config,
    clone: &Path,
    head: &Head,
) -> Outcome<Result<Captured, String>> {
    let (branch, sha) = head;
    let repo = ports.repos.at(clone);
    // Onto the clone's own branch first: a reset on the archive branch would
    // move the archive.
    if !repo.checkout(WORK, true).await?.ok()
        && let Err(why) = said("switch", &repo.create_local_branch(WORK, sha).await?)
    {
        return Ok(Err(why));
    }
    if let Err(why) = said("reset", &repo.reset_hard(sha).await?) {
        return Ok(Err(why));
    }
    if let Err(why) = said("clean", &repo.clean().await?) {
        return Ok(Err(why));
    }
    let own = clone.join(&config.script);
    let script = if ports.disk.exists(&own) {
        own
    } else {
        let fallback = config.workdir.join("dump-schema.sh");
        ports.disk.write_to_string(&fallback, DEFAULT_SCRIPT)?;
        fallback
    };
    let ran = ports
        .process
        .run(
            "bash",
            &[script.display().to_string()],
            clone,
            config.timeout,
        )
        .await?;
    if !ran.ok() {
        return Ok(Err(format!(
            "{} exited {}: {}",
            config.script,
            ran.code
                .map_or_else(|| "on a signal".to_string(), |c| c.to_string()),
            ran.why()
        )));
    }
    Ok(Ok(Captured {
        branch: branch.clone(),
        sha: sha.clone(),
        schema: Some(ran.stdout).filter(|s| !s.trim().is_empty()),
        model: ports.disk.read_to_string(&clone.join(MODEL)),
    }))
}
