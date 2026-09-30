#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]

//! Le framework du harness : vocabulaire, traces, exécution.
//!
//! Rien ici n'importe `harness-workflows` ni `harness-launcher` — et Cargo
//! refuse de compiler l'inverse. L'invariant « le framework ignore ses
//! utilisateurs » est donc tenu par le graphe de crates, à la compilation,
//! plutôt que par un test qui parcourt des AST.

pub mod domain;
pub mod execution;
pub mod traces;
