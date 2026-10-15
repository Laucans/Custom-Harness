//! A Concept's conformance fixtures, run against the Capability that
//! implements it — installed by `harness init-repo`; the repository owns
//! it from here.
//!
//! What the architecture shares between systems is meaning, not code: a
//! Capability that declares `implements: Risk@3` keeps its own logic and
//! passes the fixtures of `concepts/Risk/v3/fixtures.json`. This crate is
//! how it passes them, from a test of its own:
//!
//! ```ignore
//! #[test]
//! fn conformance() {
//!     platform_conformance::check("Risk@3", |input| my_capability::compute(&input));
//! }
//! ```
//!
//! The fixtures file (contract A, `conformance`):
//!
//! ```json
//! {
//!   "concept": "Risk",
//!   "version": 3,
//!   "cases": [
//!     { "name": "a clean payer", "input": { "payments": [] }, "output": { "riskScore": 3, "riskLevel": "LOW" } }
//!   ]
//! }
//! ```
//!
//! A dev-dependency only: it reaches no store, and a test-only crate is not
//! a bridge between slices (`scripts/isolation.sh`).

#![deny(unsafe_code)]
#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

/// A Concept's fixtures, as `concepts/<Concept>/v<n>/fixtures.json` holds them.
#[derive(Debug, Deserialize)]
pub struct Fixtures {
    /// The Concept's name.
    pub concept: String,
    /// Its version.
    pub version: u64,
    /// The cases every implementation must pass.
    pub cases: Vec<Case>,
}

/// One case: what goes in, what must come out.
#[derive(Debug, Deserialize)]
pub struct Case {
    /// What the case is about, for the failure message.
    pub name: String,
    /// The input handed to the implementation.
    pub input: Value,
    /// The output it must answer, compared as JSON.
    pub output: Value,
}

/// `Risk@3` → `(Risk, 3)`.
///
/// # Panics
///
/// When `id` is not `<Concept>@<version>`.
#[must_use]
pub fn parse_id(id: &str) -> (String, u64) {
    let (name, version) = id
        .split_once('@')
        .unwrap_or_else(|| panic!("a Concept id is <Concept>@<version>, not {id:?}"));
    let version = version
        .trim_start_matches('v')
        .parse()
        .unwrap_or_else(|_| panic!("the version of {id:?} is not a number"));
    (name.to_string(), version)
}

/// The repository root: the first ancestor of `CARGO_MANIFEST_DIR` that
/// holds a `concepts/` folder.
///
/// # Panics
///
/// When no ancestor does — the test is not running in a repository under
/// the architecture.
#[must_use]
pub fn repository_root() -> PathBuf {
    let manifest = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR for a test"),
    );
    manifest
        .ancestors()
        .find(|dir| dir.join("concepts").is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            panic!(
                "no concepts/ folder above {} — not a repository under the architecture",
                manifest.display()
            )
        })
}

/// Reads the fixtures of `id` from the repository.
///
/// # Panics
///
/// When the file is missing, is not the expected JSON, or names another
/// Concept or version than `id` does.
#[must_use]
pub fn fixtures_of(id: &str) -> Fixtures {
    let (name, version) = parse_id(id);
    let path = repository_root()
        .join("concepts")
        .join(&name)
        .join(format!("v{version}"))
        .join("fixtures.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let fixtures: Fixtures = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not a fixtures file: {e}", path.display()));
    assert_eq!(fixtures.concept, name, "{} names another Concept", path.display());
    assert_eq!(fixtures.version, version, "{} is another version", path.display());
    fixtures
}

/// The cases of `fixtures` that `compute` answers wrong: the name of each,
/// with what it answered.
pub fn failures<F>(fixtures: &Fixtures, mut compute: F) -> Vec<String>
where
    F: FnMut(Value) -> Value,
{
    fixtures
        .cases
        .iter()
        .filter_map(|case| {
            let got = compute(case.input.clone());
            (got != case.output).then(|| {
                format!(
                    "{}: expected {}, got {}",
                    case.name, case.output, got
                )
            })
        })
        .collect()
}

/// Runs the fixtures of `id` against `compute`, and fails the test on the
/// first case answered wrong — every failing case named.
///
/// # Panics
///
/// When a case fails, or the fixtures cannot be read.
pub fn check<F>(id: &str, compute: F)
where
    F: FnMut(Value) -> Value,
{
    let fixtures = fixtures_of(id);
    assert!(
        !fixtures.cases.is_empty(),
        "{id}: a Concept with no case constrains nothing"
    );
    let failed = failures(&fixtures, compute);
    assert!(
        failed.is_empty(),
        "{id}: {} of {} case(s) fail conformance:\n  - {}",
        failed.len(),
        fixtures.cases.len(),
        failed.join("\n  - ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixtures() -> Fixtures {
        serde_json::from_value(json!({
            "concept": "Risk",
            "version": 3,
            "cases": [
                {"name": "clean", "input": {"late": 0}, "output": {"level": "LOW"}},
                {"name": "late", "input": {"late": 4}, "output": {"level": "HIGH"}}
            ]
        }))
        .expect("fixtures")
    }

    #[test]
    fn an_id_splits_into_name_and_version() {
        assert_eq!(parse_id("Risk@3"), ("Risk".to_string(), 3));
        assert_eq!(parse_id("Risk@v3"), ("Risk".to_string(), 3));
    }

    #[test]
    fn a_right_implementation_fails_no_case_and_a_wrong_one_is_named() {
        let right = |input: Value| {
            let late = input["late"].as_u64().unwrap_or(0);
            json!({"level": if late > 2 { "HIGH" } else { "LOW" }})
        };
        assert!(failures(&fixtures(), right).is_empty());
        let wrong = |_: Value| json!({"level": "LOW"});
        let failed = failures(&fixtures(), wrong);
        assert_eq!(failed.len(), 1);
        assert!(failed[0].starts_with("late: expected"), "{}", failed[0]);
    }
}
