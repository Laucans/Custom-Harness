//! Why the product's data model diverges.
//!
//! One paid session that reads both
//! sides of a version the watch just archived — the schema the migrations
//! build and the agents' `data/model.json` — with the history behind them,
//! and says, divergence by divergence, which side is wrong and what to do.
//!
//! Triggered by the router when `data_snapshot` archives a version of
//! `main` or the integration branch whose two sides disagree. It edits
//! nothing and posts nothing: its answer becomes an error notification in
//! the factory view, where the human reads the state of affairs and may turn
//! it into an issue the pipeline takes from there.
//!
//! Same skeleton as other workflows (`ARCHITECTURE.md`), without `checks/`:
//! nothing judges the single stage but its own reply's shape.

pub mod action;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
