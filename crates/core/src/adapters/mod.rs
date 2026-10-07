//! External dependencies, wrapped — one implementation per port.
//!
//! A port decides, it never calls a subprocess or external library itself —
//! this layer does it, and only this layer. The traits implemented here are
//! declared in [`ports`](crate::ports); nothing outside the launcher names a
//! type from this module.

pub mod agent;
pub mod shell;
pub mod store;
