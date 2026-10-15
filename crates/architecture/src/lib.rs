#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]

//! The agent-native architecture every target repository follows.
//!
//! The harness builds products, and every product it builds has the same
//! shape: a read side of many independent units — Capabilities, Micro-UIs,
//! Concepts, persisted queries, screen compositions — and a write side behind
//! one `DataGuard` — `DataCapabilities`, invariants, relations, migrations.
//! This crate is that shape, written once:
//!
//! - [`vocabulary`] — the units, their effects, sides and layers, and the
//!   `## Architecture` declaration a task carries;
//! - [`tree`] — a checkout read as the architecture sees it, and the rules it
//!   must keep;
//! - [`model`] and [`schema_sql`] — the product's data model, the agents' side
//!   and the database's, merged with their divergences; [`compare`] — two of
//!   them side by side, what one changes relative to the other;
//! - [`kit`] — the product's design system (its tokens) and its kit of visual
//!   components, the harness's base fragments the kit is styled from, and the
//!   static check a fragment is reused on;
//! - [`assets`] — the files `init-repo` installs to bring a repository under
//!   the architecture.
//!
//! It names no workflow and decides nothing about a run: `harness-workflows`
//! reads it to cut, build and check tasks, `harness-view` to draw the product.
//! It is not in `harness-core` on purpose — a workflow family that builds no
//! software (a consulting report) depends on `core` without inheriting it.
//! Pure but for [`tree::Tree::read`], which reads through core's `Disk` port.

pub mod assets;
pub mod closure;
pub mod compare;
pub mod kit;
pub mod model;
pub mod rules;
pub mod schema_sql;
pub mod tree;
pub mod vocabulary;

#[cfg(test)]
mod fake_disk;
