//! A task's every missing unit, found at once — where its sessions found
//! them one stop at a time.
//!
//! Triggered by the router (`Route::ReStudy`) on a task whose streak of
//! discoveries is due (`common::restudy::due`). **Read-only on the
//! product**: one session (opus) on its lane's own checkout of the
//! milestone's branch walks every rule of the task — the screen's
//! composition, each Micro-UI's needs and actions against the manifests,
//! the fixtures, the running server probed on its own ports
//! ([`config::ports_pair`]) — and answers `RESTUDY: tasks [...]` (the
//! slices, as a stop declares them) and `RESTUDY_CONTEST: [...]` (the rule
//! tests to contest). The harness opens the slices through the one opener
//! (`common::missing_task`), which chains them by `needs` and `produces`,
//! parks the task on every one, and writes under `## Decision` what was
//! opened and what to contest.
//!
//! **The streak is taken before the session is paid for** (the
//! `restudied:` marker), as `decide` takes its label: a re-study that fails
//! is not paid again every tick.
//!
//! Same skeleton as the other workflows (`ARCHITECTURE.md`).

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
