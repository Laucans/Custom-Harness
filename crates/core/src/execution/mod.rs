//! Running things, sorted the way each workflow sorts itself:
//!
//! - `data` — what a run carries, generic over the workflow's own state;
//! - `action` — what writes;
//! - `checks` — what judges, never writes;
//! - `orchestration` — what sequences: the stage, the round, the workflow.
//!
//! Named, not linked: the four are private modules, and everything they hold
//! is re-exported flat below. Two public paths to the same type would mean two
//! ways to import it, and a workflow picking the deep one would couple itself
//! to this layout.
//!
//! `traits` stays beside this summary rather than inside one of the four: the
//! two base traits it defines — `Verification` for `checks`, `Executable` for
//! `orchestration` — underlie more than one category, so neither owns it.
//! `provisioning` stays separate too: it mounts and unmounts a run's
//! workspace, called by the launcher around a workflow rather than from
//! within one.

pub mod provisioning;

mod action;
mod checks;
mod data;
mod orchestration;
mod traits;

pub use action::ask::ask_and_record;
pub use action::kinds::{Action, Open, SessionAction, Unpaid};
pub use checks::gate::Gate;
pub use checks::guards::{InThisRun, MarkDone, StageAlreadyDone};
pub use data::context::{Context, Settings};
pub use orchestration::round::{Round, Tolerance};
pub use orchestration::stage::{Stage, StageBody};
pub use orchestration::workflow::{Lock, Workflow};
pub use traits::{Executable, Guarded, Verification};
