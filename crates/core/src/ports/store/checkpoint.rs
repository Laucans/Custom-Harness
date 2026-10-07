//! Where the harness is, and how it resumes — the port.
//!
//! Two things are kept, and the port keeps them separate: a **pointer**, which
//! names the task an interrupted run was doing, and the **states**, one record
//! per step of a flow. A workflow writes both through this port and never
//! learns that a directory of files is what answers.
//!
//! Synchronous, unlike [`Session`](crate::ports::agent::Session) and
//! [`Repo`](crate::ports::shell::git::Repo): those spawn processes, which is
//! slow and async-only in tokio. Writing two kilobytes gains nothing from
//! `await`, and claiming it is async would give a false idea of cost.
//!
//! # The invariant that costs money
//!
//! **An unreadable store is never "nothing ran yet".** The two answers are
//! worth a `/code` session apart: the latter makes us repay a stage that may
//! have already merged. A missing store, an unknown flow, and an empty ID all
//! mean the same harmless thing and return `None`; everything else returns
//! [`Halt::Unreadable`](crate::domain::Halt::Unreadable).
//!
//! The implementation — a hand-readable pointer and one JSONL file per flow —
//! lives in [`adapters::store::checkpoint`](crate::adapters::store::checkpoint).

use serde_json::Value;

use crate::domain::Outcome;

/// The resume point, as the pointer describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pointer {
    /// The current task, if the harness had one.
    pub task: Option<String>,
    /// The flow whose states carry the details.
    pub flow_id: Option<String>,
}

/// Where a run says what it was doing, so the next one can carry on.
pub trait Checkpoints {
    /// The resume point, or an empty pointer if there is none.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if a pointer
    /// exists but cannot be read — an unreadable pointer is not an absent one.
    fn pointer(&self) -> Outcome<Pointer>;

    /// Write the resume point.
    ///
    /// # Errors
    ///
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if the pointer could not
    /// be written.
    fn set_pointer(&self, task: &str, flow_id: &str) -> Outcome<()>;

    /// The task is done: the resume point has nothing left to describe.
    ///
    /// # Errors
    ///
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if a pointer exists and
    /// could not be dropped — leaving it in place would resume a task that is
    /// already done.
    fn clear(&self) -> Outcome<()>;

    /// Add the round state after a step.
    ///
    /// # Errors
    ///
    /// [`Halt::Failed`](crate::domain::Halt::Failed) if the state could not be
    /// written.
    fn save(&self, flow_id: &str, step: &str, state: &Value) -> Outcome<()>;

    /// The most recent state of this flow, or `None`.
    ///
    /// `None` means "nothing has run yet" — missing store, unknown flow, empty
    /// ID. All these answers are harmless.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`](crate::domain::Halt::Unreadable) if the store
    /// exists but cannot be decoded. **This is not the same as `None`.**
    fn load(&self, flow_id: &str) -> Outcome<Option<Value>>;
}
