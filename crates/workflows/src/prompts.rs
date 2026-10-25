//! Every prompt a human may edit: its key, the station it belongs to, and
//! its default.
//!
//! The defaults stay where they were written, beside the stage that sends
//! them; this module only names them. A stage asks its workflow's
//! `Config::prompts` for `text(KEY, DEFAULT)` — the human's edit if there is
//! one, the constant otherwise — so the key and the default are never two
//! literals that could drift apart.
//!
//! What is editable is a stage's **own** text. The blocks every session
//! receives — EXECUTION CONTEXT, SCOPE, the repository configuration — are
//! the framework's, filled in around it, and stay as they are.
//!
//! A key is `<line>/<prompt>`, the line named the way the view names it (the
//! folder its runs are logged under), so `harness-view` can place each prompt
//! on the station that sends it.

use crate::arbiter::orchestration::stages as arbiter;
use crate::common::explore;
use crate::contract_manager::orchestration::stages as contract_manager;
use crate::decide::orchestration::stages as decide;
use crate::dev_loop::orchestration::stages as dev_loop;
use crate::planner::orchestration::stages as planner;
use crate::pr_fix::orchestration::stages as pr_fix;
use crate::pr_review::orchestration::stages as pr_review;
use crate::refinement::orchestration::prompts as refinement;
use crate::rules::orchestration::stages as rules;
use crate::split::orchestration::stages as split;
use crate::test_debt::orchestration::stages as test_debt;

/// `code`'s instructions, for a task on the read side.
pub const CODE: &str = "agent-loop/code";
/// `code`'s instructions, for a task on the write side.
pub const CODE_WRITE_SIDE: &str = "agent-loop/code-write-side";
/// How the Rust tests are run — appended to `code`.
pub const TEST_SCOPE: &str = "agent-loop/test-scope";
/// The repository map, shared by refinement and planner.
pub const EXPLORE: &str = "explore";
/// Refinement's router.
pub const ROUTER: &str = "refinement/router";
/// Refinement's Business Goal section.
pub const BUSINESS_GOAL: &str = "refinement/business-goal";
/// Refinement's Technical section.
pub const TECHNICAL: &str = "refinement/technical";
/// Refinement's Acceptance Criteria section.
pub const ACCEPTANCE_CRITERIA: &str = "refinement/acceptance-criteria";
/// Refinement's Business Rules section.
pub const BUSINESS_RULES: &str = "refinement/business-rules";
/// Refinement's Technical Implementation Plan section.
pub const TECHNICAL_PLAN: &str = "refinement/technical-plan";
/// Refinement's coherence pass.
pub const COHERENCE: &str = "refinement/coherence";
/// Refinement's advice on the technical refinement.
pub const ADVICE: &str = "refinement/human-advice";
/// The graph pass of the technical refinement.
pub const GRAPH: &str = "refinement/graph-advice";
/// The planner's plan.
pub const PLAN: &str = "planner/plan";
/// Split's slice.
pub const SLICE: &str = "split/slice";
/// PR review's line-by-line pass.
pub const INLINE: &str = "pr-review/inline";
/// PR review's notes for the human.
pub const BRIEF: &str = "pr-review/brief";
/// PR fix's repair.
pub const FIX: &str = "pr-fix/fix";
/// Test debt's tests for the mutants of one file.
pub const COVER: &str = "test-debt/cover";
/// The rules' task: the rules and their tests.
pub const RULES_WRITE: &str = "rules/write";
/// The rules' task: a contested test settled.
pub const RULES_CONTEST: &str = "rules/contest";
/// The rules' task: each test judged against its criterion.
pub const RULES_FIDELITY: &str = "rules/fidelity";
/// The decision the harness takes where a human was asked.
pub const DECIDE: &str = "decide/decide";
/// The arbiter's one session: which task produces each open id.
pub const ARBITRATE: &str = "arbiter/arbitrate";
/// The contract manager's one session: the new version of a contract.
pub const CONTRACT_EVOLVE: &str = "contract-manager/evolve";

/// One editable prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prompt {
    /// What the edits are kept under.
    pub key: &'static str,
    /// What a human calls it.
    pub title: &'static str,
    /// Where it is sent from: `(line, station)`, the way the view names them.
    /// Several when a text is shared.
    pub stations: &'static [(&'static str, &'static str)],
    /// The text the stage sends until a human edits it.
    pub default: &'static str,
}

/// Every editable prompt, line by line.
const CATALOG: &[Prompt] = &[
    Prompt {
        key: CODE,
        title: "Code — read side",
        stations: &[("agent-loop", "code")],
        default: dev_loop::CODE,
    },
    Prompt {
        key: CODE_WRITE_SIDE,
        title: "Code — write side (merged once reviewed and green)",
        stations: &[("agent-loop", "code")],
        default: dev_loop::CODE_WRITE_SIDE,
    },
    Prompt {
        key: TEST_SCOPE,
        title: "How the tests are run",
        stations: &[("agent-loop", "code")],
        default: dev_loop::TEST_SCOPE,
    },
    Prompt {
        key: EXPLORE,
        title: "Repository map (refinement and planner)",
        stations: &[("refinement", "explore"), ("planner", "explore")],
        default: explore::EXPLORE_PROMPT,
    },
    Prompt {
        key: ROUTER,
        title: "Router",
        stations: &[("refinement", "router")],
        default: refinement::ROUTER_PROMPT,
    },
    Prompt {
        key: BUSINESS_GOAL,
        title: "Business Goal",
        stations: &[("refinement", "sections")],
        default: refinement::BUSINESS_GOAL_PROMPT,
    },
    Prompt {
        key: TECHNICAL,
        title: "Technical",
        stations: &[("refinement", "sections")],
        default: refinement::TECHNICAL_PROMPT,
    },
    Prompt {
        key: ACCEPTANCE_CRITERIA,
        title: "Acceptance Criteria",
        stations: &[("refinement", "sections")],
        default: refinement::ACCEPTANCE_CRITERIA_PROMPT,
    },
    Prompt {
        key: BUSINESS_RULES,
        title: "Business Rules",
        stations: &[("refinement", "sections")],
        default: refinement::BUSINESS_RULES_PROMPT,
    },
    Prompt {
        key: TECHNICAL_PLAN,
        title: "Technical Implementation Plan",
        stations: &[("refinement", "sections")],
        default: refinement::TECHNICAL_PLAN_PROMPT,
    },
    Prompt {
        key: COHERENCE,
        title: "Coherence",
        stations: &[("refinement", "coherence")],
        default: refinement::COHERENCE_PROMPT,
    },
    Prompt {
        key: ADVICE,
        title: "Advice on the technical refinement",
        stations: &[("refinement", "human-advice")],
        default: refinement::ADVICE_PROMPT,
    },
    Prompt {
        key: GRAPH,
        title: "Graph pass: does the design hold the task's Architecture?",
        stations: &[("refinement", "graph-advice")],
        default: refinement::GRAPH_PROMPT,
    },
    Prompt {
        key: PLAN,
        title: "Plan",
        stations: &[("planner", "plan")],
        default: planner::PLAN_PROMPT,
    },
    Prompt {
        key: SLICE,
        title: "Slice",
        stations: &[("split", "slice")],
        default: split::SLICE_PROMPT,
    },
    Prompt {
        key: INLINE,
        title: "Line-by-line review",
        stations: &[("pr-review", "inline")],
        default: pr_review::INLINE_PROMPT,
    },
    Prompt {
        key: BRIEF,
        title: "Notes for the human",
        stations: &[("pr-review", "brief")],
        default: pr_review::BRIEF_PROMPT,
    },
    Prompt {
        key: FIX,
        title: "Repair",
        stations: &[("pr-fix", "fix")],
        default: pr_fix::FIX_PROMPT,
    },
    Prompt {
        key: COVER,
        title: "Tests for the mutants",
        stations: &[("test-debt", "cover")],
        default: test_debt::COVER_PROMPT,
    },
    Prompt {
        key: RULES_WRITE,
        title: "The rules and their tests",
        stations: &[("rules", "write")],
        default: rules::WRITE_PROMPT,
    },
    Prompt {
        key: RULES_CONTEST,
        title: "A contested rule's test, settled",
        stations: &[("rules", "write")],
        default: rules::CONTEST_PROMPT,
    },
    Prompt {
        key: RULES_FIDELITY,
        title: "Each rule's test judged against its criterion",
        stations: &[("rules", "fidelity")],
        default: rules::FIDELITY_PROMPT,
    },
    Prompt {
        key: DECIDE,
        title: "A decision taken where a human was asked",
        stations: &[("decide", "decide")],
        default: decide::DECIDE_PROMPT,
    },
    Prompt {
        key: ARBITRATE,
        title: "Arbiter: which task produces what a refinement found missing",
        stations: &[("arbiter", "arbitrate")],
        default: arbiter::ARBITRATE_PROMPT,
    },
    Prompt {
        key: CONTRACT_EVOLVE,
        title: "Contract manager: the new version of a contract a consumer needs",
        stations: &[("contract-manager", "evolve")],
        default: contract_manager::EVOLVE_PROMPT,
    },
];

/// Every editable prompt, line by line.
#[must_use]
pub const fn catalog() -> &'static [Prompt] {
    CATALOG
}

/// One prompt by its key.
#[must_use]
pub fn find(key: &str) -> Option<Prompt> {
    catalog().iter().find(|prompt| prompt.key == key).copied()
}

/// The default of one prompt, by its key — what the harness needs to apply a
/// draft.
#[must_use]
pub fn default_of(key: &str) -> Option<String> {
    find(key).map(|prompt| prompt.default.to_string())
}

/// The prompts a line's stations send, in catalog order.
#[must_use]
pub fn of_line(line: &str) -> Vec<Prompt> {
    catalog()
        .iter()
        .filter(|prompt| prompt.stations.iter().any(|(at, _)| *at == line))
        .copied()
        .collect()
}

/// The prompts one station sends, in catalog order.
#[must_use]
pub fn of_station(line: &str, station: &str) -> Vec<Prompt> {
    catalog()
        .iter()
        .filter(|prompt| prompt.stations.contains(&(line, station)))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_station_of_the_plant_knows_the_rules_come_first() {
        for (name, text, says) in [
            (
                "plan",
                planner::PLAN_PROMPT,
                &[
                    "crates/server",
                    "apps/shell",
                    "{dossier}",
                    "rules.json",
                    "\"rules\": [\"R3\"]",
                ][..],
            ),
            (
                "slice",
                split::SLICE_PROMPT,
                &[
                    "## Rules",
                    "crates/server",
                    "apps/shell",
                    "\"rules\": [\"R3\"]",
                    "\"acceptance\"",
                    "{dossier}",
                    "never invented",
                ][..],
            ),
            (
                "code",
                dev_loop::CODE,
                &[
                    "## Rules",
                    "rules-runtime.sh serve",
                    "CONTEST.md",
                    "Never change",
                    "rules-hold proves them",
                    "never stop on it",
                    "rm -f target/rules-status.jsonl",
                    "rules-gate.mjs status",
                    "it says mis-mounted",
                ][..],
            ),
            (
                "code write side",
                dev_loop::CODE_WRITE_SIDE,
                &[
                    "## Rules",
                    "rules-runtime.sh serve",
                    "CONTEST.md",
                    "Never change",
                    "rm -f target/rules-status.jsonl",
                    "rules-gate.mjs status",
                    "it says mis-mounted",
                ][..],
            ),
            (
                "fix",
                pr_fix::FIX_PROMPT,
                &["rules-hold", "CONTEST.md", "tests/rules/", "mis-mounted"][..],
            ),
            (
                "acceptance",
                refinement::ACCEPTANCE_CRITERIA_PROMPT,
                &["## Rules", "contradicts"][..],
            ),
            (
                "brief",
                pr_review::BRIEF_PROMPT,
                &["tests/rules/", "rules-hold"][..],
            ),
        ] {
            for said in says {
                assert!(text.contains(said), "{name} says {said}");
            }
        }
    }

    #[test]
    fn every_key_is_unique_and_every_default_says_something() {
        let all = catalog();
        let keys: BTreeSet<&str> = all.iter().map(|p| p.key).collect();
        assert_eq!(keys.len(), all.len());
        assert!(
            all.iter()
                .all(|p| !p.default.trim().is_empty() && !p.stations.is_empty())
        );
    }

    #[test]
    fn every_default_can_be_replayed_with_another_text() {
        // A dry-run reads a past prompt's values back from between the prose
        // of the text it was sent with: a default starting or ending with a
        // placeholder, or carrying two in a row, would hide where one ends.
        use harness_core::domain::prompt_book::{placeholders, replay};
        for prompt in catalog() {
            let names = placeholders(prompt.default);
            let values: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "V")).collect();
            let filled = harness_core::domain::prompts::splice(prompt.default, &values);
            assert!(
                replay(&filled, prompt.default, prompt.default).is_ok(),
                "{} cannot be replayed",
                prompt.key
            );
        }
    }

    #[test]
    fn a_station_lists_its_prompts_and_a_shared_one_is_on_both() {
        let code: Vec<&str> = of_station("agent-loop", "code")
            .iter()
            .map(|p| p.key)
            .collect();
        assert_eq!(code, [CODE, CODE_WRITE_SIDE, TEST_SCOPE]);
        assert_eq!(of_station("planner", "explore")[0].key, EXPLORE);
        assert!(find("nope").is_none());
        let planner: Vec<&str> = of_line("planner").iter().map(|p| p.key).collect();
        assert_eq!(planner, [EXPLORE, PLAN]);
        assert_eq!(default_of(FIX).as_deref(), Some(pr_fix::FIX_PROMPT));
        let planner: Vec<&str> = of_line("planner").iter().map(|p| p.key).collect();
        assert_eq!(planner, [EXPLORE, PLAN]);
        assert_eq!(default_of(FIX).as_deref(), Some(pr_fix::FIX_PROMPT));
    }
}
