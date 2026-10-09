//! The files a target repository is brought under the architecture with.
//!
//! Compiled in, so every repository the plant works on carries the same
//! architecture from the same source: the architecture document, the rules
//! section of `CLAUDE.md`, the thirteen contracts, the CI gates, the project's
//! CI for its stack, the script that scopes its tests, the one that dumps its
//! schema and an empty data model to grow. What to write of them in a given
//! checkout, and what to keep, is `init-repo`'s decision, not this module's.

/// One file of the architecture: where it goes in the repository, what it says.
pub struct Asset {
    /// The path in the repository.
    pub path: &'static str,
    /// Its content.
    pub content: &'static str,
}

/// The opening marker of the rules block in `CLAUDE.md`, and of the
/// architecture document: the version is what a later `init-repo` compares.
pub const MARKER_OPEN: &str = "<!-- harness:architecture v1 -->";
/// The closing marker.
pub const MARKER_CLOSE: &str = "<!-- /harness:architecture -->";

/// The rules section appended to `CLAUDE.md`, markers included.
pub const CLAUDE_RULES: &str = include_str!("../assets/CLAUDE-rules.md");

/// The script that dumps the product's schema, as installed in
/// `scripts/dump-schema.sh`; `data_snapshot` runs it when the repository has none.
pub const DUMP_SCHEMA: &str = include_str!("../assets/dump-schema.sh");

/// The files installed as they are: the document, the gates, the CI, the
/// test scope, the schema dump, the data model, the contracts.
pub const FILES: [Asset; 20] = [
    Asset {
        path: "docs/ARCHITECTURE.md",
        content: include_str!("../assets/ARCHITECTURE.md"),
    },
    Asset {
        path: ".github/workflows/gates.yml",
        content: include_str!("../assets/gates.yml"),
    },
    Asset {
        path: ".github/workflows/ci.yml",
        content: include_str!("../assets/ci.yml"),
    },
    Asset {
        path: "scripts/test-scope.sh",
        content: include_str!("../assets/test-scope.sh"),
    },
    Asset {
        path: "scripts/dump-schema.sh",
        content: include_str!("../assets/dump-schema.sh"),
    },
    Asset {
        path: "data/model.json",
        content: include_str!("../assets/model.json"),
    },
    Asset {
        path: "contracts/README.md",
        content: include_str!("../assets/contracts/README.md"),
    },
    Asset {
        path: "contracts/a-concept.schema.json",
        content: include_str!("../assets/contracts/a-concept.schema.json"),
    },
    Asset {
        path: "contracts/b-capability-manifest.schema.json",
        content: include_str!("../assets/contracts/b-capability-manifest.schema.json"),
    },
    Asset {
        path: "contracts/c-persisted-query.schema.json",
        content: include_str!("../assets/contracts/c-persisted-query.schema.json"),
    },
    Asset {
        path: "contracts/d-micro-ui-manifest.schema.json",
        content: include_str!("../assets/contracts/d-micro-ui-manifest.schema.json"),
    },
    Asset {
        path: "contracts/e-screen-composition.schema.json",
        content: include_str!("../assets/contracts/e-screen-composition.schema.json"),
    },
    Asset {
        path: "contracts/f-data-capability.schema.json",
        content: include_str!("../assets/contracts/f-data-capability.schema.json"),
    },
    Asset {
        path: "contracts/g-queue-entry.schema.json",
        content: include_str!("../assets/contracts/g-queue-entry.schema.json"),
    },
    Asset {
        path: "contracts/h-command-result.schema.json",
        content: include_str!("../assets/contracts/h-command-result.schema.json"),
    },
    Asset {
        path: "contracts/i-aggregate.schema.json",
        content: include_str!("../assets/contracts/i-aggregate.schema.json"),
    },
    Asset {
        path: "contracts/j-impact-plan.schema.json",
        content: include_str!("../assets/contracts/j-impact-plan.schema.json"),
    },
    Asset {
        path: "contracts/k-reconciliation.schema.json",
        content: include_str!("../assets/contracts/k-reconciliation.schema.json"),
    },
    Asset {
        path: "contracts/l-message.schema.json",
        content: include_str!("../assets/contracts/l-message.schema.json"),
    },
    Asset {
        path: "contracts/m-data-model.schema.json",
        content: include_str!("../assets/contracts/m-data-model.schema.json"),
    },
];

/// The content of the asset installed at `path`, if there is one.
#[must_use]
pub fn content_of(path: &str) -> Option<&'static str> {
    FILES.iter().find(|a| a.path == path).map(|a| a.content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_unquoted_run_line_of_the_workflows_holds_a_colon_space() {
        for workflow in [
            include_str!("../assets/gates.yml"),
            include_str!("../assets/ci.yml"),
        ] {
            for (n, line) in workflow.lines().enumerate() {
                let Some(value) = line.trim_start().strip_prefix("- run: ") else {
                    continue;
                };
                let quoted = value.starts_with('\'') || value.starts_with('"');
                assert!(
                    quoted || !value.contains(": "),
                    "line {}: an unquoted `run:` value holds `: `",
                    n + 1
                );
            }
        }
    }

    #[test]
    fn only_the_three_gates_that_need_the_engine_stay_off() {
        let gates = include_str!("../assets/gates.yml");
        assert_eq!(
            gates.lines().filter(|l| l.trim() == "if: false").count(),
            3,
            "semantic-conformance, queries-valid and schema-impact wait for the DataGuard engine"
        );
    }

    #[test]
    fn every_asset_is_in_place_and_the_rules_carry_both_markers() {
        for asset in &FILES {
            assert!(!asset.content.trim().is_empty(), "{} is empty", asset.path);
            assert!(!asset.path.starts_with('/'));
        }
        assert!(CLAUDE_RULES.starts_with(MARKER_OPEN));
        assert!(CLAUDE_RULES.trim_end().ends_with(MARKER_CLOSE));
        let document = FILES[0].content;
        assert!(document.contains(MARKER_OPEN) && document.contains(MARKER_CLOSE));
        for asset in FILES.iter().filter(|a| {
            std::path::Path::new(a.path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        }) {
            serde_json::from_str::<serde_json::Value>(asset.content)
                .unwrap_or_else(|e| panic!("{} is not JSON: {e}", asset.path));
        }
    }

    #[test]
    fn the_ci_tests_through_the_script_it_is_installed_with() {
        let text = |path: &str| {
            FILES
                .iter()
                .find(|a| a.path == path)
                .map(|a| a.content)
                .unwrap_or_default()
        };
        let ci = text(".github/workflows/ci.yml");
        assert!(ci.contains("bash scripts/test-scope.sh \"$BASE\""), "{ci}");
        assert!(ci.contains("fetch-depth: 0"), "the scope needs the history");
        assert!(text("scripts/test-scope.sh").starts_with("#!/usr/bin/env bash"));
    }
}
