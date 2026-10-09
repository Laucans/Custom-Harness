//! The draft clone brought to the base's head, the schema dumped, the draft
//! branch made — reusing the snapshot's own clone and dump.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};

use super::said;
use crate::data_model_draft::config::Config;
use crate::data_model_draft::data::state::{DraftState, draft_branch, dump_name};
use crate::data_model_draft::ports::Ports;
use crate::data_snapshot::action::capture::{capture, mount};
use crate::data_snapshot::config::{ARCHIVE_BRANCH, Config as SnapshotConfig};
use crate::data_snapshot::ports::Ports as SnapshotPorts;

/// Prepares the draft.
pub struct Prepare {
    /// The snapshot's ports, over the draft's.
    pub ports: SnapshotPorts,
    /// The snapshot's settings, over the draft's.
    pub config: SnapshotConfig,
    /// The base branch.
    pub base: String,
}

impl Prepare {
    /// Built from the draft's own ports and settings.
    #[must_use]
    pub fn new(ports: &Ports, config: &Config) -> Self {
        Self {
            ports: SnapshotPorts {
                gh: Rc::clone(&ports.gh),
                repos: Rc::clone(&ports.repos),
                disk: Rc::clone(&ports.disk),
                process: Rc::clone(&ports.process),
                now: ports.now,
            },
            config: SnapshotConfig {
                slug: config.slug.clone(),
                workdir: config.workdir.clone(),
                archive_branch: ARCHIVE_BRANCH.to_string(),
                script: config.script.clone(),
                timeout: config.timeout,
            },
            base: config.base.clone(),
        }
    }
}

#[async_trait(?Send)]
impl Action<DraftState> for Prepare {
    async fn run(&self, ctx: &mut Context<DraftState>) -> Outcome<Verdict> {
        let sha = self
            .ports
            .gh
            .branch_sha(&self.base)
            .await?
            .ok_or_else(|| Halt::Halted(format!("{} has no head on GitHub", self.base)))?;
        let clone = mount(&self.ports, &self.config)
            .await?
            .map_err(Halt::Failed)?;
        let captured = capture(
            &self.ports,
            &self.config,
            &clone,
            &(self.base.clone(), sha.clone()),
        )
        .await?
        .map_err(Halt::Failed)?;
        if captured.model.is_some() {
            return Err(Halt::Halted(format!(
                "{} already has a data/model.json: nothing to draft",
                self.base
            )));
        }
        let schema = captured.schema.ok_or_else(|| {
            Halt::Halted(format!(
                "the dump of {} printed nothing: no migrations to describe yet",
                self.base
            ))
        })?;
        let schema_path = self.config.workdir.join(dump_name(&self.base));
        self.ports.disk.write_to_string(&schema_path, &schema)?;
        // The draft branch starts from the head just dumped; a former
        // attempt's branch is moved there.
        let branch = draft_branch(&self.base);
        let repo = self.ports.repos.at(&clone);
        if !repo.create_local_branch(&branch, &sha).await?.ok() {
            said("checkout", &repo.checkout(&branch, true).await?)?;
            said("reset", &repo.reset_hard(&sha).await?)?;
        }
        ctx.traces.say(&format!(
            "draft: {} @ {} dumped ({} lines), drafting on {branch}",
            self.base,
            sha.chars().take(7).collect::<String>(),
            schema.lines().count()
        ));
        ctx.state = DraftState {
            clone,
            sha,
            branch,
            schema_path,
            schema,
            pr: None,
            divergences: Vec::new(),
        };
        Ok(Verdict::Continue)
    }
}
