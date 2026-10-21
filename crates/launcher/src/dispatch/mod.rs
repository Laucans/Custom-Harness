//! One module per thing the harness can be asked to run, plus what they
//! share.
//!
//! **This is where the concrete adapters are named.** A `GhCli`, a `GitCli`,
//! a `claude` session factory, a ledger on disk — `harness-workflows` never
//! names one, and that is what leaves it one test seam per port. Every
//! module here does the same three things in the same order: open a log,
//! mount the checkout it needs, build the adapters, hand them to the
//! workflow's own `run::build`.
//!
//! What differs between them is only what they mount:
//!
//! - [`dev_loop`] takes an **exclusive** workspace — it is the one that runs
//!   several sessions deep and leaves a checkpoint behind.
//! - [`planner`], [`split`], [`refinement`], [`arbiter`] and [`pr_review`]
//!   share one **read-only** checkout ([`shared`]): none of them commits.
//! - [`pr_fix`] gets a **writable** one of its own, on the PR's branch,
//!   because it does commit; [`test_debt`] and [`rules`] too, on the
//!   integration branch.
//! - [`init_repo`] and [`main_agent_merge`] mount nothing: both are
//!   deterministic commands that only talk to GitHub.
//! - [`catch_up`] mounts a writable checkout of its own only when GitHub
//!   refuses to bring the integration branch into a milestone's: there it
//!   regenerates `Cargo.lock` and the registry, and pushes the merge.
//!
//! [`tooling`] holds the gates [`dev_loop`] mounts around its run — the
//! launcher's own `checks` layer, the only one in this crate.
//!
//! [`doctor`] and [`costs`] are the odd ones out: they mount nothing and run
//! no session. The first reads why the last run stopped and repairs what it
//! can; the second reads what the runs cost, by unit of the architecture —
//! the two modules here whose subject is the harness itself rather than the
//! target repository.
//!
//! **Nothing here decides *whether* to run.** That is `cli` (a human typed
//! it) or `router` (a poll found the conditions met).

pub mod arbiter;
pub mod catch_up;
pub mod contract_manager;
pub mod costs;
pub mod data_diagnosis;
pub mod data_model_draft;
pub mod data_snapshot;
pub mod decide;
pub mod dev_loop;
pub mod doctor;
pub mod init_repo;
pub mod lanes;
pub mod main_agent_merge;
pub mod plan_graph;
pub mod planner;
pub mod pr_fix;
pub mod pr_review;
pub mod refinement;
pub mod rules;
pub mod shared;
pub mod skills;
pub mod split;
pub mod test_debt;
pub mod tooling;
pub mod trial;
