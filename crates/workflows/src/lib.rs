#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// Same reason as at the root of `harness-core`: decision #3 chooses `?Send`
// everywhere, and `clippy::future_not_send` presupposes the opposite.
#![allow(clippy::future_not_send)]

//! The instances: what `harness-core` cannot name.
//!
//! One submodule per workflow, plus `common` for what at least two of them
//! actually read — labels, the routing decision, the repo map, a fake GitHub
//! for their tests.
//!
//! **The workflows share the same skeleton** — `ports` / `config` / `run`
//! at the root, then `data` / `action` / `checks` / `orchestration`. What each
//! carries, the sense of dependencies between them, and the reason for each
//! rule: `ARCHITECTURE.md`, to be read before adding a workflow or moving a file
//! in an existing one. `init_repo` is a departure from that skeleton — see
//! its own `mod.rs`.

pub mod common;
pub mod dev_loop;
pub mod init_repo;
pub mod main_agent_merge;
pub mod milestone_merge;
pub mod planner;
pub mod pr_fix;
pub mod pr_review;
pub mod refinement;
pub mod split;
