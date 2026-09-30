//! La boucle de développement : le premier workflow du harness.
//!
//! Un round choisit une task, la fait passer par ses trois stages, et constate
//! sa livraison. La définition vit ici ; le framework qui la fait tourner vit
//! dans `harness-core`.
//!
//! `dev_loop` et non `loop` : `loop` est un mot-clé.

pub mod labels;
pub mod state;
pub mod tasks;
