//! External dependencies, wrapped.
//!
//! A port decides, it never calls a subprocess or external library itself —
//! this layer does it, and only this layer. One submodule per external component.

pub mod agent;
pub mod shell;
pub mod store;
