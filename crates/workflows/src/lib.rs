#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]

//! Les instances : ce que `pipeline-core` ne peut pas nommer.
//!
//! Vide pour l'instant. `Stage`/`Round` tiennent un objet `Session` dont le
//! porteur n'est pas tranché (`docs/MIGRATION.md`) ; le premier workflow —
//! la boucle de dev — s'écrit ici une fois ce port choisi.
