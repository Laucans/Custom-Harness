#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// Bevy systems take their parameters by value — `Res<T>`, `Query<…>`,
// `Commands` — and the scheduler hands them over that way. The lint would
// fire on every system in the crate; it contradicts the engine's design, not
// a choice made here.
#![allow(clippy::needless_pass_by_value)]

//! The plant, drawn. A Bevy scene of boxes under a light, seen through an
//! orthographic camera at an isometric angle, compiled to WebAssembly and
//! bound to the view's canvas.
//!
//! The page (`crates/view/static/app.js`) keeps the data and the chrome: it
//! receives the picture over SSE and pushes it here as JSON; it owns the
//! breadcrumbs, the panes and the steward's terminal. This crate only draws,
//! and reports back what the pointer did — a click on a station, a hover over
//! a chimney — so the page can open the right pane.
//!
//! Two halves, so the shape of the plant is tested without a GPU:
//! [`scene`] turns a picture and a viewpoint into a list of props, labels
//! and hotspots; [`app`] turns that list into entities.

pub mod app;
pub mod bridge;
pub mod model;
pub mod scene;
pub mod sprites;

#[cfg(target_arch = "wasm32")]
pub mod web;
