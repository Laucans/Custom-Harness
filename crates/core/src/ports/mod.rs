//! What the framework needs from the outside world, as traits.
//!
//! The inside of the hexagon. Everything here is a trait (plus the few values
//! those traits exchange), declared by `harness-core` and named by the
//! workflows; nothing here spawns a process, touches the disk or knows a
//! binary. The implementations live in [`adapters`](crate::adapters), and the
//! launcher is what picks one — which is why a workflow is testable against a
//! fake without mocking a call site, and why `--dry-run` is a wiring choice
//! rather than a branch in the framework.
//!
//! A port never appears in the same module as an implementation of it: the
//! direction of the dependency is `adapters -> ports`, never the reverse, and
//! that is what lets a workflow family with no `git` in it reuse the
//! framework.

pub mod agent;
pub mod shell;
pub mod store;
