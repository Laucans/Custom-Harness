//! What sequences: the table of steps, the round that holds them, and the
//! entire review.
//!
//! **`stages` is the workflow's design surface.** The order of its table *is*
//! the execution order; `round` carries the sequence and the one failure it
//! tolerates; `workflow` carries the `Workflow` shape around it — the
//! precheck, the lock, the summary.

pub mod round;
pub mod stages;
pub mod workflow;
