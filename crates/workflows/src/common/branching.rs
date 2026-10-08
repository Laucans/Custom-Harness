//! The branch name a milestone pushes its tasks' PRs into.
//!
//! Shared because `split` writes it into each task's body (as a `branch:`
//! line naming the *task's own* branch, computed by the session, not this
//! module) while `init-repo`'s audit and `main_agent_merge` both need the
//! *milestone's own* branch name — derived, never asked of a session, so
//! the two sides can never disagree on what it is.

/// The branch a milestone's tasks merge into: `milestone/<number>-<slug>`.
#[must_use]
pub fn milestone_branch(number: u64, title: &str) -> String {
    format!("milestone/{number}-{}", slugify(title))
}

/// Lowercase, non-alphanumeric runs collapsed to one `-`, trimmed of
/// leading/trailing `-`.
fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_dash = false;
    for ch in text.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_title_becomes_a_lowercase_slug() {
        assert_eq!(
            milestone_branch(4, "Territory tooling"),
            "milestone/4-territory-tooling"
        );
    }

    #[test]
    fn punctuation_collapses_to_a_single_dash() {
        assert_eq!(
            slugify("Schema, seed & first DB-backed page"),
            "schema-seed-first-db-backed-page"
        );
    }

    #[test]
    fn leading_and_trailing_punctuation_is_trimmed() {
        assert_eq!(slugify("  — Territory tooling! —  "), "territory-tooling");
    }

    #[test]
    fn repeated_separators_do_not_repeat_dashes() {
        assert_eq!(slugify("a   b---c"), "a-b-c");
    }

    #[test]
    fn two_different_titles_never_collide_by_accident_in_the_common_case() {
        assert_ne!(
            milestone_branch(4, "Territory tooling"),
            milestone_branch(4, "Character creation")
        );
    }
}
