//! The binaries, wrapped: `git`, `gh`, the disk and its volume, and the
//! launcher they share.
//!
//! The implementations behind [`ports::shell`](harness_core::ports::shell).
//! `process` is the only one that talks to the system; the others translate
//! arguments and read outputs.

pub mod database;
pub mod disk;
pub mod git;
pub mod github;
pub mod process;
pub mod volume;
