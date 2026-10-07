//! The read-only audit criteria, over text already read.
//!
//! Pure: every function here takes an `Option<&str>` that already separates
//! "the file is absent" from "the read failed" — a failed read never
//! reaches these functions at all, it halts the run before an [`Audit`] is
//! built (see [`crate::init_repo::action::apply`]).

use crate::common::labels::Label;

/// A label outside the `harness:*` namespace.
///
/// `init-repo` creates it without knowing what it means — only
/// `grill-to-roadmap` does, and that is deliberate. It stays out
/// of `common::labels::ALL` so the loop, which reads that list, never sees
/// it.
pub const GRILL_BACKLOG: Label = Label {
    name: "grill:backlog",
    color: "bfdadc",
    description: "Ideas deferred during a grilling session, not a roadmap item",
};

/// What the read-only audit found.
pub struct Audit {
    /// `.github/workflows/ci.yml` triggers on the integration branch.
    pub ci_triggers: bool,
    /// `.github/workflows/ci.yml` also triggers on milestone branches
    /// (`milestone/**`) — needed once a task's PR targets one instead of
    /// the integration branch directly.
    pub ci_triggers_on_milestones: bool,
    /// The repo carries a `CLAUDE.md` for a session to read.
    ///
    /// Advisory, not blocking: every workflow runs without one. But the
    /// repository map (`common::explore`) reads it first, so a repo without
    /// one has a planner working from the roadmap issue alone — worth
    /// knowing before paying for that run, not worth refusing it.
    ///
    /// Replaced an audit of the `pr-review` hook in `.claude/settings.json`:
    /// that hook was abandoned (it needs a permanently reachable public
    /// URL), the review is triggered by `harness:to-review` instead, and an
    /// advisory pointing at a mechanism that no longer exists sent a human
    /// to build the wrong thing.
    pub has_claude_md: bool,
}

/// Whether `ci.yml` triggers on `branch`.
///
/// Same criterion as the launcher's gate (`CiTriggersOnTheBranch`) by
/// construction — a `.contains(branch)` substring test. Kept as a one-line
/// duplicate rather than a new cross-crate shared function: not worth a new
/// public surface on either crate for one line.
#[must_use]
pub fn ci_triggers_on(ci_yml: Option<&str>, branch: &str) -> bool {
    ci_yml.is_some_and(|text| text.contains(branch))
}

/// Whether `ci.yml` also triggers on milestone branches.
///
/// Same substring nature as [`ci_triggers_on`]: `"milestone/"` is the fixed
/// prefix every milestone branch carries (`milestone/<n>-<slug>`).
#[must_use]
pub fn ci_triggers_on_milestones(ci_yml: Option<&str>) -> bool {
    ci_yml.is_some_and(|text| text.contains("milestone/"))
}

/// Whether the repo carries a `CLAUDE.md` worth reading.
///
/// Present but empty counts as absent: the repository map would read it and
/// learn nothing, which is the state this reports.
#[must_use]
pub fn has_claude_md(claude_md: Option<&str>) -> bool {
    claude_md.is_some_and(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_404_on_ci_yml_reports_missing() {
        assert!(!ci_triggers_on(None, "main_agent"));
    }

    #[test]
    fn a_ci_yml_naming_the_branch_triggers() {
        assert!(ci_triggers_on(
            Some("on:\n  push:\n    branches: [main_agent]\n"),
            "main_agent"
        ));
    }

    #[test]
    fn a_ci_yml_naming_another_branch_does_not_trigger() {
        assert!(!ci_triggers_on(
            Some("on:\n  push:\n    branches: [main]\n"),
            "main_agent"
        ));
    }

    #[test]
    fn a_404_on_ci_yml_reports_no_milestone_trigger() {
        assert!(!ci_triggers_on_milestones(None));
    }

    #[test]
    fn a_ci_yml_naming_milestone_branches_triggers() {
        assert!(ci_triggers_on_milestones(Some(
            "on:\n  push:\n    branches: [main_agent, 'milestone/**']\n"
        )));
    }

    #[test]
    fn a_ci_yml_without_the_milestone_prefix_does_not_trigger() {
        assert!(!ci_triggers_on_milestones(Some(
            "on:\n  push:\n    branches: [main_agent]\n"
        )));
    }

    #[test]
    fn a_repo_without_a_claude_md_is_reported_as_such() {
        assert!(!has_claude_md(None));
    }

    #[test]
    fn an_empty_claude_md_counts_as_absent() {
        // The map would read it and learn nothing — the same situation, and
        // the advisory exists to say so.
        assert!(!has_claude_md(Some("   \n\n")));
    }

    #[test]
    fn a_claude_md_with_content_is_clean() {
        assert!(has_claude_md(Some("# project\n\nwhat it is.\n")));
    }
}
