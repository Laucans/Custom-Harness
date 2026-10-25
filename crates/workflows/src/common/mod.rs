//! What multiple workflows read, written once.
//!
//! Only things **at least two** workflows actually need today go here — not
//! what might someday serve a third. `labels` serves the loop and refinement;
//! `explore` serves refinement and the planner; `json_reply` serves the
//! planner and split, both of which ask a session for a plan and parse it
//! back the same way; `branching` names a milestone's own branch wherever
//! it's needed outside `split` (which writes a *task's* branch, decided by
//! its session, not derived); `delivery` reads the `Closes #n` convention
//! for the loop (a task is done) and the milestone merge (every task's code
//! is really on the branch); `routing` is the one pure decision the
//! launcher's polling loop (`harness watch`) acts on — `rule_waits` the
//! rules' pending tests it weighs the tasks by —, `catch_up` what keeps
//! every building milestone branch up to the integration branch; `contest`
//! what a PR waits on and how a settled rules contest is followed through,
//! for the router and `decide`; `dossier` reads what
//! the factory's creation decided, for the planner and split. The vocabulary of the
//! agent-native architecture every target repo follows is not here: it is
//! the `harness-architecture` crate.

pub mod branch_sync;
pub mod branching;
pub mod catch_up;
pub mod close_merged;
pub mod contest;
pub mod contract_ask;
pub mod decision;
pub mod delivery;
pub mod dossier;
pub mod explore;
pub(crate) mod handler_split;
pub mod hierarchy;
pub mod json_reply;
pub mod labels;
pub mod review;
pub mod routing;
pub mod rule_waits;
pub mod sections;
pub mod sentinel;

#[cfg(test)]
pub(crate) mod fake_databases;
#[cfg(test)]
pub(crate) mod fake_disk;
#[cfg(test)]
pub(crate) mod fake_git;
#[cfg(test)]
pub(crate) mod fake_github;
#[cfg(test)]
pub(crate) mod fake_handoffs;
#[cfg(test)]
pub(crate) mod fake_locks;
#[cfg(test)]
pub(crate) mod fake_process;
