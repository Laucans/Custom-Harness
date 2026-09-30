//! L'extérieur, emballé.
//!
//! Une porte décide, elle n'appelle jamais un sous-processus ou une
//! bibliothèque externe elle-même — c'est cette couche qui le fait, et elle
//! seule. Un sous-module par composant externe, comme côté Python.

pub mod agent;
