//! What sequences: the texts of the steps, the table, the round, and the
//! entire run.
//!
//! **`stages` is the workflow's design surface.** The order of its table *is*
//! the execution order; `prompts` carries what each step says, `round` puts
//! the repo map in front of the sequence, and `workflow` carries the
//! `Workflow` shape around it — the precheck, the lock, the summary.

pub mod prompts;
pub mod round;
pub mod stages;
pub mod workflow;
