//! Launcher wiring for the `split` workflow.
//!
//! Against the shared read-only checkout (`shared::mount_shared`) like
//! `planner.rs` — `split` never commits either. Unlike `planner`, it does
//! not wire the shared repository map: splitting a milestone into tasks
//! doesn't need it.

use std::path::Path;
use std::rc::Rc;

use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Gate, Guarded, Settings};
use harness_core::ports::agent::SessionSpec;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::split::config::Config;
use harness_workflows::split::data::state::SplitState;
use harness_workflows::split::ports::Ports;
use harness_workflows::split::run;

use crate::adapters::sink::Both;
use crate::adapters::spending;
use crate::adapters::spending::LedgerSpending;
use crate::dispatch::shared;
use harness_core::ports::store::costs::CostHistory;

/// The profile the ledger argues for: one notch finer than `profile` when
/// the tasks cut at that profile have cost more than one session should
/// (`split::config::adjusted`). Said in the log when it moves.
fn profile_from_history(here: &Path, profile: &str, explicit: bool, log: &Logbook) -> String {
    let ledger = LedgerSpending::new(&Workspace::new(here).ledger(), "", "");
    let typical = ledger.typical("", profile).ok().flatten();
    let chosen = harness_workflows::split::config::adjusted(profile, explicit, typical.as_ref());
    if chosen != profile {
        log.say(&format!(
            "split: profile {profile} cut one notch finer, to {chosen} — its tasks' third \
             quartile ran past {:.0} $",
            harness_workflows::split::config::PER_TASK_CEILING
        ));
    }
    chosen
}

/// What tasks of each unit have cost, for the slice prompt; nothing when
/// the ledger cannot be read, said in the log rather than refused — the
/// note is advice to the cut, not a gate.
fn cost_note_from_history(here: &Path, log: &Logbook) -> String {
    let ledger = LedgerSpending::new(&Workspace::new(here).ledger(), "", "");
    match harness_workflows::split::config::cost_note_for(&ledger) {
        Ok(note) => note,
        Err(why) => {
            log.warn(&format!(
                "split: the cost ledger could not be read ({why}) — the slices are cut \
                 without what their kinds cost"
            ));
            String::new()
        }
    }
}

/// Splits one milestone into tasks: mounts the checkout, runs the workflow,
/// unmounts.
///
/// # Errors
/// Whatever the workflow propagates — a precheck refusal, a session
/// failure, a quota exhausted.
pub async fn run(
    milestone: u64,
    here: &Path,
    checkout: &shared::Checkout<'_>,
    permission_mode: &str,
    dry_run: bool,
    profile: &str,
    explicit_profile: bool,
) -> Outcome<Verdict> {
    let run_id = spending::run_id_for(milestone);
    let branch = checkout.branch;
    let sink = Rc::new(
        Both::new(
            &Workspace::new(here)
                .log_dir("split")
                .join(&run_id)
                .join("run.log"),
        )
        .map_err(|e| {
            harness_core::domain::Halt::Failed(format!("log file cannot be opened: {e}"))
        })?,
    );
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);

    let mount = shared::mount_checkout(here, checkout, &run_id, dry_run, &log).await?;
    let workspace = &mount.workspace;
    let built = shared::adapters(
        workspace,
        &run_id,
        permission_mode,
        dry_run,
        &log,
        Some(shared::trace_dir(sink.path())),
    )
    .await;

    let ports = Ports {
        gh: Rc::clone(&built.gh),
        sessions: Rc::clone(&built.sessions),
        spending: Rc::clone(&built.spending),
        locks: Rc::clone(&built.locks),
        disk: Rc::clone(&built.disk),
    };
    let config = Config {
        split_dir: workspace.state_root().join(".llocal/split-locks"),
        base_branch: branch.to_string(),
        slice: SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        },
        root: workspace.root().to_path_buf(),
        prompts: shared::prompts(workspace, &log, &shared::trace_dir(sink.path()))?,
        profile: profile_from_history(here, profile, explicit_profile, &log),
        cost_note: cost_note_from_history(here, &log),
    };

    let workflow = run::build(
        &ports,
        &config,
        &run::Request { milestone },
        Gate::empty("tooling"),
    );
    let mut ctx = Context::new(
        Settings {
            dry_run,
            stages: String::new(),
        },
        SplitState::default(),
        log.clone(),
    );
    let verdict = Guarded::execute(&workflow, &mut ctx).await;
    shared::release_checkout(&mount, &log).await;
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::{Spend, Tags};
    use harness_core::ports::store::spending::{Entry, Spending as _};
    use std::path::PathBuf;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "harness-split-history-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a directory");
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes `count` finished tasks of `unit` at `profile`, each costing `cost`.
    fn history(here: &Path, unit: &str, profile: &str, count: u32, cost: f64) {
        let ledger = LedgerSpending::new(&Workspace::new(here).ledger(), "r", "h");
        for n in 0..count {
            let spend = Spend {
                cost_usd: Some(cost),
                ..Spend::default()
            };
            ledger
                .record(&Entry {
                    round: 1,
                    task: &format!("{unit}-{n}"),
                    stage: "code",
                    spend: &spend,
                    outcome: "ok",
                    fingerprint: "f",
                    tags: &Tags {
                        unit: unit.to_string(),
                        profile: profile.to_string(),
                        side: String::new(),
                        milestone: String::new(),
                    },
                })
                .expect("a row");
        }
    }

    #[test]
    fn a_profile_whose_tasks_ran_dear_is_cut_finer_from_the_ledger() {
        let dir = Dir::new("dear");
        history(&dir.0, "capability", "mid", 3, 30.0);
        assert_eq!(
            profile_from_history(&dir.0, "mid", false, &Logbook::null()),
            "small"
        );
        assert_eq!(
            profile_from_history(&dir.0, "mid", true, &Logbook::null()),
            "mid",
            "an explicit profile stands"
        );
    }

    #[test]
    fn a_profile_with_no_history_or_cheap_tasks_stands() {
        let dir = Dir::new("cheap");
        assert_eq!(
            profile_from_history(&dir.0, "strong", false, &Logbook::null()),
            "strong"
        );
        history(&dir.0, "capability", "strong", 5, 2.0);
        assert_eq!(
            profile_from_history(&dir.0, "strong", false, &Logbook::null()),
            "strong"
        );
    }

    #[test]
    fn the_cost_note_names_the_units_the_ledger_measured() {
        let dir = Dir::new("note");
        assert_eq!(cost_note_from_history(&dir.0, &Logbook::null()), "");
        history(&dir.0, "capability", "mid", 2, 4.0);
        history(&dir.0, "aggregate", "mid", 1, 9.0);
        let note = cost_note_from_history(&dir.0, &Logbook::null());
        assert!(note.contains("- capability: median 4.00 $"), "{note}");
        assert!(note.contains("- aggregate: median 9.00 $"), "{note}");
        assert!(!note.contains("micro-ui"), "{note}");
    }
}
