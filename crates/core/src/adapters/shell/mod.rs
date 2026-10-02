//! The binaries, wrapped: `git`, `gh`, and the launcher they share.
//!
//! One submodule per external component. `process` is the only one that talks
//! to the system; the others translate arguments and read outputs.

pub mod disk;
pub mod git;
pub mod github;
pub mod process;
