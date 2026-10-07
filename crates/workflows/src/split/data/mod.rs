//! What split reads: its state, and parsing a slice plan out of a reply.
//!
//! Pure business logic — nothing here calls `gh` or touches a session.

pub mod plan;
pub mod state;
