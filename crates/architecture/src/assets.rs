//! The files a target repository is brought under the architecture with.
//!
//! Compiled in, so every repository the plant works on carries the same
//! architecture from the same source: the architecture document, the rules
//! section of `CLAUDE.md`, the seventeen contracts, the CI gates, the project's
//! CI for its stack, the script that scopes its tests, the one that finds the
//! dead code the compiler cannot see, the mutation gate's configuration and
//! its rotation with the script that files what it misses, the script that
//! dumps its schema, an empty data model to grow, an empty list of mock pages
//! and the mock-up's loader, screen host, mock server and build, and the
//! rules' kit: their gate, their runtime's stub, the business and journey
//! suites the rules' task writes their tests in. The kit's base
//! fragments are not installed: the harness keeps them and writes the styled
//! ones (`kit`). What to write of them in a given
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

/// The files installed as they are.
///
/// The document, the gates, the CI, the test scope, the isolation check, the
/// dead-code script, the mutants configuration, rotation and test-debt
/// filing, the schema dump, the data model, the mock-up's files, the
/// contracts A–Q, the ports and the secrets' scaffolding, the rules' gate,
/// runtime and suites.
pub const FILES: [Asset; 50] = [
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
        path: "scripts/dead-code.sh",
        content: include_str!("../assets/dead-code.sh"),
    },
    Asset {
        path: ".cargo/mutants.toml",
        content: include_str!("../assets/mutants.toml"),
    },
    Asset {
        path: "scripts/mutants-rotation.sh",
        content: include_str!("../assets/mutants-rotation.sh"),
    },
    Asset {
        path: "scripts/test-debt.sh",
        content: include_str!("../assets/test-debt.sh"),
    },
    Asset {
        path: "scripts/dump-schema.sh",
        content: include_str!("../assets/dump-schema.sh"),
    },
    Asset {
        path: "scripts/isolation.sh",
        content: include_str!("../assets/isolation.sh"),
    },
    Asset {
        path: "data/model.json",
        content: include_str!("../assets/model.json"),
    },
    Asset {
        path: "mock/pages.json",
        content: include_str!("../assets/mock-pages.json"),
    },
    Asset {
        path: "mock/mock.js",
        content: include_str!("../assets/mock/mock.js"),
    },
    Asset {
        path: "mock/style.css",
        content: include_str!("../assets/mock/style.css"),
    },
    Asset {
        path: "mock/build.sh",
        content: include_str!("../assets/mock/build.sh"),
    },
    Asset {
        path: "mock/build.mjs",
        content: include_str!("../assets/mock/build.mjs"),
    },
    Asset {
        path: "mock/src/server.js",
        content: include_str!("../assets/mock/src/server.js"),
    },
    Asset {
        path: "mock/src/screen.js",
        content: include_str!("../assets/mock/src/screen.js"),
    },
    Asset {
        path: "mock/src/reads.js",
        content: include_str!("../assets/mock/src/reads.js"),
    },
    Asset {
        path: "mock/src/commands.js",
        content: include_str!("../assets/mock/src/commands.js"),
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
    Asset {
        path: "contracts/n-mock-pages.schema.json",
        content: include_str!("../assets/contracts/n-mock-pages.schema.json"),
    },
    Asset {
        path: "contracts/o-design-kit.schema.json",
        content: include_str!("../assets/contracts/o-design-kit.schema.json"),
    },
    Asset {
        path: "contracts/p-rules.schema.json",
        content: include_str!("../assets/contracts/p-rules.schema.json"),
    },
    Asset {
        path: "contracts/q-ports.schema.json",
        content: include_str!("../assets/contracts/q-ports.schema.json"),
    },
    Asset {
        path: "ports.json",
        content: include_str!("../assets/ports.json"),
    },
    Asset {
        path: ".sops.yaml",
        content: include_str!("../assets/sops.yaml"),
    },
    Asset {
        path: "secrets/README.md",
        content: include_str!("../assets/secrets-readme.md"),
    },
    Asset {
        path: "scripts/rules-gate.mjs",
        content: include_str!("../assets/rules-gate.mjs"),
    },
    Asset {
        path: "scripts/rules-runtime.sh",
        content: include_str!("../assets/rules-runtime.sh"),
    },
    Asset {
        path: "tests/rules/business/Cargo.toml",
        content: include_str!("../assets/rules/business/Cargo.toml"),
    },
    Asset {
        path: "tests/rules/business/src/lib.rs",
        content: include_str!("../assets/rules/business/src/lib.rs"),
    },
    Asset {
        path: "tests/rules/journey/package.json",
        content: include_str!("../assets/rules/journey/package.json"),
    },
    Asset {
        path: "tests/rules/journey/playwright.config.ts",
        content: include_str!("../assets/rules/journey/playwright.config.ts"),
    },
    Asset {
        path: "tests/rules/journey/kit.ts",
        content: include_str!("../assets/rules/journey/kit.ts"),
    },
    Asset {
        path: "tests/rules/journey/status-reporter.ts",
        content: include_str!("../assets/rules/journey/status-reporter.ts"),
    },
    Asset {
        path: "tests/rules/journey/.gitignore",
        content: include_str!("../assets/rules/journey/gitignore"),
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
    fn the_ports_start_with_the_database_and_the_secrets_stay_encrypted() {
        let ports: serde_json::Value =
            serde_json::from_str(content_of("ports.json").unwrap_or_default()).expect("json");
        assert_eq!(ports["version"], 1);
        assert_eq!(ports["ports"][0]["id"], "database");
        assert_eq!(ports["ports"][0]["stateful"], true);
        assert!(content_of("contracts/q-ports.schema.json").is_some());
        assert!(
            content_of(".sops.yaml")
                .unwrap_or_default()
                .contains("secrets/.*\\.enc\\.yaml$")
        );
        assert!(content_of("secrets/README.md").is_some());
        for contract in [
            "contracts/b-capability-manifest.schema.json",
            "contracts/f-data-capability.schema.json",
        ] {
            assert!(
                content_of(contract)
                    .unwrap_or_default()
                    .contains("\"ports\""),
                "{contract} names its ports"
            );
        }
        assert!(
            CLAUDE_RULES.contains("goes through a port")
                && CLAUDE_RULES.contains("AGENT_LOOP_STOP: port")
        );
    }

    #[test]
    fn the_rules_kit_is_installed_and_the_gates_run_it() {
        for path in [
            "contracts/p-rules.schema.json",
            "scripts/rules-gate.mjs",
            "scripts/rules-runtime.sh",
            "tests/rules/business/Cargo.toml",
            "tests/rules/business/src/lib.rs",
            "tests/rules/journey/kit.ts",
            "tests/rules/journey/status-reporter.ts",
        ] {
            assert!(content_of(path).is_some(), "{path} is installed");
        }
        let gates = content_of(".github/workflows/gates.yml").unwrap_or_default();
        for step in [
            "node scripts/rules-gate.mjs coverage",
            "node scripts/rules-gate.mjs status $STRICT",
            "node scripts/rules-gate.mjs untouched",
            "bash scripts/rules-runtime.sh serve",
            "cargo test --manifest-path tests/rules/business/Cargo.toml",
            "npx playwright test",
            "harness:rules",
        ] {
            assert!(gates.contains(step), "the gates run {step}");
        }
        for step in [
            "bash scripts/rules-runtime.sh stop",
            "DATABASE_URL: postgres://postgres:postgres@localhost:5432/app",
        ] {
            assert!(gates.contains(step), "the rules' runtime gets {step}");
        }
        let runtime = content_of("scripts/rules-runtime.sh").unwrap_or_default();
        for part in [
            "crates/server/Cargo.toml",
            "apps/shell/package.json",
            "/health",
            "FIXTURES_DIR",
        ] {
            assert!(
                runtime.contains(part),
                "the runtime starts what the layout fixes: {part}"
            );
        }
        let gate = content_of("scripts/rules-gate.mjs").unwrap_or_default();
        assert!(
            gate.contains("p === 'scripts/rules-runtime.sh'"),
            "nobody changes the runtime"
        );
        let business = content_of("tests/rules/business/Cargo.toml").unwrap_or_default();
        assert!(
            business.contains("[workspace]"),
            "the rules' suite builds apart"
        );
        let rules = CLAUDE_RULES;
        assert!(rules.contains("CONTEST.md") && rules.contains("harness:rules"));
    }

    #[test]
    fn the_ci_runs_the_dead_code_script_and_the_mutants_it_is_installed_with() {
        let ci = content_of(".github/workflows/ci.yml").unwrap_or_default();
        assert!(ci.contains("cargo machete"), "{ci}");
        assert!(ci.contains("bash scripts/dead-code.sh"), "{ci}");
        assert!(ci.contains("--in-diff change.diff"), "{ci}");
        assert!(
            ci.contains("--workspace"),
            "default-members would narrow it"
        );
        assert!(
            content_of("scripts/dead-code.sh")
                .is_some_and(|s| s.starts_with("#!/usr/bin/env bash"))
        );
        let mutants = content_of(".cargo/mutants.toml").unwrap_or_default();
        assert!(mutants.contains("exclude_globs"), "{mutants}");
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
