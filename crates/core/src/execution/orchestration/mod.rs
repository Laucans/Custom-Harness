//! What **sequences**: the stage, the round that chains stages, and the
//! workflow that repeats a round N times.
//!
//! `stage` is the point of variation (session or local), `round` is the
//! generic sequence that most workflows use as-is, and `workflow` is what
//! counts turns and calls `round` once per turn.

pub mod round;
pub mod stage;
pub mod workflow;
