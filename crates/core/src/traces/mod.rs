//! Support transverse : le journal d'un run.
//!
//! Feuille — n'importe rien du reste de `pipeline-core`, et ignore
//! `Halt`/`Verdict` comme il ignore les workflows.

mod logbook;

pub use logbook::{Logbook, Sink, Verbosity};
