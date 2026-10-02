//! Pure vocabulary: no disk, no subprocess, no external library.
//!
//! Names no workflow, even in comments — this is what allows
//! `harness-workflows` to depend on this module without the reciprocal ever
//! becoming conceivable.

pub mod markers;
pub mod prompts;
pub mod workspace;

mod halt;
mod issue;
mod pulls;
mod remote;
mod resumable;
mod spend;
mod verdict;

pub use halt::{Halt, Severity};
pub use issue::Issue;
pub use prompts::{Named, Scope, Scoped};
pub use pulls::Pr;
pub use remote::{Slug, same_repo};
pub use resumable::Resumable;
pub use spend::{Spend, Tokens};
pub use verdict::{Outcome, Verdict};
