//! Where a pull request stands with its agent review: read from its comments
//! alone, so the review, the repair and the merge judge it the same way.
//!
//! The review cannot be a GitHub review state: the account that opens the PR
//! is the one that reviews it, and GitHub refuses an approval or a request
//! for changes on one's own PR. So the verdict is a marker inside the review
//! comment, and every repair attempt leaves a marker of its own — the order
//! of the two is the whole story.

/// What opens every review comment.
pub const MARKER: &str = "<!-- agent-review -->";

/// The review asks for changes before the PR is merged.
pub const BLOCKING: &str = "<!-- agent-review-verdict: blocking -->";

/// The review lets the PR through.
pub const CLEAN: &str = "<!-- agent-review-verdict: clean -->";

/// What every repair attempt leaves on the PR, whatever it pushed.
pub const FIX_MARKER: &str = "<!-- agent-fix -->";

/// How many repairs a blocking review gets before a human decides.
pub const MAX_FIXES: usize = 2;

/// The line the review's summary ends on, as the prompt asks for it.
const VERDICT_LINE: &str = "VERDICT:";

/// Where a PR stands with its review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No review yet, or a repair was pushed since the last one: it is owed
    /// a review.
    Unreviewed,
    /// The last review lets it through.
    Clean,
    /// The last review asks for changes; `fixes` repairs were attempted so far.
    Blocking {
        /// Repair attempts on this PR so far.
        fixes: usize,
    },
}

impl Status {
    /// A blocking review whose repairs are not used up yet.
    #[must_use]
    pub const fn wants_a_fix(self) -> bool {
        matches!(self, Self::Blocking { fixes } if fixes < MAX_FIXES)
    }

    /// A blocking review its repairs did not clear: a human decides.
    #[must_use]
    pub const fn exhausted(self) -> bool {
        matches!(self, Self::Blocking { fixes } if fixes >= MAX_FIXES)
    }
}

/// Where a PR stands, from all its comments in the order they were posted.
///
/// The last review decides, unless a repair came after it. A review with no
/// verdict marker — one posted before verdicts existed — lets the PR
/// through, as every review did then.
#[must_use]
pub fn status(comments: &str) -> Status {
    let Some(review_at) = comments.rfind(MARKER) else {
        return Status::Unreviewed;
    };
    let fixes = comments.matches(FIX_MARKER).count();
    if comments
        .rfind(FIX_MARKER)
        .is_some_and(|fix_at| fix_at > review_at)
    {
        return Status::Unreviewed;
    }
    let last_review = comments.get(review_at..).unwrap_or_default();
    if last_review.contains(BLOCKING) {
        Status::Blocking { fixes }
    } else {
        Status::Clean
    }
}

/// How many repairs were attempted on a PR, whatever asked for them — a red
/// check, a blocking review, a conflict with the base. The cap
/// [`MAX_FIXES`] is on this count.
#[must_use]
pub fn repairs(comments: &str) -> usize {
    comments.matches(FIX_MARKER).count()
}

/// Whether a review summary asks for changes: its `VERDICT:` says `blocking`.
///
/// A summary with no such line does not block — it is a review that forgot
/// its verdict, and holding the PR on that would hold it forever.
#[must_use]
pub fn is_blocking(summary: &str) -> bool {
    summary
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix(VERDICT_LINE))
        .is_some_and(|verdict| verdict.trim().to_lowercase().starts_with("blocking"))
}

/// The marker a review comment carries for its summary's verdict.
#[must_use]
pub fn verdict_marker(summary: &str) -> &'static str {
    if is_blocking(summary) {
        BLOCKING
    } else {
        CLEAN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_are_counted_whatever_asked_for_them() {
        assert_eq!(repairs("nothing"), 0);
        assert_eq!(repairs(&format!("{FIX_MARKER}\n...\n{FIX_MARKER}")), 2);
    }

    fn review(verdict: &str) -> String {
        format!("{MARKER}\n## notes\n\n...\n{verdict}\n")
    }

    #[test]
    fn a_pr_with_no_review_is_unreviewed() {
        assert_eq!(status("a human said hi"), Status::Unreviewed);
    }

    #[test]
    fn the_last_review_decides() {
        let comments = format!("{}{}", review(BLOCKING), review(CLEAN));
        assert_eq!(status(&comments), Status::Clean);
        let comments = format!("{}{}", review(CLEAN), review(BLOCKING));
        assert_eq!(status(&comments), Status::Blocking { fixes: 0 });
    }

    #[test]
    fn a_repair_after_the_review_owes_it_a_new_review() {
        let comments = format!("{}{FIX_MARKER}\n", review(BLOCKING));
        assert_eq!(status(&comments), Status::Unreviewed);
    }

    #[test]
    fn repairs_are_counted_until_a_human_decides() {
        let once = format!("{}{FIX_MARKER}{}", review(BLOCKING), review(BLOCKING));
        assert_eq!(status(&once), Status::Blocking { fixes: 1 });
        assert!(status(&once).wants_a_fix());
        let twice = format!("{once}{FIX_MARKER}{}", review(BLOCKING));
        assert!(status(&twice).exhausted());
        assert!(!status(&twice).wants_a_fix());
    }

    #[test]
    fn a_review_from_before_verdicts_lets_the_pr_through() {
        assert_eq!(status(&review("")), Status::Clean);
    }

    #[test]
    fn the_verdict_is_the_summary_s_last_verdict_line() {
        assert!(is_blocking(
            "findings...\nVERDICT: blocking — the gate is bypassable"
        ));
        assert!(!is_blocking("findings...\nVERDICT: clean"));
        assert!(!is_blocking("no verdict at all"));
        assert_eq!(verdict_marker("VERDICT: Blocking"), BLOCKING);
    }
}
