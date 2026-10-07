//! What the repair reads: its state, and nothing else.
//!
//! No parsing module here, unlike `planner` or `split`: a repair's output is
//! a pushed commit, not a structure to read back out of a reply's text.

pub mod state;
