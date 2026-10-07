//! What the loop reads: the board, a round's state, the repository's own
//! configuration, and what a task means to it.
//!
//! Pure business logic or read-only — nothing here calls `gh` or writes
//! anywhere. What writes lives in `action`, what judges in `checks`.

pub mod board;
pub mod brief;
pub mod dependencies;
pub mod signatures;
pub mod stack;
pub mod state;
pub mod tasks;
