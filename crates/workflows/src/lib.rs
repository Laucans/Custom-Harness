#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// Même raison qu'à la racine de `harness-core` : la décision n°3 choisit
// `?Send` partout, et `clippy::future_not_send` présuppose le contraire.
#![allow(clippy::future_not_send)]

//! Les instances : ce que `harness-core` ne peut pas nommer.
//!
//! Un sous-module par workflow, plus `common` pour ce qu'au moins deux d'entre
//! eux lisent réellement — les étiquettes, un faux GitHub pour leurs tests.

pub mod common;
pub mod dev_loop;
pub mod pr_review;
pub mod refinement;
