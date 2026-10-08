//! The text the review publishes on the PR — the review's only business.
//!
//! Everything is here because everything is published: the marker saying a
//! PR already has one, and the footer. Nothing reads a file or calls a
//! binary, so everything can be re-read and changed without running anything.
//!
//! This text **remains in French**: it is published on a GitHub PR for a
//! French-speaking human.

use harness_core::domain::prompts::splice;

/// What says a PR already has a review from this harness.
pub use crate::common::review::MARKER;

const FOOTER: &str = "_Automated review (`harness`, workflow `pr_review`) — {passes}{cost}. \
Its last line is its verdict: a blocking one sends a repair before the merge. \
Line-by-line findings are in the **Files changed** tab._";

/// The footer, with passes and cost substituted.
#[must_use]
pub fn footer(passes: &str, cost: &str) -> String {
    splice(FOOTER, &[("passes", passes), ("cost", cost)])
}

/// The complete comment, as posted.
#[must_use]
pub fn comment(stamp: &str, body: &str, footer: &str) -> String {
    let verdict = crate::common::review::verdict_marker(body);
    format!("{MARKER}\n## 🤖 Review notes — {stamp}\n\n{body}\n\n---\n{footer}\n{verdict}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_comment_carries_the_summary_s_verdict() {
        let blocking = comment("t", "notes\nVERDICT: blocking — bypassable gate", "f");
        assert!(blocking.contains(crate::common::review::BLOCKING));
        let clean = comment("t", "notes\nVERDICT: clean", "f");
        assert!(clean.contains(crate::common::review::CLEAN));
    }

    #[test]
    fn the_footer_substitutes_both_fields() {
        let said = footer(
            "findings `sonnet`/level `medium`, notes `sonnet`",
            ", cost $0.42",
        );
        assert!(said.contains("findings `sonnet`"));
        assert!(said.contains("cost $0.42"));
    }

    #[test]
    fn the_comment_carries_the_marker_first_so_a_later_run_can_find_it() {
        let said = comment("2026-10-02 16:00", "the body", "the footer");
        assert!(said.starts_with(MARKER));
        assert!(said.contains("the body"));
        assert!(said.contains("the footer"));
    }
}
