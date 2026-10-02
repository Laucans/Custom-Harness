//! Assembles the entire loop from ports and a config already built.
//!
//! **Not the construction of concrete adapters** — that stays the launcher's
//! work, the only place with the right to name a `GhCli` or
//! `ClaudeCliFactory`. This module receives [`Ports`] (some `Rc<dyn Trait>`,
//! never a concrete type) and [`Config`] already filled, and composes what no
//! workflow before it ever had to write twice: the preflight gates table, the
//! rounds factory, and the [`DevLoop`] that holds them.
//!
//! Same pattern as `pr_review::run::build` and `refinement::run::build`: the
//! request (here, [`Request`]) is distinct from the wiring, and the launcher
//! remains solely responsible for what belongs to no workflow — the tooling
//! gates, command-line parsing, log writing.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use harness_core::adapters::shell::disk::Disk;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::store::checkpoint::Checkpoint;
use harness_core::execution::Gate;

use crate::dev_loop::action::actions::{MarkWaitingMerge, PickTask};
use crate::dev_loop::checks::{gates, preflight};
use crate::dev_loop::config::Config;
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::orchestration::round::TaskRound;
use crate::dev_loop::orchestration::{stages, workflow::DevLoop};
use crate::dev_loop::ports::Ports;

/// What an invocation requests — distinct from [`Ports`] and [`Config`], which
/// are infrastructure rather than a request.
pub struct Request {
    /// At most how many turns.
    pub rounds_budget: u32,
    /// What `--stages` was worth for this run. Empty: all.
    pub stages_filter: String,
    /// Branch the rollover stage.
    pub rollover: bool,
    /// The task the resume point designates, if there is one. Only applies to
    /// the first turn — see the `rounds` factory below.
    pub resuming: Option<String>,
}

/// Mounts the entire loop, from its wiring and what must hold before the first
/// round is paid.
#[must_use]
pub fn build(
    ports: &Ports,
    config: &Config,
    request: Request,
    pre: Gate<Loop>,
    store: Option<Rc<Checkpoint>>,
    flow_id: String,
) -> DevLoop {
    DevLoop {
        pre,
        remaining: Cell::new(request.rounds_budget),
        rounds: rounds(
            ports,
            config,
            request.rollover,
            request.stages_filter,
            request.resuming,
        ),
        store,
        flow_id,
    }
}

/// The gates that speak to the workspace mounted and what's in it.
///
/// `root` is the workspace root — `.claude/skills` and installed dependencies
/// are inferred from it, rather than being passed separately.
#[must_use]
pub fn workspace_gates(
    ports: &Ports,
    config: &Config,
    root: PathBuf,
    dry_run: bool,
    disk: Rc<dyn Disk>,
    gh: Rc<dyn GitHub>,
) -> Gate<Loop> {
    let skills = root.join(".claude/skills");
    Gate {
        name: "dev_loop preflight",
        checks: vec![
            Box::new(preflight::LabelsExist { gh: Rc::clone(&gh) }),
            Box::new(preflight::MilestoneIsReachable { gh }),
            Box::new(preflight::SkillsExist {
                disk: Rc::clone(&disk),
                skills,
                named: named_skills(ports, config),
            }),
            // Last: the only one that speaks to what's *in* the workspace
            // rather than what it is.
            Box::new(preflight::DependenciesAreInstalled {
                disk,
                root,
                needed: if dry_run { Vec::new() } else { installed() },
            }),
        ],
    }
}

/// What this repository must have installed for a stage to verify.
///
/// None of this is tracked by git — that's precisely why a fresh clone doesn't
/// have it, and why the gate exists.
fn installed() -> Vec<(String, String)> {
    vec![("node_modules".to_string(), "npm install".to_string())]
}

/// The skills this run names: the stages, and the commands that open them.
///
/// A lead command is not the name of any table entry — `/tech-analyst` opens
/// the `code` stage — so nothing else would notice it missing.
fn named_skills(ports: &Ports, config: &Config) -> Vec<String> {
    let mut named: Vec<String> = stages::table(ports, config, 1)
        .iter()
        .map(|stage| stage.name.clone())
        .collect();
    named.push("tech-analyst".to_string());
    named
}

/// The round factory that the workflow calls once per turn.
///
/// Resumption applies only to the **first** turn: subsequent turns choose from
/// the table, and giving the resume key again would replay the same task.
fn rounds(
    ports: &Ports,
    config: &Config,
    rollover: bool,
    stages_filter: String,
    resuming: Option<String>,
) -> Box<dyn Fn(u32) -> TaskRound> {
    let gh = Rc::clone(&ports.gh);
    let sessions = Rc::clone(&ports.sessions);
    let spending = Rc::clone(&ports.spending);
    let disk = Rc::clone(&ports.disk);
    let branch = config.integration_branch.clone();
    let model = config.model.clone();
    let effort = config.effort.clone();
    let restart = config.restart;
    let grill_dir = config.grill_dir.clone();
    Box::new(move |turn| {
        let ports = Ports {
            gh: Rc::clone(&gh),
            sessions: Rc::clone(&sessions),
            spending: Rc::clone(&spending),
            disk: Rc::clone(&disk),
        };
        let config = Config {
            integration_branch: branch.clone(),
            model: model.clone(),
            effort: effort.clone(),
            restart,
            grill_dir: grill_dir.clone(),
        };
        TaskRound {
            turn,
            pick: PickTask {
                gh: Rc::clone(&gh),
                resuming: if turn == 1 { resuming.clone() } else { None },
            },
            stages: stages::table(&ports, &config, turn),
            rollover: rollover.then(|| stages::planner(&ports, &config, turn)),
            delivered: MarkWaitingMerge {
                gh: Rc::clone(&gh),
                integration_branch: branch.clone(),
            },
            post: Gate {
                name: "round must have delivered",
                checks: vec![Box::new(gates::AMergedPrClosesTheTask {
                    gh: Rc::clone(&gh),
                    integration_branch: branch.clone(),
                    code_runs: code_runs(&stages_filter),
                    stages: stages_filter.clone(),
                })],
            },
        }
    })
}

/// Does `--stages` let the only stage that delivers a task run?
///
/// What the round's postcondition needs to know so that "nothing marks
/// delivery" doesn't read as a bug when the filter removed `code`.
fn code_runs(stages: &str) -> bool {
    stages.is_empty() || stages.split_whitespace().any(|name| name == "code")
}

#[cfg(test)]
mod tests {
    //! What is tested here is the **wiring**: which round receives what, which
    //! stage is connected, which gate receives which setting. The rules
    //! themselves are tested where they live, against fakes from their crate.

    use super::*;
    use crate::dev_loop::config::fake as config_fake;
    use crate::dev_loop::ports::fake as ports_fake;

    fn request(resuming: Option<String>) -> Request {
        Request {
            rounds_budget: 3,
            stages_filter: String::new(),
            rollover: false,
            resuming,
        }
    }

    #[test]
    fn the_run_names_the_lead_skills_the_table_does_not() {
        // `/tech-analyst` opens the `code` stage: it's not the name of any
        // table entry, so nothing else would notice it missing.
        let named = named_skills(&ports_fake::ports(), &config_fake::config());
        assert!(named.contains(&"tech-analyst".to_string()));
        assert!(named.contains(&"code".to_string()));
        assert!(named.contains(&"create-test".to_string()));
    }

    #[test]
    fn the_resume_key_is_only_given_to_the_first_turn() {
        // Giving it again would replay the same task at each turn of the budget.
        let built = rounds(
            &ports_fake::ports(),
            &config_fake::config(),
            false,
            String::new(),
            Some("11".to_string()),
        );
        assert_eq!(built(1).pick.resuming.as_deref(), Some("11"));
        assert_eq!(built(2).pick.resuming, None);
    }

    #[test]
    fn each_turn_gets_its_own_number_for_the_ledger_and_the_journal() {
        let built = rounds(
            &ports_fake::ports(),
            &config_fake::config(),
            false,
            String::new(),
            None,
        );
        assert_eq!(built(3).turn, 3);
    }

    #[test]
    fn the_rollover_stage_is_only_wired_when_asked_for() {
        let off = rounds(
            &ports_fake::ports(),
            &config_fake::config(),
            false,
            String::new(),
            None,
        );
        assert!(off(1).rollover.is_none(), "an opus run is not the default");
        let on = rounds(
            &ports_fake::ports(),
            &config_fake::config(),
            true,
            String::new(),
            None,
        );
        assert!(on(1).rollover.is_some());
    }

    #[test]
    fn the_table_stays_whole_and_the_filter_lives_in_the_gates() {
        // `--stages` does not remove an entry: it skips a stage *by saying so*.
        // Removing the entry would make the journal unreadable — a pipeline
        // that lost two stages without explanation.
        let built = rounds(
            &ports_fake::ports(),
            &config_fake::config(),
            false,
            "code".to_string(),
            None,
        );
        assert_eq!(built(1).stages.len(), 3);
    }

    #[test]
    fn leaving_code_out_of_stages_is_what_the_delivery_gate_must_know() {
        // Without this, "nothing marks delivery" reads as a bug when in fact
        // it's `--stages` that removed the only stage that delivers.
        assert!(code_runs(""), "empty means all");
        assert!(code_runs("business-analyst code"));
        assert!(!code_runs("business-analyst"));
    }

    #[test]
    fn a_dry_run_asks_for_no_installed_dependencies() {
        // `workspace_gates` only asks for `installed()` if the run is not dry
        // — what this constant holds must at least exist.
        assert!(!installed().is_empty());
    }

    #[test]
    fn build_wires_the_budget_and_the_flow_id_straight_through() {
        let built = build(
            &ports_fake::ports(),
            &config_fake::config(),
            request(None),
            Gate::empty("outillage"),
            None,
            "run-1".to_string(),
        );
        assert_eq!(built.remaining.get(), 3);
        assert_eq!(built.flow_id, "run-1");
        assert!(built.store.is_none());
    }
}
