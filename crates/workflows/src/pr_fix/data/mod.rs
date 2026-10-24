//! What the repair reads: its state, and which PRs it is due on
//! ([`candidates`], read by the router).
//!
//! No parsing module here, unlike `planner` or `split`: a repair's output is
//! a pushed commit, not a structure to read back out of a reply's text.

pub mod candidates;
pub mod state;
