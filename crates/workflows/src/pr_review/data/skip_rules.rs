//! Which PRs the review skips without spending anything.
//!
//! One review per task, not per PR: `/code` and `/create-test` each open a
//! PR, and reviewing the test one would re-review the same change twice.
//! Branches `test/*` are skipped.
//!
//! Pure business: no I/O, no subprocesses.

use harness_core::domain::Pr;

use crate::pr_review::data::notes;

/// Why this PR doesn't need review, or `None`.
///
/// The four rules in one place: that's what makes them readable as a policy
/// rather than a series of early returns buried in orchestration. `force`
/// lifts them all.
#[must_use]
pub fn skip_reason(force: bool, base: &str, pr: &Pr, comments: &str) -> Option<String> {
    if force {
        return None;
    }
    let num = &pr.num;
    if pr.base != base {
        return Some(format!(
            "PR #{num} targets '{}', not '{base}' (--force to override)",
            pr.base
        ));
    }
    if pr.draft {
        return Some(format!("PR #{num} is a draft"));
    }
    if pr.head.starts_with("test/") {
        return Some(format!(
            "#{num} is the test PR for a change already reviewed on its /code \
             PR (--force to review it anyway)"
        ));
    }
    if comments.contains(notes::MARKER) {
        return Some(format!(
            "PR #{num} already carries an agent review (--force to redo)"
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr() -> Pr {
        Pr {
            num: "32".to_string(),
            base: "main_agent".to_string(),
            head: "feat/cities".to_string(),
            title: "feat(db): table cities".to_string(),
            url: String::new(),
            state: "OPEN".to_string(),
            draft: false,
            labels: Vec::new(),
        }
    }

    #[test]
    fn a_pr_targeting_the_wrong_branch_is_skipped() {
        let said = skip_reason(false, "autre-branche", &pr(), "").expect("un saut");
        assert!(said.contains("targets"));
    }

    #[test]
    fn a_draft_is_skipped() {
        let mut draft = pr();
        draft.draft = true;
        assert!(skip_reason(false, "main_agent", &draft, "").is_some());
    }

    #[test]
    fn a_test_branch_is_skipped_as_already_reviewed_on_its_code_pr() {
        let mut test_pr = pr();
        test_pr.head = "test/cities".to_string();
        let said = skip_reason(false, "main_agent", &test_pr, "").expect("un saut");
        assert!(said.contains("already reviewed"));
    }

    #[test]
    fn an_already_reviewed_pr_is_skipped() {
        let said = skip_reason(
            false,
            "main_agent",
            &pr(),
            "...\n<!-- agent-review -->\n...",
        )
        .expect("un saut");
        assert!(said.contains("already carries"));
    }

    #[test]
    fn a_pr_with_none_of_the_four_reasons_is_not_skipped() {
        assert!(skip_reason(false, "main_agent", &pr(), "nothing interesting").is_none());
    }

    #[test]
    fn force_lifts_every_rule() {
        let mut draft = pr();
        draft.draft = true;
        draft.base = "autre-branche".to_string();
        assert!(skip_reason(true, "main_agent", &draft, "<!-- agent-review -->").is_none());
    }
}
