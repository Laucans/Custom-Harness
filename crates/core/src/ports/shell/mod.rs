//! The ports onto the shell: a repository, GitHub, the disk.
//!
//! One module per external component, and `process` for the value they all
//! hand back. The implementations live in
//! [`adapters::shell`](crate::adapters::shell) — a port never spawns anything
//! itself.

pub mod disk;
pub mod git;
pub mod github;
pub mod process;
