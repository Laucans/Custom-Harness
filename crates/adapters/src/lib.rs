#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// Same exception as `harness-core`, for the same reason: every port is
// `#[async_trait(?Send)]` (decision #3 of `docs/MIGRATION.md`), so every
// adapter future is non-`Send` by construction.
#![allow(clippy::future_not_send)]

//! External dependencies, wrapped — one implementation per port.
//!
//! A port decides, it never calls a subprocess or external library itself —
//! this layer does it, and only this layer. The traits implemented here are
//! declared in [`harness_core::ports`]; only the two outer rings
//! (`harness-launcher`, `harness-view`) depend on this crate, so the inside
//! cannot name a type from it: Cargo refuses the dependency.

pub mod agent;
pub mod shell;
pub mod store;
