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

use crate::common::explore;
use crate::dev_loop::orchestration::stages as dev_loop;
use crate::planner::orchestration::stages as planner;
use crate::pr_fix::orchestration::stages as pr_fix;
use crate::pr_review::orchestration::stages as pr_review;
use crate::refinement::orchestration::prompts as refinement;
use crate::split::orchestration::stages as split;

/// `technical-refinement`'s instructions.
pub const TECHNICAL_REFINEMENT: &str = "agent-loop/technical-refinement";
/// `code`'s instructions, for a task on the read side.
pub const CODE: &str = "agent-loop/code";
/// `code`'s instructions, for a task on the write side.
pub const CODE_WRITE_SIDE: &str = "agent-loop/code-write-side";
/// How the Rust tests are run — appended to `code`, the whole of `create-test`.
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
        key: TECHNICAL_REFINEMENT,
        title: "Technical refinement",
        stations: &[("agent-loop", "technical-refinement")],
        default: dev_loop::TECHNICAL_REFINEMENT,
    },
    Prompt {
        key: CODE,
        title: "Code — read side",
        stations: &[("agent-loop", "code")],
        default: dev_loop::CODE,
    },
    Prompt {
        key: CODE_WRITE_SIDE,
        title: "Code — write side (a human merges)",
        stations: &[("agent-loop", "code")],
        default: dev_loop::CODE_WRITE_SIDE,
    },
    Prompt {
        key: TEST_SCOPE,
        title: "How the tests are run (code and create-test)",
        stations: &[("agent-loop", "code"), ("agent-loop", "create-test")],
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
    fn a_station_lists_its_prompts_and_a_shared_one_is_on_both() {
        let code: Vec<&str> = of_station("agent-loop", "code")
            .iter()
            .map(|p| p.key)
            .collect();
        assert_eq!(code, [CODE, CODE_WRITE_SIDE, TEST_SCOPE]);
        assert_eq!(of_station("planner", "explore")[0].key, EXPLORE);
        assert!(find("nope").is_none());
    }
}
