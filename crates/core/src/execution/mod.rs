//! Faire tourner : le contexte, les deux traits, la porte, la stage, le
//! round.

pub mod provisioning;

mod action;
mod ask;
mod context;
mod gate;
mod guards;
mod round;
mod stage;
mod traits;

pub use action::{Action, Open, SessionAction, Unpaid};
pub use ask::ask_and_record;
pub use context::{Context, Settings};
pub use gate::Gate;
pub use guards::{InThisRun, MarkDone, StageAlreadyDone};
pub use round::Round;
pub use stage::{Stage, StageBody};
pub use traits::{Executable, Guarded, Verification};
