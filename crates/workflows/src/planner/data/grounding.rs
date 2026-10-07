//! What the planner is told about the project ahead of time.
//!
//! Assembled from the two **digests** that `grill-to-roadmap` and
//! `technical-grill-with-docs` write (`.claude/skills/`, project-level) —
//! never from the ADRs or glossary themselves, which stay for a human to
//! read directly and would blow the planner's prompt budget if spliced in
//! whole. Pure: takes what was already read, decides nothing about the
//! filesystem.
//!
//! Carried over from `dev_loop`'s former `planner` stage: the behavior moved
//! here when that stage was replaced by this standalone, independently
//! triggered workflow (see `dev_loop::orchestration::round`'s module doc).

use std::fmt::Write as _;

/// Small on purpose: a digest, not the ADRs. About 1k tokens.
const BUDGET: usize = 4000;

/// Appended when the combined text ran over budget.
const CUT: &str = "\n[... truncated: the grounding digest ran over its budget ...]";

/// Combines the two digests into one block for the planner's prompt, or an
/// empty string if neither exists — the normal case, and not an error: most
/// repositories have not been grilled yet.
#[must_use]
pub fn combine(business: Option<&str>, technical: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(text) = business {
        let _ = write!(out, "Business constraints:\n{text}");
    }
    if let Some(text) = technical {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        let _ = write!(out, "Technical constraints:\n{text}");
    }
    if out.chars().count() > BUDGET {
        let mut truncated: String = out.chars().take(BUDGET).collect();
        truncated.push_str(CUT);
        truncated
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neither_digest_yields_an_empty_block() {
        assert_eq!(combine(None, None), "");
    }

    #[test]
    fn only_business_is_labelled_and_alone() {
        let out = combine(Some("ship fast"), None);
        assert_eq!(out, "Business constraints:\nship fast");
    }

    #[test]
    fn only_technical_is_labelled_and_alone() {
        let out = combine(None, Some("no new deps"));
        assert_eq!(out, "Technical constraints:\nno new deps");
    }

    #[test]
    fn both_are_labelled_and_separated() {
        let out = combine(Some("ship fast"), Some("no new deps"));
        assert_eq!(
            out,
            "Business constraints:\nship fast\n\nTechnical constraints:\nno new deps"
        );
    }

    #[test]
    fn an_oversized_combination_is_truncated_with_a_notice() {
        let huge = "x".repeat(BUDGET + 500);
        let out = combine(Some(&huge), None);
        assert!(out.len() < huge.len());
        assert!(out.ends_with(CUT));
    }
}
