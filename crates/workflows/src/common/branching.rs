//! The branch name a milestone pushes its tasks' PRs into.
//!
//! Shared because `split` writes it into each task's body (as a `branch:`
//! line naming the *task's own* branch, computed by the session, not this
//! module) while `init-repo`'s audit and `main_agent_merge` both need the
//! *milestone's own* branch name — derived, never asked of a session, so
//! the two sides can never disagree on what it is. And what keeps that
//! branch up to the integration branch ([`catch_up`]).

use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Logbook;

/// Brings a milestone's branch up to the integration branch before its
/// tasks are handed out.
///
/// A fix pushed on the integration branch — a file the harness upgraded,
/// a milestone that landed after this one was cut — reaches the tasks of a
/// milestone already open: a milestone cut before the one before it landed
/// had tasks that found no Data layer. A refusal (a conflict) is said and
/// never forced: the milestone's own merge to the integration branch meets
/// it there. Whether the branch holds the integration branch now — a task
/// started on a branch that could not take it stops on what the base has
/// and it lacks (`ForgeDesMondes`, 2026-10-20: five sessions in one tick).
pub async fn catch_up(
    gh: &dyn GitHub,
    milestone_branch: &str,
    integration: &str,
    log: &Logbook,
) -> bool {
    match gh.merge_branch(milestone_branch, integration).await {
        Ok(()) => {
            log.debug(&format!("{milestone_branch} holds {integration}"));
            true
        }
        Err(halt) => {
            log.warn(&format!(
                "{milestone_branch} cannot take {integration}: {}",
                halt.reason()
            ));
            false
        }
    }
}

/// The branch a milestone's tasks merge into: `milestone/<number>-<slug>`.
#[must_use]
pub fn milestone_branch(number: u64, title: &str) -> String {
    format!("milestone/{number}-{}", slugify(title))
}

/// The milestone a branch of [`milestone_branch`]'s shape names, if it is
/// one: `milestone/18-campaigns` is #18's; `main_agent` is nobody's.
#[must_use]
pub fn milestone_number(branch: &str) -> Option<u64> {
    branch
        .strip_prefix("milestone/")?
        .split('-')
        .next()?
        .parse()
        .ok()
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
    use crate::common::fake_github::{FakeGitHub, Wrote};

    #[tokio::test]
    async fn a_milestone_branch_takes_the_integration_branch_in() {
        let gh = FakeGitHub::default();
        assert!(catch_up(&gh, "milestone/17-ai", "main_agent", &Logbook::null()).await);
        assert!(
            gh.wrote.borrow().iter().any(|w| matches!(
                w,
                Wrote::MergedBranch(base, head) if base == "milestone/17-ai" && head == "main_agent"
            )),
            "{:?}",
            gh.wrote.borrow()
        );
    }

    #[tokio::test]
    async fn a_merge_github_refuses_is_said_and_answered_false() {
        let gh = FakeGitHub {
            refused_merges: vec!["milestone/20-sessions".to_string()],
            ..FakeGitHub::default()
        };
        assert!(
            !catch_up(&gh, "milestone/20-sessions", "main_agent", &Logbook::null()).await,
            "a conflict: the branch does not hold the base"
        );
        assert!(gh.wrote.borrow().is_empty(), "{:?}", gh.wrote.borrow());
    }

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
