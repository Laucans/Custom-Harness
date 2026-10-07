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
//! launcher's polling loop (`harness watch`) acts on.

pub mod branching;
pub mod delivery;
pub mod explore;
pub mod hierarchy;
pub mod json_reply;
pub mod labels;
pub mod routing;
pub mod sections;

#[cfg(test)]
pub(crate) mod fake_disk;
#[cfg(test)]
pub(crate) mod fake_github;
#[cfg(test)]
pub(crate) mod fake_locks;
