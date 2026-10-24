//! The files a target repository is brought under the architecture with.
//!
//! Compiled in, so every repository the plant works on carries the same
//! architecture from the same source: the architecture document, the rules
//! section of `CLAUDE.md`, the eighteen contracts, the CI gates, the project's
//! CI for its stack, the script that scopes its tests, the one that finds the
//! dead code the compiler cannot see, the mutation gate's configuration and
//! its rotation with the script that files what it misses, the script that
//! dumps its schema, the guard that gives each test a scratch database and
//! drops it even when the test panics, an empty data model to grow, an empty
//! list of mock pages and the mock-up's loader, screen host, mock server and build, and the
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

/// The version of the two documents these assets install: what their
/// markers carry, and what a later `init-repo` compares to replace an older
/// block.
pub const VERSION: u32 = 2;
/// The opening marker of the two documents, with their version.
pub const MARKER_OPEN: &str = "<!-- harness:architecture v2 -->";
/// The version of the rules block in `CLAUDE.md`, counted apart from the
/// documents: shortening the rules every session pays for must not rewrite
/// a product's documents.
pub const CLAUDE_VERSION: u32 = 6;
/// The opening marker of the rules block in `CLAUDE.md`, with its version.
pub const CLAUDE_MARKER_OPEN: &str = "<!-- harness:architecture v6 -->";
/// What every version of the opening marker starts with.
pub const MARKER_PREFIX: &str = "<!-- harness:architecture v";
/// The closing marker.
pub const MARKER_CLOSE: &str = "<!-- /harness:architecture -->";

/// The version of the block a text carries, read from its opening marker:
/// `None` when it carries none.
#[must_use]
pub fn installed_version(text: &str) -> Option<u32> {
    let at = text.find(MARKER_PREFIX)?;
    let rest = &text[at + MARKER_PREFIX.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The rules section appended to `CLAUDE.md`, markers included.
pub const CLAUDE_RULES: &str = include_str!("../assets/CLAUDE-rules.md");

/// The script that dumps the product's schema, as installed in
/// `scripts/dump-schema.sh`; `data_snapshot` runs it when the repository has none.
pub const DUMP_SCHEMA: &str = include_str!("../assets/dump-schema.sh");

/// The files installed as they are.
///
/// The two documents, the gates, the CI, the test scope, the quiet wrapper a
/// session runs its checks through, the isolation check, the dead-code
/// script, the mutants configuration, rotation and test-debt filing, the
/// schema dump, the data model, the mock-up's files, the conformance
/// runner, the tests' scratch-database guard, the `DataGuard` server (its generic library, the registry of the
/// units and the script that writes it, the product's binary, installed
/// once and the product's from then on), the contracts A–R, the ports
/// and the secrets' scaffolding, the rules' gate, runtime and suites, and
/// the root `.gitignore` that keeps build outputs out of a round's changes.
pub const FILES: [Asset; 69] = [
    Asset {
        path: "docs/ARCHITECTURE.md",
        content: include_str!("../assets/ARCHITECTURE.md"),
    },
    Asset {
        path: "docs/DATA-INTEGRITY.md",
        content: include_str!("../assets/DATA-INTEGRITY.md"),
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
        path: "scripts/quiet.sh",
        content: include_str!("../assets/quiet.sh"),
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
        path: "scripts/queries.sh",
        content: include_str!("../assets/queries.sh"),
    },
    Asset {
        path: "crates/platform/conformance/Cargo.toml",
        content: include_str!("../assets/conformance/Cargo.toml"),
    },
    Asset {
        path: "crates/platform/conformance/src/lib.rs",
        content: include_str!("../assets/conformance/src/lib.rs"),
    },
    Asset {
        path: "crates/platform/scratch-db/Cargo.toml",
        content: include_str!("../assets/scratch-db/Cargo.toml"),
    },
    Asset {
        path: "crates/platform/scratch-db/src/lib.rs",
        content: include_str!("../assets/scratch-db/src/lib.rs"),
    },
    Asset {
        path: "crates/dataguard/server/Cargo.toml",
        content: include_str!("../assets/dataguard-server/Cargo.toml"),
    },
    Asset {
        path: "crates/dataguard/server/src/lib.rs",
        content: include_str!("../assets/dataguard-server/src/lib.rs"),
    },
    Asset {
        path: "crates/dataguard/registry/build.rs",
        content: include_str!("../assets/dataguard-registry/build.rs"),
    },
    Asset {
        path: "crates/dataguard/registry/src/lib.rs",
        content: include_str!("../assets/dataguard-registry/src/lib.rs"),
    },
    Asset {
        path: "scripts/register.sh",
        content: include_str!("../assets/register.sh"),
    },
    Asset {
        path: "crates/server/Cargo.toml",
        content: include_str!("../assets/server/Cargo.toml"),
    },
    Asset {
        path: "crates/server/src/main.rs",
        content: include_str!("../assets/server/src/main.rs"),
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
        path: "contracts/r-plan-graph.schema.json",
        content: include_str!("../assets/contracts/r-plan-graph.schema.json"),
    },
    Asset {
        path: "scripts/contracts-gate.mjs",
        content: include_str!("../assets/contracts-gate.mjs"),
    },
    Asset {
        path: "ports.json",
        content: include_str!("../assets/ports.json"),
    },
    Asset {
        path: "scripts/ports-gate.mjs",
        content: include_str!("../assets/ports-gate.mjs"),
    },
    Asset {
        path: "scripts/design-system-gate.mjs",
        content: include_str!("../assets/design-system-gate.mjs"),
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
    Asset {
        path: ".gitignore",
        content: include_str!("../assets/gitignore"),
    },
];

/// The content of the asset installed at `path`, if there is one.
#[must_use]
pub fn content_of(path: &str) -> Option<&'static str> {
    FILES.iter().find(|a| a.path == path).map(|a| a.content)
}

/// The gate ids of a gates workflow: the `id` of each gate's step, its
/// parts (`<gate>--<part>`) left out.
#[cfg(test)]
pub(crate) fn gate_ids(workflow: &str) -> Vec<&str> {
    workflow
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("- id: "))
        .map(str::trim)
        .filter(|id| !id.contains("--"))
        .collect()
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
    fn the_rules_runtime_creates_its_database_after_the_build() {
        // An idle dated database outlived a cold build would be the watch's
        // sweep's: created right before the server connects to it.
        let script = content_of("scripts/rules-runtime.sh").unwrap_or_default();
        let at = |part: &str| {
            script
                .find(part)
                .unwrap_or_else(|| panic!("the runtime holds {part}"))
        };
        assert!(
            at("database=\"$(scratch_database)\"")
                > at("cargo build --quiet --manifest-path crates/server/Cargo.toml"),
            "the database is created after the server's build"
        );
        assert!(at("database=\"$(scratch_database)\"") < at("nohup \"$binary\""));
    }

    #[test]
    fn the_rules_runtime_s_background_jobs_hold_no_descriptor_of_its_stdout() {
        // ForgeDesMondes #67: `serve | tee rules-env` never returned once
        // the shell existed — its backgrounded subshell kept the pipe open.
        let script = content_of("scripts/rules-runtime.sh").expect("installed");
        for line in script.lines().filter(|l| l.trim_end().ends_with('&')) {
            assert!(
                line.contains(">/dev/null 2>&1 &") || !line.trim_start().starts_with('('),
                "a backgrounded subshell without its own redirections: {line}"
            );
            assert!(!line.contains("& echo"), "{line}");
        }
        let serve = script
            .lines()
            .find(|l| l.contains("npm run serve"))
            .expect("the shell is served");
        assert!(serve.contains("</dev/null >/dev/null 2>&1 &"), "{serve}");
    }

    #[test]
    fn the_root_gitignore_keeps_build_outputs_out_of_a_round_s_changes() {
        // The architecture gate judges the dirty paths too: an unignored
        // `target/` reads as the round touching outside its closure.
        let ignore = content_of(".gitignore").expect("installed");
        for line in ["target/", "node_modules/", ".env.local"] {
            assert!(ignore.lines().any(|l| l == line), "{line} is ignored");
        }
    }

    #[test]
    fn the_two_documents_split_the_layout_from_the_engine_s_manual() {
        let layout = content_of("docs/ARCHITECTURE.md").expect("installed");
        let manual = content_of("docs/DATA-INTEGRITY.md").expect("installed");
        for engine_section in [
            "## DataQueue and concurrency",
            "### G. DataQueue entry",
            "### J. Impact plan",
            "### K. Reconciliation proposal",
            "### L. Message",
        ] {
            assert!(manual.contains(engine_section), "{engine_section}");
            assert!(
                !layout.contains(engine_section),
                "{engine_section} left the layout"
            );
        }
        for layout_section in ["### F. DataCapability", "### I. Aggregate", "## CI gates"] {
            assert!(layout.contains(layout_section), "{layout_section}");
        }
        assert!(
            layout.contains("docs/DATA-INTEGRITY.md"),
            "the layout points at the manual"
        );
        assert!(
            layout.contains("never rejected **for concurrency**"),
            "principle 10, reworded"
        );
        assert!(layout.contains("host: hostless"), "hosts without a server");
    }

    #[test]
    fn every_gate_of_the_workflow_is_cited_by_a_rule_card() {
        let gates = gate_ids(include_str!("../assets/gates.yml"));
        assert_eq!(gates.len(), 29, "{gates:?}");
        let cards: String = crate::vocabulary::Unit::ALL
            .iter()
            .map(|unit| crate::rules::card(*unit))
            .collect();
        for gate in gates {
            assert!(cards.contains(gate), "no rule card names the gate `{gate}`");
        }
    }

    #[test]
    fn a_gate_is_a_step_named_after_it_and_no_gate_hides_another() {
        let gates = include_str!("../assets/gates.yml");
        let lines: Vec<&str> = gates.lines().collect();
        let mut ids = std::collections::HashSet::new();
        for (n, line) in lines.iter().enumerate() {
            let Some(id) = line.trim_start().strip_prefix("- id: ") else {
                continue;
            };
            assert!(ids.insert(id), "two steps are {id}");
            let gate = id.split("--").next().unwrap_or(id);
            assert!(gate_ids(gates).contains(&gate), "{id} is a part of no gate");
            let name = lines.get(n + 1).copied().unwrap_or_default().trim();
            assert_eq!(name, format!("name: {id}"), "a red step names its gate");
            // Every gate runs whatever a gate before it in the job said.
            let condition = lines.get(n + 2).copied().unwrap_or_default().trim();
            let runs_anyway =
                condition.starts_with("if: ${{ !cancelled()") || condition == "if: always()";
            // The rules' runtime is the exception: its tests need it served.
            assert!(
                runs_anyway || id == "rules-hold--serve" || id == "rules-hold",
                "{id}: {condition}"
            );
        }
        // Each job ends by naming the gates that failed.
        let jobs = gates.matches("\n    steps:\n").count();
        assert_eq!(jobs, 4, "the gates are grouped in four jobs");
        assert_eq!(
            gates
                .matches("- name: Failing gates\n        if: failure()")
                .count(),
            jobs
        );
        assert!(gates.contains(r#"echo "::error title=$gate::"#));
        assert!(gates.contains("STEPS: ${{ toJSON(steps) }}"));
    }

    #[test]
    fn every_gate_is_on_now_that_the_engine_exists() {
        let gates = include_str!("../assets/gates.yml");
        assert_eq!(
            gates.lines().filter(|l| l.trim() == "if: false").count(),
            0,
            "queries-valid and schema-impact run with the DataGuard engine"
        );
        assert!(gates.contains("PREPARE gate_q_"), "queries prepare");
        assert!(
            gates.contains("drop[[:space:]]+column"),
            "a dropped column names its readers"
        );
        assert!(
            gates.contains("platform_conformance::check"),
            "conformance runs"
        );
        assert!(
            gates.contains("bash scripts/queries.sh --check"),
            "the registry is checked"
        );
    }

    #[test]
    fn every_asset_is_in_place_and_the_rules_carry_both_markers() {
        for asset in &FILES {
            assert!(!asset.content.trim().is_empty(), "{} is empty", asset.path);
            assert!(!asset.path.starts_with('/'));
        }
        assert!(CLAUDE_RULES.starts_with(CLAUDE_MARKER_OPEN));
        assert!(CLAUDE_RULES.trim_end().ends_with(MARKER_CLOSE));
        assert_eq!(installed_version(CLAUDE_RULES), Some(CLAUDE_VERSION));
        for document in [FILES[0].content, FILES[1].content] {
            assert!(document.contains(MARKER_OPEN) && document.contains(MARKER_CLOSE));
            assert_eq!(installed_version(document), Some(VERSION));
        }
        assert_eq!(installed_version("no marker"), None);
        assert_eq!(
            installed_version("<!-- harness:architecture v1 -->"),
            Some(1)
        );
        assert_eq!(installed_version("<!-- harness:architecture vx -->"), None);
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
        let gates = content_of(".github/workflows/gates.yml").unwrap_or_default();
        for step in [
            "validate q-ports.schema.json ports.json",
            "node scripts/ports-gate.mjs declared",
            "node scripts/ports-gate.mjs contracts",
            "node scripts/ports-gate.mjs behind",
            "node scripts/ports-gate.mjs secrets",
        ] {
            assert!(gates.contains(step), "the gates run {step}");
        }
        assert!(content_of("scripts/ports-gate.mjs").is_some());
    }

    #[test]
    fn a_hostless_capability_conforms_to_contract_b_in_the_walkers_spelling() {
        let schema = |path: &str| -> serde_json::Value {
            serde_json::from_str(content_of(path).unwrap_or_default()).expect("json")
        };
        let b = schema("contracts/b-capability-manifest.schema.json");
        let f = schema("contracts/f-data-capability.schema.json");
        // One spelling for both kinds of unit, and the one the walkers read.
        let hosts = serde_json::json!(["server", "hostless"]);
        assert_eq!(b["properties"]["host"]["enum"], hosts);
        assert_eq!(f["properties"]["host"]["enum"], hosts);
        assert!(
            content_of("scripts/register.sh").is_some_and(|s| s.contains(r#"'"host":"hostless"'"#))
        );
        assert!(
            content_of("crates/dataguard/registry/build.rs")
                .is_some_and(|s| s.contains(r#""\"host\":\"hostless\"""#))
        );
        // A hostless Capability: B's required keys, nothing B does not
        // declare (`additionalProperties` is false), a host of its enum.
        // `npx ajv-cli@5 validate` accepts it, as the gate runs it.
        let manifest = serde_json::json!({
            "capability": "credit.calculateRisk",
            "system": "credit",
            "version": "1.0.0",
            "description": "Computes the customer risk",
            "reads": ["queries/customerPayments.graphql"],
            "host": "hostless"
        });
        let fields = manifest.as_object().expect("an object");
        assert_eq!(b["additionalProperties"], false);
        for key in fields.keys() {
            assert!(b["properties"].get(key).is_some(), "B declares {key}");
        }
        for required in b["required"].as_array().expect("required") {
            let key = required.as_str().expect("a key");
            assert!(fields.contains_key(key), "B requires {key}");
        }
        assert!(
            hosts
                .as_array()
                .is_some_and(|all| all.contains(&manifest["host"]))
        );
    }

    #[test]
    fn the_design_system_gate_is_installed_and_the_gates_run_it() {
        let gate = content_of("scripts/design-system-gate.mjs").unwrap_or_default();
        assert!(
            gate.contains("design-system-gate: computed placement"),
            "placement is the product's to mark"
        );
        assert!(
            !gate.contains("PLACEMENT_FILES"),
            "no product's file names in the template"
        );
        let gates = content_of(".github/workflows/gates.yml").unwrap_or_default();
        for step in [
            "node scripts/design-system-gate.mjs",
            "- id: design-system-single-writer",
            "fetch-depth: 0",
            "mock/design/ and mock/kit/ are read-only outside the design-system branch",
        ] {
            assert!(gates.contains(step), "the gates run {step}");
        }
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
        for owned in [
            "crates/dataguard/server/**",
            "crates/dataguard/registry/**",
            "crates/platform/scratch-db/**",
        ] {
            assert!(
                mutants.contains(&format!("  \"{owned}\",")),
                "what the harness owns is judged in the harness: {owned}"
            );
        }
        let job = ci
            .split("\n  mutants:\n")
            .nth(1)
            .and_then(|rest| rest.split("\n  typescript:").next())
            .unwrap_or_default();
        let condition = job.lines().next().unwrap_or_default();
        assert!(
            condition.contains("github.event_name == 'pull_request'")
                && condition.contains("!startsWith(github.head_ref, 'milestone/')"),
            "mutants judge a task's pull request only, never a push: {condition}"
        );
        // Two shards of the same diff, each reporting all it missed.
        for part in [
            "shard: [0, 1]",
            "fail-fast: false",
            "SHARD: ${{ matrix.shard }}",
            "--shard \"$SHARD/2\"",
            "--workspace --in-place --no-shuffle --in-diff change.diff",
            "git diff \"$BASE\" HEAD > change.diff",
            "3) test ! -s mutants.out/missed.txt ;;",
            "name: mutants-${{ matrix.shard }}.out",
            // The light build, tested by nextest, against the tests' store.
            "CARGO_PROFILE_DEV_DEBUG: 0",
            "CARGO_PROFILE_TEST_DEBUG: 0",
            "tool: cargo-mutants,cargo-nextest",
            "--test-tool nextest",
            "image: postgres:16",
            "DATABASE_URL: postgres://postgres:postgres@localhost:5432/app",
        ] {
            assert!(job.contains(part), "the mutants job: {part}");
        }
        assert!(
            !ci.split("\n  mutants:\n")
                .next()
                .unwrap_or_default()
                .contains("--in-diff"),
            "the rust job no longer runs the in-diff mutants"
        );
        assert!(mutants.contains("all_features = true"), "{mutants}");
        assert!(mutants.contains("minimum_test_timeout = 20"), "{mutants}");
    }

    #[test]
    fn the_scratch_guard_is_installed_and_drops_without_ever_panicking() {
        let manifest = content_of("crates/platform/scratch-db/Cargo.toml").unwrap_or_default();
        assert!(
            manifest.contains("name = \"platform-scratch-db\""),
            "{manifest}"
        );
        assert!(
            manifest.contains("harness-dataguard = { workspace = true"),
            "the engine's client, from the workspace: {manifest}"
        );
        assert!(
            manifest.contains("tokio = { version = \"1\""),
            "a product's workspace holds no tokio: {manifest}"
        );
        let guard = content_of("crates/platform/scratch-db/src/lib.rs").unwrap_or_default();
        assert!(guard.contains("harness_dataguard::tokio_postgres"));
        let drop = guard
            .split("impl Drop for ScratchDb")
            .nth(1)
            .and_then(|rest| rest.split("\nfn name_for").next())
            .expect("a drop guard");
        assert!(drop.contains("WITH (FORCE)"), "{drop}");
        assert!(
            drop.contains("std::thread::spawn"),
            "its own runtime: {drop}"
        );
        assert!(
            drop.contains("tokio::time::timeout"),
            "a hung server: {drop}"
        );
        for panics in ["unwrap(", "expect(", "panic!"] {
            assert!(!drop.contains(panics), "a drop never panics: {panics}");
        }
    }

    #[test]
    fn the_rules_name_the_scratch_guard_and_its_gate_reads_the_added_files() {
        for part in [
            "platform_scratch_db::ScratchDb::create",
            "crates/platform/scratch-db/",
            "never edit it",
            "Gate: `scratch-databases-shared`",
        ] {
            assert!(CLAUDE_RULES.contains(part), "the rules say {part}");
        }
        let gates = content_of(".github/workflows/gates.yml").unwrap_or_default();
        let gate = gates
            .split("- id: scratch-databases-shared")
            .nth(1)
            .and_then(|rest| rest.split("\n      - id: ").next())
            .expect("the gate");
        for part in [
            "git diff --name-only --diff-filter=A \"$BASE\"...HEAD",
            "milestone/*) echo",
            "crates/platform/scratch-db/*) continue ;;",
            "CREATE[[:space:]]+DATABASE",
        ] {
            assert!(gate.contains(part), "the gate: {part}");
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
        assert!(text("scripts/quiet.sh").starts_with("#!/usr/bin/env bash"));
        assert!(
            !ci.contains("quiet.sh"),
            "the CI keeps its whole logs: the wrapper is a session's"
        );
    }
}
