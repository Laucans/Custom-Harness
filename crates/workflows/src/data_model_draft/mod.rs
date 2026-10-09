//! A first `data/model.json` for a branch that has none.
//!
//! Asked from the factory view's data pane: the branch's head is dumped
//! (`scripts/dump-schema.sh`, as `data_snapshot` does), a paid session reads
//! the dump and the code and writes the agents' side of the data model
//! (contract M), and the file is proposed as a pull request on the branch —
//! the write side, merged by a human.
//!
//! Three stages: `prepare` (the clone, the dump, the draft branch — local),
//! `draft` (the session), `publish` (the file checked, committed, pushed, the
//! pull request opened — local).

pub mod action;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
