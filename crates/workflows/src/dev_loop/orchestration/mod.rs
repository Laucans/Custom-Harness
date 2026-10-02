//! Ce qui séquence : la table d'un round, le round lui-même, et la boucle qui
//! en répète N.
//!
//! **`stages` est la surface de design du workflow.** L'ordre de sa table
//! *est* l'ordre d'exécution ; `round` porte ce qui l'entoure (le choix de la
//! task, la bifurcation de rollover, la post-condition), et `workflow` compte
//! les tours et appelle le round une fois par tour.

pub mod round;
pub mod stages;
pub mod workflow;
