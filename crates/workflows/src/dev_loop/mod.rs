//! The development loop: the harness's first workflow.
//!
//! A round picks a task, runs it through its three stages, and observes
//! delivery. The definition lives here; the framework that runs it lives
//! in `harness-core`.
//!
//! `dev_loop` not `loop`: `loop` is a keyword.
//!
//! Files are organized by role rather than flat:
//!
//! - [`data`] — what the loop reads, pure business logic;
//! - [`action`] — what it writes;
//! - [`checks`] — what judges without ever writing;
//! - [`orchestration`] — what sequences: table, round, loop.
//!
//! [`ports`], [`config`] and [`run`] stay beside this overview rather than
//! in one of the four: none of the three is a loop step. `ports` and `config`
//! are the contract the launcher fulfills so everything else uses them — one
//! carries `Rc<dyn Trait>`, the other values, and mixing them in one type
//! would answer two different questions under one name. `run` is the entry
//! point that, with this contract fulfilled, assembles the entire loop — it's
//! what `harness-launcher` calls, after building the concrete adapters no
//! workflow has the right to name.

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
