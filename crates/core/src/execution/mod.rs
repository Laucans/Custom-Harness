//! Faire tourner : le contexte, les deux traits, la porte, la stage, le
//! round.

mod action;
mod context;
mod gate;
mod round;
mod stage;
mod traits;

pub use action::{Action, Open, SessionAction};
pub use context::{Context, Settings};
pub use gate::Gate;
pub use round::Round;
pub use stage::{Stage, StageBody};
pub use traits::{Executable, Guarded, Verification};
