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
/// What a decision to repair once more leaves on the PR: the repairs before
/// it are no longer counted.
pub const DECIDED_MARKER: &str = "<!-- harness-decided: repair -->";
/// What a decision to merge as it is leaves on a PR whose checks still run.
///
/// Posted with [`MARKER`] and [`CLEAN`], it is the review that lets the PR
/// through, so the merge train takes it once green instead of asking for a
/// decision again.
pub const DECIDED_MERGE: &str = "<!-- harness-decided: merge -->";

/// How many repairs a blocking review gets before the harness decides.
pub const MAX_FIXES: usize = 2;

/// The line the review's summary ends on, as the prompt asks for it.
pub const VERDICT_LINE: &str = "VERDICT:";

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

    /// A blocking review its repairs did not clear: the harness decides.
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
    let fixes = repairs(comments);
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

/// The line the rules' fidelity reader ends its verdict with, followed by
/// `faithful` or `unfaithful`.
pub const FIDELITY: &str = "<!-- rules-fidelity: ";

/// Whether the rules' fidelity reader's last verdict on the PR is
/// `faithful`: the merge train takes it once green.
///
/// The reader judges and never merges — a session merges no pull request
/// (its `gh pr merge` is denied): the harness does, on green checks.
#[must_use]
pub fn judged_faithful(comments: &str) -> bool {
    comments
        .rfind(FIDELITY)
        .and_then(|at| comments.get(at.saturating_add(FIDELITY.len())..))
        .is_some_and(|verdict| verdict.starts_with("faithful"))
}

/// Whether the harness decided to merge the PR as it is: its last review is
/// that decision ([`DECIDED_MERGE`]) and no repair came after it.
///
/// What lets a pull request no agent review ever reads — the rules' task's,
/// judged by its own reader — be taken by the merge train once green.
#[must_use]
pub fn decided_to_merge(comments: &str) -> bool {
    status(comments) == Status::Clean
        && comments
            .rfind(MARKER)
            .and_then(|at| comments.get(at..))
            .is_some_and(|last_review| last_review.contains(DECIDED_MERGE))
}

/// How many repairs were attempted on a PR, whatever asked for them — a red
/// check, a blocking review, a conflict with the base. The cap
/// [`MAX_FIXES`] is on this count.
#[must_use]
pub fn repairs(comments: &str) -> usize {
    since_decision(comments).matches(FIX_MARKER).count()
}

/// How many repairs a PR has had in all, decisions to repair once more
/// included — what decides which model the next one runs on.
#[must_use]
pub fn repairs_in_all(comments: &str) -> usize {
    comments.matches(FIX_MARKER).count()
}

/// The comments since the last decision to repair once more: what the cap
/// counts in.
fn since_decision(comments: &str) -> &str {
    comments
        .rfind(DECIDED_MARKER)
        .and_then(|at| comments.get(at..))
        .unwrap_or(comments)
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
    fn a_decision_to_repair_once_more_counts_the_repairs_again() {
        let review = format!("{MARKER}\n{BLOCKING}");
        let used = format!("{FIX_MARKER} {review} {FIX_MARKER} {review}");
        assert_eq!(repairs(&used), 2);
        assert!(repairs_in_all(&used) >= repairs(&used));
        assert!(status(&used).exhausted());
        let decided = format!("{used} {DECIDED_MARKER} repair once more");
        assert_eq!(repairs(&decided), 0);
        let again = format!("{decided} {review}");
        assert!(status(&again).wants_a_fix(), "one more repair is owed");
    }

    #[test]
    fn repairs_are_counted_until_the_harness_decides() {
        let once = format!("{}{FIX_MARKER}{}", review(BLOCKING), review(BLOCKING));
        assert_eq!(status(&once), Status::Blocking { fixes: 1 });
        assert!(status(&once).wants_a_fix());
        let twice = format!("{once}{FIX_MARKER}{}", review(BLOCKING));
        assert!(status(&twice).exhausted());
        assert!(!status(&twice).wants_a_fix());
    }

    #[test]
    fn a_decision_to_merge_is_the_last_review_with_no_repair_after_it() {
        let decided = format!("{MARKER}\n{CLEAN}\n{DECIDED_MERGE}\nmerge as it is");
        assert!(decided_to_merge(&decided));
        assert!(
            !decided_to_merge(&review(CLEAN)),
            "a clean review is not a decision"
        );
        assert!(!decided_to_merge("nothing"));
        let repaired = format!("{decided}\n{FIX_MARKER}\n");
        assert!(
            !decided_to_merge(&repaired),
            "a repair after the decision owes a review"
        );
        let overruled = format!("{decided}\n{}", review(BLOCKING));
        assert!(!decided_to_merge(&overruled), "a later review decides");
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

    #[test]
    fn only_the_last_fidelity_verdict_counts_and_it_must_say_faithful() {
        let faithful = format!("{FIDELITY}faithful -->");
        let unfaithful = format!("{FIDELITY}unfaithful -->");
        assert!(judged_faithful(&faithful));
        assert!(!judged_faithful(&unfaithful));
        assert!(!judged_faithful(""));
        assert!(!judged_faithful("faithful, no marker"));
        assert!(judged_faithful(&format!("{unfaithful}\nfixed\n{faithful}")));
        assert!(!judged_faithful(&format!(
            "{faithful}\nnew finding\n{unfaithful}"
        )));
    }
}
