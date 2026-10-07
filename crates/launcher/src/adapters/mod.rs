//! The ports `harness-core` declares and only the launcher can fill.
//!
//! Both implementations here exist because the fact they need is a fact of
//! the **process**, not of a workflow: where this run's log goes, and what
//! time it is on which machine. `harness-core` carries neither a clock nor a
//! console, and that is deliberate — a ledger line receives its timestamp
//! rather than reading one, which is what makes it testable.
//!
//! Distinct from [`crate::dispatch`], which also builds concrete things: the
//! adapters there (`GhCli`, `GitCli`, a session factory) come from
//! `harness-core` and are merely *wired* per run. These two are **written
//! here**.

pub mod sink;
pub mod spending;
