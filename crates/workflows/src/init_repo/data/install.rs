//! What `init-repo` installs in the target repository, and what a run must
//! write of it given what is already there.
//!
//! Pure: decides from texts already read, writes nothing. The files are the
//! harness's own assets, compiled in — the architecture document, the
//! rules section of `CLAUDE.md`, the twelve contracts, the CI gates, the
//! project's CI for its stack and the script that scopes its tests — so a
//! repository brought under the plant carries the same architecture as every
//! other one, from the same source.
//!
//! # Two rules
//!
//! - **Nothing already there is overwritten** unless `--force`: a human may
//!   have adapted a contract or a gate, and the harness's version is not
//!   worth more than theirs. A file that exists is reported as kept.
//! - **`CLAUDE.md` is appended to, never replaced**: the rules go in a
//!   block between two markers, after whatever the project already says
//!   about itself. `--force` rewrites the block in place and touches nothing
//!   outside it.

/// One file the harness installs: where it goes, what it says.
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

/// The files installed as they are: the document, the gates, the CI, the
/// test scope, the contracts.
pub const FILES: [Asset; 17] = [
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
];

/// The path of the file the rules are appended to.
pub const CLAUDE_MD: &str = "CLAUDE.md";

/// The commit that carries the install.
pub const COMMIT_MESSAGE: &str = "chore(harness): install the agent-native architecture rules";

/// What one run writes, and what it leaves alone.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct InstallPlan {
    /// Files to write, with their full new content — `CLAUDE.md` included
    /// when its block is to be added or rewritten.
    pub writes: Vec<(String, String)>,
    /// Files already there and left as they are.
    pub kept: Vec<String>,
}

impl InstallPlan {
    /// Decides the writes from what is in the checkout. `existing` answers
    /// the content of a path, or `None` when the file is absent.
    pub fn new(existing: &dyn Fn(&str) -> Option<String>, force: bool) -> Self {
        let mut plan = Self::default();
        for asset in &FILES {
            match existing(asset.path) {
                Some(_) if !force => plan.kept.push(asset.path.to_string()),
                _ => plan
                    .writes
                    .push((asset.path.to_string(), asset.content.to_string())),
            }
        }
        match merge_claude_md(existing(CLAUDE_MD).as_deref(), force) {
            Some(text) => plan.writes.push((CLAUDE_MD.to_string(), text)),
            None => plan.kept.push(CLAUDE_MD.to_string()),
        }
        plan
    }

    /// Nothing to write: the repository already carries everything.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }
}

/// The `CLAUDE.md` a run must write, or `None` when it is to be left alone.
///
/// Absent: the block alone. Present without the block: the block appended
/// after the project's own text. Present with the block: left alone, unless
/// `force` — then the block is rewritten in place, and nothing outside it
/// moves.
#[must_use]
pub fn merge_claude_md(existing: Option<&str>, force: bool) -> Option<String> {
    let block = CLAUDE_RULES.trim_end();
    let Some(text) = existing else {
        return Some(format!("{block}\n"));
    };
    let Some(open) = text.find(MARKER_OPEN) else {
        return Some(format!("{}\n\n{block}\n", text.trim_end()));
    };
    if !force {
        return None;
    }
    let close = text[open..]
        .find(MARKER_CLOSE)
        .map_or(text.len(), |at| open + at + MARKER_CLOSE.len());
    Some(format!(
        "{}{block}{}",
        &text[..open],
        text.get(close..).unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub refuses a workflow that is not valid YAML and runs nothing:
    /// a plain `run:` scalar holding `: ` is the mistake that made the
    /// installed gates never run.
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
    fn the_installed_ci_runs_on_the_integration_and_milestone_branches() {
        let ci = include_str!("../assets/ci.yml");
        assert!(crate::init_repo::data::audit::ci_triggers_on(
            Some(ci),
            "main_agent"
        ));
        assert!(crate::init_repo::data::audit::ci_triggers_on_milestones(
            Some(ci)
        ));
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

    #[test]
    fn an_empty_repository_gets_every_file_and_a_claude_md_made_of_the_rules() {
        let plan = InstallPlan::new(&|_| None, false);
        assert_eq!(plan.writes.len(), FILES.len() + 1);
        assert_eq!(plan.kept, [] as [std::string::String; 0]);
        let (_, claude) = plan
            .writes
            .iter()
            .find(|(path, _)| path == CLAUDE_MD)
            .expect("CLAUDE.md");
        assert!(claude.starts_with(MARKER_OPEN));
    }

    #[test]
    fn a_file_already_there_is_kept_unless_forced() {
        let existing = |path: &str| {
            (path == "contracts/a-concept.schema.json").then(|| "{\"mine\": true}".to_string())
        };
        let plan = InstallPlan::new(&existing, false);
        assert_eq!(plan.kept, ["contracts/a-concept.schema.json"]);
        assert_eq!(plan.writes.len(), FILES.len());
        let forced = InstallPlan::new(&existing, true);
        assert_eq!(forced.kept, [] as [std::string::String; 0]);
        assert_eq!(forced.writes.len(), FILES.len() + 1);
    }

    #[test]
    fn the_rules_are_appended_after_the_project_s_own_claude_md() {
        let merged =
            merge_claude_md(Some("# my project\n\nwhat it is.\n"), false).expect("appended");
        assert!(
            merged.starts_with("# my project\n\nwhat it is.\n\n<!-- harness:architecture v1 -->")
        );
        assert!(merged.trim_end().ends_with(MARKER_CLOSE));
    }

    #[test]
    fn a_claude_md_that_carries_the_block_is_left_alone_and_rewritten_only_under_force() {
        let text =
            format!("# mine\n\n{MARKER_OPEN}\nold rules\n{MARKER_CLOSE}\n\n## after\n\nkept too\n");
        assert_eq!(merge_claude_md(Some(&text), false), None);
        let forced = merge_claude_md(Some(&text), true).expect("rewritten");
        assert!(forced.starts_with("# mine\n\n<!-- harness:architecture v1 -->"));
        assert!(!forced.contains("old rules"));
        assert!(forced.ends_with("\n\n## after\n\nkept too\n"), "{forced}");
        let plan = InstallPlan::new(&|path| (path == CLAUDE_MD).then(|| text.clone()), false);
        assert!(plan.kept.contains(&CLAUDE_MD.to_string()));
    }

    #[test]
    fn a_repository_with_everything_has_an_empty_plan() {
        let full = |path: &str| {
            if path == CLAUDE_MD {
                Some(CLAUDE_RULES.to_string())
            } else {
                FILES
                    .iter()
                    .find(|a| a.path == path)
                    .map(|a| a.content.to_string())
            }
        };
        let plan = InstallPlan::new(&full, false);
        assert!(plan.is_empty());
        assert_eq!(plan.kept.len(), FILES.len() + 1);
    }
}
