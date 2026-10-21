//! Finds the product's units and writes the glue `src/lib.rs` includes
//! (`OUT_DIR/glue.rs`): one entry per folder under
//! `crates/<system>/{aggregates,data-capabilities,capabilities}/`, its
//! manifest read in, its crate's conventional item called.
//!
//! The registry's `Cargo.toml` must name every unit crate by `path`, and
//! nothing else of the kind: it is `scripts/register.sh`'s, and a gap
//! either way stops the build with the units it concerns.

use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// The three kinds of unit: their folder under a system, their manifest.
const KINDS: [(&str, &str); 3] = [
    ("aggregates", "aggregate.json"),
    ("data-capabilities", "data-capability.json"),
    ("capabilities", "capability.json"),
];

/// One unit folder.
struct Found {
    /// `aggregates`, `data-capabilities` or `capabilities`.
    kind: &'static str,
    /// `crates/<system>/<kind>/<folder>`.
    relative: String,
    /// The folder's own name.
    folder: String,
    /// Its manifest, when it holds one.
    manifest: Option<PathBuf>,
    /// Whether it holds a `Cargo.toml`.
    is_crate: bool,
}

impl Found {
    /// The library the glue calls: `aggregate_<snake>` for an aggregate,
    /// the folder with `-` as `_` otherwise.
    fn lib(&self) -> String {
        if self.kind == "aggregates" {
            format!("aggregate_{}", snake(&self.folder))
        } else {
            self.folder.replace('-', "_")
        }
    }
}

/// `AiSettings` as `ai_settings`: an underscore before an uppercase letter
/// that follows a lowercase one or a digit, `-` as `_`, all lowercase.
fn snake(name: &str) -> String {
    let mut out = String::new();
    let mut previous: Option<char> = None;
    for c in name.chars() {
        if c == '-' {
            out.push('_');
        } else {
            if c.is_ascii_uppercase()
                && previous.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit())
            {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        }
        previous = Some(c);
    }
    out
}

/// True when the manifest says no server runs the unit.
fn hostless(manifest: &Path) -> bool {
    fs::read_to_string(manifest).is_ok_and(|text| {
        let packed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        packed.contains("\"host\":\"hostless\"")
    })
}

/// The sorted subfolders of `dir`; none when it does not exist.
fn folders(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Every unit folder under `crates/`, hostless ones left out.
fn walk(crates: &Path) -> Vec<Found> {
    let mut found = Vec::new();
    for system in folders(crates) {
        let system_name = name_of(&system);
        for (kind, manifest_name) in KINDS {
            for dir in folders(&system.join(kind)) {
                let folder = name_of(&dir);
                let manifest = Some(dir.join(manifest_name)).filter(|m| m.is_file());
                let is_crate = dir.join("Cargo.toml").is_file();
                if (manifest.is_none() && !is_crate) || manifest.as_deref().is_some_and(hostless) {
                    continue;
                }
                found.push(Found {
                    kind,
                    relative: format!("crates/{system_name}/{kind}/{folder}"),
                    folder,
                    manifest,
                    is_crate,
                });
            }
        }
    }
    found
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The unit folders the registry's `Cargo.toml` names: every
/// `path = "../../<system>/<kind>/<folder>"` of a unit kind.
fn registered(manifest: &str) -> BTreeSet<String> {
    manifest
        .lines()
        .filter_map(|line| {
            let at = line.find("path = \"")? + "path = \"".len();
            let rest = &line[at..];
            let path = &rest[..rest.find('"')?];
            let unit = path.strip_prefix("../../")?.trim_end_matches('/');
            let kind = unit.split('/').nth(1)?;
            (unit.split('/').count() == 3 && KINDS.iter().any(|(k, _)| *k == kind))
                .then(|| format!("crates/{unit}"))
        })
        .collect()
}

fn main() {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = here
        .ancestors()
        .nth(3)
        .expect("crates/dataguard/registry sits three levels under the root")
        .to_path_buf();
    let crates = root.join("crates");
    let queries = root.join("queries/registry.json");
    // A directory is watched whole: a unit folder added anywhere under
    // crates/ reruns this script.
    // A path that does not exist would rerun it on every build: the
    // registry of queries is watched once there is one.
    println!("cargo:rerun-if-changed={}", crates.display());
    if queries.is_file() {
        println!("cargo:rerun-if-changed={}", queries.display());
    }
    println!("cargo:rerun-if-changed=Cargo.toml");

    let units = walk(&crates);
    let manifest = fs::read_to_string(here.join("Cargo.toml")).expect("the registry's Cargo.toml");
    let named = registered(&manifest);
    let on_disk: BTreeSet<String> = units
        .iter()
        .filter(|unit| unit.is_crate)
        .map(|unit| unit.relative.clone())
        .collect();
    let unnamed: Vec<&String> = on_disk.difference(&named).collect();
    let stale: Vec<&String> = named.difference(&on_disk).collect();
    if !unnamed.is_empty() || !stale.is_empty() {
        let list = |units: &[&String]| {
            units
                .iter()
                .map(|u| u.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        panic!(
            "crates/dataguard/registry/Cargo.toml does not match the unit folders: run scripts/register.sh\n  \
             not registered: {}\n  registered, no unit crate there: {}",
            list(&unnamed),
            list(&stale)
        );
    }
    let mut libs = BTreeSet::new();
    for unit in units.iter().filter(|unit| unit.is_crate) {
        assert!(
            libs.insert(unit.lib()),
            "two unit crates would both be the library {}: rename the folder {}",
            unit.lib(),
            unit.relative
        );
    }

    let mut glue =
        String::from("// Generated by build.rs from the unit folders: never edited.\n\n");
    for (kind, name) in [
        ("aggregates", "AGGREGATES"),
        ("data-capabilities", "DATA_CAPABILITIES"),
        ("capabilities", "CAPABILITIES"),
    ] {
        let _ = writeln!(
            glue,
            "/// The {kind} the checkout holds.\npub const {name}: &[Unit] = &["
        );
        for unit in units.iter().filter(|unit| unit.kind == kind) {
            let manifest = unit.manifest.as_ref().map_or_else(
                || "None".to_string(),
                |path| format!("Some(include_str!({:?}))", path.display().to_string()),
            );
            let plugin = if !unit.is_crate {
                "None".to_string()
            } else if kind == "capabilities" {
                format!("Some(answer_of_{})", unit.lib())
            } else {
                format!("Some({}::plugin)", unit.lib())
            };
            let _ = writeln!(
                glue,
                "    Unit {{ folder: {:?}, manifest: {manifest}, plugin: {plugin} }},",
                unit.relative
            );
        }
        glue.push_str("];\n\n");
    }
    for unit in units
        .iter()
        .filter(|unit| unit.is_crate && unit.kind == "capabilities")
    {
        let lib = unit.lib();
        let _ = writeln!(
            glue,
            "fn answer_of_{lib}() -> Plugin {{\n    \
             Plugin::new().answer(harness_dataguard::plugins::Answer {{\n        \
             capability: {lib}::CAPABILITY.to_string(),\n        \
             sql: {lib}::SQL.to_string(),\n        \
             variables: {lib}::VARIABLES.iter().map(|v| (*v).to_string()).collect(),\n        \
             answer: {lib}::answer,\n    \
             }})\n}}\n"
        );
    }
    glue.push_str(
        "/// The registry of persisted queries (contract C).\npub const QUERIES: &str = ",
    );
    if queries.is_file() {
        let _ = writeln!(glue, "include_str!({:?});", queries.display().to_string());
    } else {
        glue.push_str("\"{\\\"queries\\\":[]}\";\n");
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("glue.rs");
    fs::write(out, glue).expect("write glue.rs");
}
