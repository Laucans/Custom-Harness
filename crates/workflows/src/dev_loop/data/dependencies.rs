//! What a target repo must have installed before a stage can verify
//! anything — derived from the repo, never assumed.
//!
//! **Why this is not a constant.** It was one: `node_modules`, with
//! `npm install` as the remedy, hard-coded from the era when the target was
//! a single Next.js project. That reading of "the target" is the same
//! mistake as expecting a target repo to carry copies of the harness's
//! skills: the harness orchestrates, and what a clone needs installed is a
//! fact *of the clone*. A Rust target was told to run `npm install`, and a
//! repo with no `package.json` at all — a fresh one, for instance — was
//! refused for lacking a directory it could never have.
//!
//! What a manifest implies is deliberately narrow: only dependencies a
//! clone **cannot** carry, because git does not track them, and that a
//! stage's own verification command would trip over. Cargo fetches what it
//! needs on build, so a `Cargo.toml` implies nothing here.
//!
//! Pure: no filesystem access. The caller says which manifests it found.

/// A manifest filename, and what its presence demands.
struct Rule {
    /// The manifest that implies the dependency.
    manifest: &'static str,
    /// The path that must exist in the checkout.
    needs: &'static str,
    /// The command that puts it there.
    remedy: &'static str,
}

/// What each ecosystem's manifest implies, in the order checked.
const RULES: [Rule; 3] = [
    Rule {
        manifest: "package.json",
        needs: "node_modules",
        remedy: "npm install",
    },
    Rule {
        manifest: "pyproject.toml",
        needs: ".venv",
        remedy: "uv sync",
    },
    Rule {
        manifest: "requirements.txt",
        needs: ".venv",
        remedy: "python -m venv .venv && pip install -r requirements.txt",
    },
];

/// What must be installed, given the manifests found at the checkout root.
///
/// An unrecognised repo — or an empty one — needs nothing: better to let a
/// stage's own verification command fail with its real error than to refuse
/// the run over a guess about the ecosystem.
#[must_use]
pub fn needed(present: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for rule in RULES {
        if !present.iter().any(|name| name == rule.manifest) {
            continue;
        }
        // Two manifests can imply the same directory (`pyproject.toml` and
        // `requirements.txt` both mean `.venv`); the first remedy wins.
        if out.iter().any(|(needs, _)| needs == rule.needs) {
            continue;
        }
        out.push((rule.needs.to_string(), rule.remedy.to_string()));
    }
    out
}

/// The manifests this module knows how to read something from.
///
/// The caller checks the checkout for exactly these, so the list of what to
/// `stat` stays with the rules that interpret them.
#[must_use]
pub fn manifests() -> Vec<String> {
    RULES.iter().map(|rule| rule.manifest.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn a_node_project_needs_its_modules() {
        assert_eq!(
            needed(&found(&["package.json"])),
            [("node_modules".to_string(), "npm install".to_string())]
        );
    }

    #[test]
    fn a_rust_project_needs_nothing_installed() {
        // Cargo fetches on build. The old hard-coded list told a Rust
        // target to run `npm install`.
        assert!(needed(&found(&["Cargo.toml"])).is_empty());
    }

    #[test]
    fn an_empty_repo_needs_nothing_rather_than_being_refused() {
        // A fresh target repo carries a README and little else. Demanding
        // `node_modules` there refused a run over a directory the repo
        // could never have had.
        assert!(needed(&[]).is_empty());
    }

    #[test]
    fn two_manifests_implying_the_same_directory_name_it_once() {
        let out = needed(&found(&["pyproject.toml", "requirements.txt"]));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, ".venv");
        assert_eq!(out[0].1, "uv sync", "the first rule's remedy wins");
    }

    #[test]
    fn a_polyglot_repo_needs_both() {
        let out = needed(&found(&["package.json", "pyproject.toml"]));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn every_rule_is_reachable_from_the_manifest_list() {
        // The caller only `stat`s what `manifests()` names, so a rule whose
        // manifest is missing from that list could never fire.
        let listed = manifests();
        for rule in RULES {
            assert!(
                listed.iter().any(|name| name == rule.manifest),
                "{} missing from manifests()",
                rule.manifest
            );
        }
    }
}
