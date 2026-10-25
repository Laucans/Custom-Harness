//! What the repair reads: its state, which PRs it is due on
//! ([`candidates`], read by the router), and which model it runs on
//! ([`ladder`]).
//!
//! No parsing module here, unlike `planner` or `split`: a repair's output is
//! a pushed commit, not a structure to read back out of a reply's text.

pub mod candidates;
pub mod ladder;
pub mod state;
