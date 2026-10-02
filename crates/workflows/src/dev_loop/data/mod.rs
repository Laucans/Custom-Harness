//! Ce que la boucle lit : le tableau, l'état d'un round, et ce qu'une task
//! veut dire pour elle.
//!
//! Métier pur ou lecture seule — rien ici n'appelle `gh` ni n'écrit nulle
//! part. Ce qui écrit vit dans `action`, ce qui juge dans `checks`.

pub mod board;
pub mod grounding;
pub mod state;
pub mod tasks;
