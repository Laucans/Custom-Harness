//! Les binaires, emballés : `git`, `gh`, et le lanceur qu'ils partagent.
//!
//! Un sous-module par composant externe. `process` est le seul à parler au
//! système ; les autres traduisent des arguments et lisent des sorties.

pub mod git;
pub mod github;
pub mod process;
