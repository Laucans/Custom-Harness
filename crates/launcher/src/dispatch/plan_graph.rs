//! The wiring of `harness plan-graph <roadmap>`: the shared read-only
//! checkout for the design dossier, `GhCli` for the board and the write.

use std::path::Path;
use std::rc::Rc;

use harness_adapters::shell::github::GhCli;
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome, Slug};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::common::dossier::Dossier;
use harness_workflows::plan_graph;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::cli::PlanGraphArgs;
use crate::dispatch::shared;

/// Writes the graph of one roadmap item's milestones, and prints what it
/// found.
///
/// # Errors
/// A target that is no repository, a checkout that cannot be mounted, a
/// board that cannot be read, a write GitHub refuses.
pub async fn by_hand(args: &PlanGraphArgs, here: &Path) -> Outcome<()> {
    let slug = Slug::parse(&args.target_repo_url).ok_or_else(|| {
        Halt::Failed(format!(
            "{:?} is not a repository the graph can be read from: set TARGET_REPO_URL",
            args.target_repo_url
        ))
    })?;
    let run_id = spending::run_id_for(args.roadmap);
    let journal = Workspace::new(here)
        .log_dir("plan-graph")
        .join(&run_id)
        .join("run.log");
    let sink = Rc::new(
        Both::new(&journal)
            .map_err(|e| Halt::Failed(format!("the journal cannot be opened: {e}")))?,
    );
    let log = Logbook::new(sink as Rc<dyn Sink>, Verbosity::Normal);
    // The dossier is read off the shared checkout, as the planner reads it.
    let mount = shared::mount_shared(
        here,
        &args.target_repo_url,
        &args.branch,
        &run_id,
        false,
        &log,
    )
    .await?;
    let dossier = Dossier::read(
        &harness_adapters::shell::disk::RealDisk,
        mount.workspace.root(),
    );
    let gh: Rc<dyn GitHub> = Rc::new(GhCli::for_slug(&slug));
    let written = plan_graph::write(gh.as_ref(), &dossier, args.roadmap, &args.branch).await?;
    for line in written.lines() {
        log.say(&line);
        println!("{line}");
    }
    Ok(())
}
