//! La boucle de développement : le premier workflow du harness.
//!
//! Un round choisit une task, la fait passer par ses trois stages, et constate
//! sa livraison. La définition vit ici ; le framework qui la fait tourner vit
//! dans `harness-core`.
//!
//! `dev_loop` et non `loop` : `loop` est un mot-clé.

pub mod actions;
pub mod board;
pub mod gates;
pub mod preflight;
pub mod round;
pub mod stages;
pub mod state;
pub mod tasks;
pub mod wiring;
pub mod workflow;
