//! What `init-repo` reads and decides: the plan, the audit criteria, the
//! rendered report, the `.env.local` rewrite, the files of the architecture
//! to install.
//!
//! Pure business logic — nothing here calls `gh`, nothing here writes.
//! What writes lives in [`crate::init_repo::action`].

pub mod audit;
pub mod env_file;
pub mod install;
pub mod plan;
pub mod report;
