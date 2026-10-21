//! The registry of the product's units: every aggregate, `DataCapability` and
//! Capability folder under `crates/<system>/`, gathered into the lists the
//! server plants (`dataguard_server::plant`).
//!
//! Nobody edits this crate. Its build script walks the unit folders and
//! writes the glue this file includes; its `Cargo.toml` is written by
//! `scripts/register.sh`, one dependency line per unit crate. A unit folder
//! the `Cargo.toml` does not name, or a line naming no unit folder, stops
//! the build and says to run the script. Adding a unit is its folder and
//! one rerun of the script: no shared file is edited by hand.
//!
//! Each unit crate exposes one conventional item, which the glue calls:
//!
//! - an aggregate (`crates/<system>/aggregates/<Aggregate>/`, library
//!   `aggregate_<snake>`) and a `DataCapability`
//!   (`crates/<system>/data-capabilities/<folder>/`, library named after its
//!   folder, `-` as `_`): `pub fn plugin() -> Plugin`;
//! - a Capability (`crates/<system>/capabilities/<folder>/`, library named
//!   after its folder): `pub const CAPABILITY: &str`, `pub const SQL: &str`,
//!   `pub const VARIABLES: &[&str]` and `pub fn answer(&Value,
//!   &BTreeMap<String, Value>) -> Value`, which the glue wraps into an
//!   `Answer`. A Capability never depends on the engine.
//!
//! A unit whose manifest says `"host": "hostless"` runs in no server and is
//! skipped. A folder holding only its manifest (no `Cargo.toml` yet) is
//! registered by its manifest alone.

use harness_dataguard::plugins::{Plugin, Plugins};

/// One unit the build script found.
pub struct Unit {
    /// Its folder from the repository root: `crates/<system>/<kind>/<folder>`.
    pub folder: &'static str,
    /// Its manifest, when its folder holds one: `aggregate.json` (I),
    /// `data-capability.json` (F) or `capability.json` (B).
    pub manifest: Option<&'static str>,
    /// What its crate plugs in, when its folder is a crate.
    pub plugin: Option<fn() -> Plugin>,
}

impl Unit {
    /// The types the unit reads from the server's `Shared`, by name.
    #[must_use]
    pub fn needs(&self) -> Vec<&'static str> {
        self.plugin
            .map(|plugin| std::iter::once(plugin()).collect::<Plugins>().needs())
            .unwrap_or_default()
    }
}

// `AGGREGATES`, `DATA_CAPABILITIES`, `CAPABILITIES` (each a `&[Unit]`) and
// `QUERIES` (the registry of persisted queries, contract C).
include!(concat!(env!("OUT_DIR"), "/glue.rs"));

/// Every unit's plugin, together.
#[must_use]
pub fn plugins() -> Plugins {
    AGGREGATES
        .iter()
        .chain(DATA_CAPABILITIES)
        .chain(CAPABILITIES)
        .filter_map(|unit| unit.plugin)
        .map(|plugin| plugin())
        .collect()
}
