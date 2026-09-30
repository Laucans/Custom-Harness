//! Faire tourner : le contexte, les deux traits, la porte.
//!
//! `Stage`/`Round` n'y sont pas encore : ils tiennent un objet `Session`
//! dont le porteur (tmux ? stream-json ?) n'est pas tranché — voir
//! `docs/MIGRATION.md`, « Ce qui reste à trancher ».

mod context;
mod gate;
mod traits;

pub use context::{Context, Settings};
pub use gate::Gate;
pub use traits::{Executable, Guarded, Verification};
