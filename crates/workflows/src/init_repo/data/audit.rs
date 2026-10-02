//! The read-only audit criteria, over text already read.
//!
//! Pure: every function here takes an `Option<&str>` that already separates
//! "the file is absent" from "the read failed" — a failed read never
//! reaches these functions at all, it halts the run before an [`Audit`] is
//! built (see [`crate::init_repo::action::apply`]).

/// The five skills `dev_loop/run.rs` names, kept textually identical here on
/// purpose: a second list risks drifting from the one preflight actually
/// checks.
pub const SKILLS: [&str; 5] = [
    "business-analyst",
    "code",
    "create-test",
    "planner",
    "tech-analyst",
];

/// What the read-only audit found.
pub struct Audit {
    /// `.github/workflows/ci.yml` triggers on the integration branch.
    pub ci_triggers: bool,
    /// The skills missing from `.claude/skills/<skill>/SKILL.md`.
    pub missing_skills: Vec<&'static str>,
    /// `.claude/settings.json` declares the `pr-review` hook.
    pub pr_review_hook: bool,
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

/// Whether `.claude/settings.json` names the `pr-review` hook.
#[must_use]
pub fn declares_pr_review_hook(settings_json: Option<&str>) -> bool {
    settings_json.is_some_and(|text| text.contains("pr-review"))
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
    fn a_missing_settings_file_has_no_hook() {
        assert!(!declares_pr_review_hook(None));
    }

    #[test]
    fn a_settings_file_naming_the_hook_is_clean() {
        assert!(declares_pr_review_hook(Some(
            r#"{"hooks":{"PostToolUse":[{"hooks":[{"command":"pr-review"}]}]}}"#
        )));
    }
}
