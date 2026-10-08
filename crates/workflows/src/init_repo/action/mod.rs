//! What `init-repo` writes: labels, the integration branch, the architecture's
//! files in the repository, `.env.local`.
//!
//! Honors `--dry-run` by performing none of them.

pub mod apply;
pub mod install;
