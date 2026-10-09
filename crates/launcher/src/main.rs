#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// Same reason as at the root of `harness-core`: decision #3 chooses `?Send`
// everywhere, and `clippy::future_not_send` presupposes the opposite.
#![allow(clippy::future_not_send)]

//! The entry point: read the arguments, find the repository, run.
//!
//! The exit code **is a contract**. An external scheduler reads it, and the
//! four values are: `0` everything is fine, `1` a voluntary halt or an
//! unreadable store, `2` a failure, `3` a quota exhausted. Moving them would
//! be a hidden contract change dressed up as refactoring.

mod adapters;
mod cli;
mod dispatch;
mod router;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use harness_core::domain::{Severity, Verdict};

/// Runs the loop and returns the code that the scheduler reads.
///
/// `current_thread`: the harness drives one session at a time, and decision
/// #3 chooses `?Send` everywhere — a multi-thread scheduler would have
/// nothing to schedule.
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = cli::Cli::parse();
    let here = match repo_root() {
        Ok(path) => path,
        Err(why) => {
            eprintln!("{why}");
            return ExitCode::from(2);
        }
    };
    match &args.command {
        Some(cli::Command::InitRepo(sub)) => dispatch::init_repo::run(sub, &here).await,
        Some(cli::Command::Watch(sub)) => match router::run(sub, &here).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(halt) => halt_to_code(&halt),
        },
        Some(cli::Command::Doctor(sub)) => {
            let log = harness_core::traces::Logbook::new(
                std::rc::Rc::new(adapters::sink::Console)
                    as std::rc::Rc<dyn harness_core::traces::Sink>,
                harness_core::traces::Verbosity::Normal,
            );
            match dispatch::doctor::treat_by_hand(&here, &sub.workspaces_dir, sub.dry_run, &log)
                .await
            {
                Ok(repair) => {
                    println!("doctor: {}", repair.outcome());
                    ExitCode::SUCCESS
                }
                Err(halt) => halt_to_code(&halt),
            }
        }
        Some(cli::Command::Refine(sub)) => {
            let ran = dispatch::refinement::run(
                harness_workflows::refinement::data::phase::Phase::Business,
                sub.issue,
                &here,
                &checkout_of(&sub.lane),
                &sub.lane.permission_mode,
                sub.lane.dry_run,
            )
            .await;
            lane_exit(&here, "refine", sub.issue, ran)
        }
        Some(cli::Command::Split(sub)) => {
            let ran = dispatch::split::run(
                sub.milestone,
                &here,
                &checkout_of(&sub.lane),
                &sub.lane.permission_mode,
                sub.lane.dry_run,
            )
            .await;
            lane_exit(&here, "split", sub.milestone, ran)
        }
        None => match dispatch::dev_loop::run(&args.run, &here).await {
            Ok(ran) => {
                if let Verdict::NothingLeft(why) = &ran.verdict {
                    println!("{why}");
                }
                println!("journal : {}", ran.log.display());
                ExitCode::SUCCESS
            }
            Err(halt) => {
                if args.run.lane {
                    let task = args.run.task.map_or_else(String::new, |n| format!(" #{n}"));
                    tell_halt(&here, &format!("dev_loop{task}"), &halt);
                }
                halt_to_code(&halt)
            }
        },
    }
}

/// The prefix and level are those that the scheduler already reads.
fn halt_to_code(halt: &harness_core::domain::Halt) -> ExitCode {
    let line = format!("{}: {}", halt.prefix(), halt.reason());
    match halt.severity() {
        Severity::Info => println!("{line}"),
        Severity::Warn | Severity::Error => eprintln!("{line}"),
    }
    ExitCode::from(u8::try_from(halt.exit_code()).unwrap_or(2))
}

/// The repository from which the run is launched.
///
/// By walking up from the current directory, **without a subprocess**.
fn repo_root() -> Result<PathBuf, String> {
    let here =
        std::env::current_dir().map_err(|e| format!("cannot read the current directory: {e}"))?;
    walk_up(&here).ok_or_else(|| {
        format!(
            "no git repository above {} — run the harness from a checkout",
            here.display()
        )
    })
}

/// The first ancestor that carries a `.git`, including this one.
fn walk_up(from: &Path) -> Option<PathBuf> {
    from.ancestors()
        .find(|parent| parent.join(".git").exists())
        .map(Path::to_path_buf)
}

/// The checkout a lane's arguments name.
fn checkout_of(lane: &cli::LaneArgs) -> dispatch::shared::Checkout<'_> {
    dispatch::shared::Checkout {
        target_repo_url: &lane.target_repo_url,
        branch: &lane.branch,
        workspace: (!lane.use_workspace.is_empty()).then_some(lane.use_workspace.as_str()),
    }
}

/// What a lane's child prints and returns once its workflow is done.
fn lane_exit(
    here: &Path,
    what: &str,
    issue: u64,
    ran: harness_core::domain::Outcome<harness_core::domain::Verdict>,
) -> ExitCode {
    match ran {
        Ok(verdict) => {
            println!("{what} #{issue}: {verdict:?}");
            ExitCode::SUCCESS
        }
        Err(halt) => {
            tell_halt(here, &format!("{what} #{issue}"), &halt);
            halt_to_code(&halt)
        }
    }
}

/// A lane's halt, kept as an event. Not in the error ledger: the doctor
/// repairs from that ledger's last row, and lanes ending side by side would
/// put rows there it was not called for.
fn tell_halt(here: &Path, workflow: &str, halt: &harness_core::domain::Halt) {
    if let Some(events) = adapters::events::Teller::open(here, "lanes") {
        events.tell(&harness_core::traces::Event::Halted {
            workflow: workflow.to_string(),
            kind: halt.prefix().to_string(),
            reason: halt.reason().to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::Halt;

    #[test]
    fn the_exit_codes_are_the_frozen_contract() {
        // An external scheduler reads them: moving them would be a hidden
        // contract change dressed up as refactoring.
        assert_eq!(Halt::Halted(String::new()).exit_code(), 1);
        assert_eq!(Halt::Unreadable(String::new()).exit_code(), 1);
        assert_eq!(Halt::Failed(String::new()).exit_code(), 2);
        assert_eq!(Halt::Quota(String::new()).exit_code(), 3);
        // And each fits in the `u8` that a process returns.
        for halt in [
            Halt::Halted(String::new()),
            Halt::Unreadable(String::new()),
            Halt::Failed(String::new()),
            Halt::Quota(String::new()),
        ] {
            assert!(u8::try_from(halt.exit_code()).is_ok());
        }
    }

    #[test]
    fn the_repository_is_found_by_walking_up_without_a_subprocess() {
        // This repository: the test runs inside it, so the answer is known.
        let found = walk_up(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("a repository");
        assert!(found.join(".git").exists());
    }

    #[test]
    fn outside_a_checkout_the_answer_is_none_rather_than_a_bare_exception() {
        assert!(walk_up(Path::new("/")).is_none());
    }
}
