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
use std::path::{Path, PathBuf};
use std::rc::Rc;

use harness_core::execution::Gate;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::checkpoint::Checkpoints;

use crate::dev_loop::action::actions::{MarkWaitingMerge, PickTask};
use crate::dev_loop::checks::{gates, preflight};
use crate::dev_loop::config::Config;
use crate::dev_loop::data::dependencies;
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
    /// The task the resume point designates, if there is one. Only applies to
    /// the first turn — see the `rounds` factory below.
    pub resuming: Option<String>,
    /// The one task this run is for (`--task`), if any — a lane of a
    /// parallel watch. Applies to every turn.
    pub wanted: Option<u64>,
}

/// Mounts the entire loop, from its wiring and what must hold before the first
/// round is paid.
#[must_use]
pub fn build(
    ports: &Ports,
    config: &Config,
    request: Request,
    pre: Gate<Loop>,
    store: Option<Rc<dyn Checkpoints>>,
    flow_id: String,
) -> DevLoop {
    DevLoop {
        pre,
        remaining: Cell::new(request.rounds_budget),
        rounds: rounds(
            ports,
            config,
            request.stages_filter,
            request.resuming,
            request.wanted,
        ),
        store,
        flow_id,
    }
}

/// What [`workspace_gates`] needs to judge the rate-limit window.
///
/// Grouped rather than three more parameters: they travel together, and `now` is
/// passed in rather than read here so the decision stays testable against a
/// clock the caller owns.
pub struct QuotaRoom {
    /// Where the last reading was recorded.
    pub at: PathBuf,
    /// `--ignore-quota`.
    pub ignored: bool,
    /// Now, in seconds since the epoch.
    pub now: u64,
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
    quota: QuotaRoom,
) -> Gate<Loop> {
    let skills = root.join(".claude/skills");
    Gate {
        name: "dev_loop preflight",
        checks: vec![
            // First: it costs one file read, and it is the only gate whose
            // refusal is about *when* rather than about what is wrong. Finding
            // out after three network calls would waste them.
            Box::new(preflight::QuotaHasRoom {
                disk: Rc::clone(&disk),
                at: quota.at,
                ignored: quota.ignored || dry_run,
                now: quota.now,
            }),
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
                needed: if dry_run {
                    Vec::new()
                } else {
                    installed(&root, disk.as_ref())
                },
                disk,
                root,
            }),
        ],
    }
}

/// What this checkout must have installed for a stage to verify, derived
/// from the manifests it actually carries.
///
/// None of it is tracked by git — that's precisely why a fresh clone doesn't
/// have it, and why the gate exists. Which ecosystem it is, though, is the
/// clone's business and not the harness's: see
/// [`data::dependencies`](crate::dev_loop::data::dependencies).
///
/// Public because the repair reads it too: the doctor runs these very commands
/// when a tick stopped for lack of them, and deriving the list twice is how the
/// gate and the repair come to disagree.
#[must_use]
pub fn installed(root: &Path, disk: &dyn Disk) -> Vec<(String, String)> {
    let present: Vec<String> = dependencies::manifests()
        .into_iter()
        .filter(|name| disk.exists(&root.join(name)))
        .collect();
    dependencies::needed(&present)
}

/// The skills this run names: the stages, and the commands that open them.
///
/// A lead command is not the name of any table entry — `/tech-analyst` opens
/// the `code` stage — so nothing else would notice it missing.
fn named_skills(ports: &Ports, config: &Config) -> Vec<String> {
    let mut named: Vec<String> = stages::table(ports, config, 1)
        .iter()
        .map(|stage| stage.name.clone())
        // Runs `/tech-analyst`: it is not a skill of its own.
        .filter(|name| name != "technical-refinement")
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
    stages_filter: String,
    resuming: Option<String>,
    wanted: Option<u64>,
) -> Box<dyn Fn(u32) -> TaskRound> {
    let gh = Rc::clone(&ports.gh);
    let sessions = Rc::clone(&ports.sessions);
    let spending = Rc::clone(&ports.spending);
    let disk = Rc::clone(&ports.disk);
    let branch = config.integration_branch.clone();
    let model = config.model.clone();
    let effort = config.effort.clone();
    let restart = config.restart;
    let stack = config.stack.clone();
    let signatures = Rc::clone(&config.signatures);
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
            stack: stack.clone(),
            signatures: Rc::clone(&signatures),
        };
        TaskRound {
            turn,
            pick: PickTask {
                gh: Rc::clone(&gh),
                resuming: if turn == 1 { resuming.clone() } else { None },
                wanted,
            },
            stages: stages::table(&ports, &config, turn),
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
            resuming,
            wanted: None,
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
            String::new(),
            Some("11".to_string()),
            None,
        );
        assert_eq!(built(1).pick.resuming.as_deref(), Some("11"));
        assert_eq!(built(2).pick.resuming, None);
    }

    #[test]
    fn each_turn_gets_its_own_number_for_the_ledger_and_the_journal() {
        let built = rounds(
            &ports_fake::ports(),
            &config_fake::config(),
            String::new(),
            None,
            None,
        );
        assert_eq!(built(3).turn, 3);
    }

    #[test]
    fn the_table_stays_whole_and_the_filter_lives_in_the_gates() {
        // `--stages` does not remove an entry: it skips a stage *by saying so*.
        // Removing the entry would make the journal unreadable — a pipeline
        // that lost two stages without explanation.
        let built = rounds(
            &ports_fake::ports(),
            &config_fake::config(),
            "code".to_string(),
            None,
            None,
        );
        assert_eq!(built(1).stages.len(), 3);
    }

    #[test]
    fn leaving_code_out_of_stages_is_what_the_delivery_gate_must_know() {
        // Without this, "nothing marks delivery" reads as a bug when in fact
        // it's `--stages` that removed the only stage that delivers.
        assert!(code_runs(""), "empty means all");
        assert!(code_runs("technical-refinement code"));
        assert!(!code_runs("technical-refinement"));
    }

    #[test]
    fn what_must_be_installed_follows_the_checkout_not_the_harness() {
        // The regression this guards: `installed()` was a constant naming
        // `node_modules`, so a Rust target was told to run `npm install` and
        // a fresh repo was refused for lacking a directory it never had.
        use harness_core::ports::shell::disk::Disk as _;
        struct Only(&'static str);
        impl harness_core::ports::shell::disk::Disk for Only {
            fn exists(&self, path: &Path) -> bool {
                path.ends_with(self.0)
            }
            fn create_dir_all(&self, _p: &Path) -> harness_core::domain::Outcome<()> {
                Ok(())
            }
            fn remove_dir_all(&self, _p: &Path) -> harness_core::domain::Outcome<()> {
                Ok(())
            }
            fn dir_names(&self, _p: &Path) -> Vec<String> {
                Vec::new()
            }
            fn read_to_string(&self, _p: &Path) -> Option<String> {
                None
            }
            fn write_to_string(&self, _p: &Path, _c: &str) -> harness_core::domain::Outcome<()> {
                Ok(())
            }
        }
        let root = PathBuf::from("/w");
        let node = Only("package.json");
        assert_eq!(installed(&root, &node).len(), 1);
        assert!(node.exists(&root.join("package.json")), "the fake answers");

        let rust = Only("Cargo.toml");
        assert!(
            installed(&root, &rust).is_empty(),
            "cargo fetches on build; nothing to install up front"
        );
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
