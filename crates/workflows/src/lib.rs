#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// Même raison qu'à la racine de `harness-core` : la décision n°3 choisit
// `?Send` partout, et `clippy::future_not_send` présuppose le contraire.
#![allow(clippy::future_not_send)]

//! Les instances : ce que `harness-core` ne peut pas nommer.
//!
//! Un sous-module par workflow. Le premier, et le seul pour l'instant, est la
//! boucle de développement.

pub mod dev_loop;
