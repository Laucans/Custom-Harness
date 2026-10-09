//! The versions committed on the archive branch: a folder per changed head,
//! the index moved, one commit, one push.

use std::path::Path;

use harness_core::domain::Outcome;

use super::capture::Captured;
use super::said;
use crate::data_snapshot::config::Config;
use crate::data_snapshot::data::index::{self, Head, Index, Snapshot};
use crate::data_snapshot::data::report::Snapped;
use crate::data_snapshot::ports::Ports;
use harness_architecture::model;

/// The index, at the archive's root.
pub const INDEX: &str = "index.json";

/// The dump, in a version's folder.
pub const SCHEMA_FILE: &str = "schema.sql";

/// The model, in a version's folder.
pub const MODEL_FILE: &str = "model.json";

/// Where the two sides of a captured head disagree — nothing to say when one
/// side is missing.
fn divergences(c: &Captured) -> Vec<String> {
    match (&c.schema, &c.model) {
        (Some(schema), Some(declared)) => {
            model::build("", &Ok(Some(schema.clone())), &Ok(Some(declared.clone()))).divergences
        }
        _ => Vec::new(),
    }
}

/// Commits what was captured on the archive branch and pushes it.
///
/// # Errors
///
/// `git` or the disk could not be reached at all; a refusal (a push
/// rejected) is the inner `Err`, and nothing is reported archived.
pub async fn archive(
    ports: &Ports,
    config: &Config,
    clone: &Path,
    captured: &[Captured],
) -> Outcome<Result<Vec<Snapped>, String>> {
    let repo = ports.repos.at(clone);
    let branch = &config.archive_branch;
    // Onto the archive: the pushed one, or a new orphan the first time.
    if repo.origin_has_branch(branch).await? {
        if let Err(why) = said("checkout", &repo.checkout(branch, true).await?) {
            return Ok(Err(why));
        }
        if let Err(why) = said(
            "reset",
            &repo.reset_hard(&format!("origin/{branch}")).await?,
        ) {
            return Ok(Err(why));
        }
    } else if let Err(why) = said("switch --orphan", &repo.create_orphan_branch(branch).await?) {
        return Ok(Err(why));
    }
    if let Err(why) = said("clean", &repo.clean().await?) {
        return Ok(Err(why));
    }

    let read = |rel: &str| ports.disk.read_to_string(&clone.join(rel));
    let mut kept = Index::parse(read(INDEX).as_deref());
    let at = (ports.now)();
    let mut snapped = Vec::new();
    let mut heads: Vec<Head> = Vec::new();
    for c in captured {
        let same = kept.latest(&c.branch).map_or_else(
            || c.schema.is_none() && c.model.is_none(),
            |last| {
                read(&format!("{}/{SCHEMA_FILE}", last.dir)) == c.schema
                    && read(&format!("{}/{MODEL_FILE}", last.dir)) == c.model
            },
        );
        if same {
            snapped.push(Snapped::Unchanged {
                branch: c.branch.clone(),
                sha: c.sha.clone(),
            });
        } else {
            let dir = index::dir_for(&c.branch, &at, &c.sha);
            ports.disk.create_dir_all(&clone.join(&dir))?;
            for (file, text) in [(SCHEMA_FILE, &c.schema), (MODEL_FILE, &c.model)] {
                if let Some(text) = text {
                    ports
                        .disk
                        .write_to_string(&clone.join(format!("{dir}/{file}")), text)?;
                }
            }
            kept.snapshots.push(Snapshot {
                branch: c.branch.clone(),
                sha: c.sha.clone(),
                at: at.clone(),
                dir: dir.clone(),
                schema: c.schema.is_some(),
                model: c.model.is_some(),
            });
            snapped.push(Snapped::Archived {
                branch: c.branch.clone(),
                sha: c.sha.clone(),
                dir,
                compared: c.schema.is_some() && c.model.is_some(),
                divergences: divergences(c),
            });
        }
        kept.heads.insert(c.branch.clone(), c.sha.clone());
        heads.push((c.branch.clone(), c.sha.clone()));
    }
    ports
        .disk
        .write_to_string(&clone.join(INDEX), &kept.to_json())?;
    // One step after the other: a refused commit is never pushed.
    if let Err(why) = said("add", &repo.stage_all().await?) {
        return Ok(Err(why));
    }
    if let Err(why) = said(
        "commit",
        &repo.commit(&index::commit_message(&heads)).await?,
    ) {
        return Ok(Err(why));
    }
    if let Err(why) = said("push", &repo.push(branch).await?) {
        return Ok(Err(why));
    }
    Ok(Ok(snapped))
}
