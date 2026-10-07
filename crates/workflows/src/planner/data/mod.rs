//! What the planner reads: its state, parsing a plan out of a reply, and
//! the grounding digests a prior grilling session may have left behind.
//!
//! Pure business logic — nothing here calls `gh` or touches a session.

pub mod grounding;
pub mod plan;
pub mod state;
