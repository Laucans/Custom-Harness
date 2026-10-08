//! What sequences: the table of a round, the round itself, and the loop that
//! repeats it N times.
//!
//! **`stages` is the workflow's design surface.** The order of its table
//! *is* the execution order; `round` carries what surrounds it (the choice
//! of task, the post-condition), and `workflow` counts the turns and calls
//! the round once per turn.

pub mod round;
pub mod stages;
pub mod workflow;
