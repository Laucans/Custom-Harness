//! What multiple workflows read, written once.
//!
//! Only things **at least two** workflows actually need today go here — not
//! what might someday serve a third. `labels` serves the loop and refinement;
//! `explore` serves refinement, and its place here holds for the day a third
//! workflow wants the same repository map, not because it exists yet.

pub mod explore;
pub mod labels;

#[cfg(test)]
pub(crate) mod fake_github;
