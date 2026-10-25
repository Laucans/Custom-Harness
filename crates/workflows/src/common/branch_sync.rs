//! Whether a decision tells a session to sync a branch itself — pure.
//!
//! The harness owns the catch-up: a milestone branch takes the integration
//! branch (`common::catch_up`), and a task PR is refreshed with its
//! milestone before it lands. A decision that tells a session to merge the
//! integration branch into its branch brings it through a task PR and races
//! the catch-up (`ForgeDesMondes`' #486, 18:04: "merge `origin/main_agent`
//! into the milestone branch"; #219, 22:09: "the session merges
//! `origin/main_agent` into its branch itself"). [`prescribed`] finds such a
//! sentence; `decide` refuses the answer that holds one, once with the
//! reason, and writes nothing on the second.
//!
//! **What is never caught**: a PR merging **its own base** into its head —
//! the repair's one allowed merge, to resolve a conflict; for a landing PR
//! its base *is* the integration branch — and a push to the PR's own head.
//! A gesture negated just before its verb (`do not merge main_agent`,
//! `never rebase`) or done by the harness (`the harness merges main_agent`,
//! `the harness's catch-up`) prescribes nothing either; a description
//! (`was merged into main_agent`, `the rules pull request into main_agent`,
//! `the merge train`) is no gesture.
//!
//! **Read by position, not by presence**: a negation or the harness
//! anywhere else in the clause clears nothing — `merge origin/main_agent
//! into milestone/24-table, no other change` and `the session merges
//! origin/main_agent since the harness's catch-up is stuck` are caught.

/// How many words before a verb a negation still negates it.
const NEGATION_REACH: usize = 3;

/// How many words before a verb the harness is still its subject
/// (`the harness merges`, `the harness then merges`).
const SUBJECT_REACH: usize = 2;

/// The words that negate the verb right after them, lower-cased.
const NEGATIONS: [&str; 8] = [
    "not", "never", "don't", "dont", "no", "without", "nor", "neither",
];

/// The words that name the harness as the subject of the verb after them.
const HARNESS: [&str; 2] = ["harness", "harness's"];

/// The words after `merge` or `pull` that make it a noun, not a gesture.
const NOUNS_AFTER: [&str; 8] = [
    "train",
    "trains",
    "request",
    "requests",
    "queue",
    "conflict",
    "conflicts",
    "commit",
];

/// The verbs that bring a branch up to date (`bring … up to date`).
const BRINGS: [&str; 6] = ["bring", "brings", "get", "gets", "make", "makes"];

/// What a PR is, as far as the guard reads it: its head and its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Own<'a> {
    /// The PR's head branch — a push to it is the repair itself.
    pub head: &'a str,
    /// The PR's base — merging it into the head is the repair's own merge.
    pub base: &'a str,
}

/// The first clause of `text` that prescribes a branch sync, if any.
///
/// A merge, pull, rebase,
/// cherry-pick or catch-up of `integration` into a branch, or a push to a
/// `milestone/*` branch — `None` when there is none. `own` is the PR the
/// decision is about, when it has one: merging its own base and pushing its
/// own head are allowed.
#[must_use]
pub fn prescribed(text: &str, integration: &str, own: Option<Own<'_>>) -> Option<String> {
    clauses(text)
        .into_iter()
        .find(|clause| prescribes(clause, integration, own))
        .map(|clause| clause.trim().to_string())
}

/// The clauses of a text: its lines, sentences and `;`-separated parts.
fn clauses(text: &str) -> Vec<&str> {
    text.split(['\n', ';'])
        .flat_map(|line| line.split(". "))
        .filter(|clause| !clause.trim().is_empty())
        .collect()
}

/// Whether one clause prescribes a sync.
fn prescribes(clause: &str, integration: &str, own: Option<Own<'_>>) -> bool {
    // A typographic apostrophe reads as the plain one (`don’t`).
    let lower = clause.to_lowercase().replace('\u{2019}', "'");
    let tokens: Vec<&str> = tokens(&lower).collect();
    let integration = integration.to_lowercase();
    let own_base = own.is_some_and(|own| own.base.eq_ignore_ascii_case(&integration));
    if !own_base
        && names(&tokens, &integration)
        && sync_verbs(&tokens).any(|at| !cleared(&tokens, at))
    {
        return true;
    }
    pushes_elsewhere(&tokens, own)
}

/// Whether a push of the clause targets a `milestone/*` branch other than
/// the PR's own head: the first such branch named after an un-negated
/// `push`, within a few words (`push the fix to origin/milestone/3-x`,
/// `push HEAD:milestone/3-x`). A base named only as a merge source before
/// the push (`merge origin/milestone/3-x in, then push`) is no target.
fn pushes_elsewhere(tokens: &[&str], own: Option<Own<'_>>) -> bool {
    tokens.iter().enumerate().any(|(at, word)| {
        matches!(*word, "push" | "pushes" | "pushing")
            && !negated(tokens, at)
            && tokens
                .iter()
                .skip(at.saturating_add(1))
                .take(6)
                .map(|token| token.strip_prefix("origin/").unwrap_or(token))
                .find(|token| token.starts_with("milestone/"))
                .is_some_and(|target| !own.is_some_and(|own| own.head.eq_ignore_ascii_case(target)))
    })
}

/// Where the clause's sync verbs sit, as word positions — whole words in
/// their imperative or present form; a `merge`/`pull` read as a noun
/// (`merge train`, `pull request`) is none, nor is a past tense (`merged`).
fn sync_verbs<'a>(tokens: &'a [&'a str]) -> impl Iterator<Item = usize> + 'a {
    let next = |at: usize| tokens.get(at.saturating_add(1)).copied().unwrap_or("");
    tokens.iter().enumerate().filter_map(move |(at, word)| {
        let verb = match *word {
            "merge" | "merges" | "merging" | "pull" | "pulls" | "pulling" => {
                !NOUNS_AFTER.contains(&next(at))
            }
            "rebase" | "rebases" | "rebasing" | "cherry-pick" | "cherry-picks"
            | "cherry-picking" | "catch-up" | "catch-ups" => true,
            "catch" | "catches" | "catching" => next(at) == "up",
            "up" => {
                tokens.get(at.saturating_add(1)..at.saturating_add(3)) == Some(&["to", "date"][..])
                    && brought_from(tokens, at).is_some()
            }
            "up-to-date" => brought_from(tokens, at).is_some(),
            _ => false,
        };
        verb.then(|| {
            if matches!(*word, "up" | "up-to-date") {
                brought_from(tokens, at).unwrap_or(at)
            } else {
                at
            }
        })
    })
}

/// The position of the `bring`/`get`/`make` an `up to date` at `at` belongs
/// to, a few words before it — a bare `is up to date` describes, it does
/// not prescribe.
fn brought_from(tokens: &[&str], at: usize) -> Option<usize> {
    (at.saturating_sub(4)..at).rev().find(|before| {
        tokens
            .get(*before)
            .is_some_and(|word| BRINGS.contains(word))
    })
}

/// Whether the verb at `at` is negated or done by the harness.
fn cleared(tokens: &[&str], at: usize) -> bool {
    negated(tokens, at)
        || tokens
            .get(at.saturating_sub(SUBJECT_REACH)..at)
            .is_some_and(|before| before.iter().any(|word| HARNESS.contains(word)))
}

/// Whether a negation sits just before the word at `at`.
fn negated(tokens: &[&str], at: usize) -> bool {
    tokens
        .get(at.saturating_sub(NEGATION_REACH)..at)
        .is_some_and(|before| before.iter().any(|word| NEGATIONS.contains(word)))
}

/// Whether the tokens name the branch `branch` whole — bare, as
/// `origin/<branch>` or possessive, never inside a longer name.
fn names(tokens: &[&str], branch: &str) -> bool {
    !branch.is_empty()
        && tokens
            .iter()
            .map(|token| token.strip_prefix("origin/").unwrap_or(token))
            .map(|token| token.strip_suffix("'s").unwrap_or(token))
            .any(|token| token == branch)
}

/// The tokens of a lower-cased clause, in order: runs of letters, digits,
/// `_ - / . '`, a sentence's final dot dropped — words and branch names
/// alike.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | '\'')))
        .map(|token| token.trim_end_matches('.'))
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const BASE: &str = "main_agent";

    #[test]
    fn the_two_live_sentences_are_caught() {
        // #486, 2026-10-24 18:04, and #219, 22:09.
        let found = prescribed(
            "Repair: merge `origin/main_agent` into milestone/24-table and resolve the \
             handler conflict, then rerun the gates.",
            BASE,
            Some(Own {
                head: "feat/oral-brief",
                base: "milestone/24-table",
            }),
        );
        assert!(found.is_some_and(|clause| clause.contains("origin/main_agent")));
        assert!(
            prescribed(
                "The session merges origin/main_agent into its branch itself, then builds.",
                BASE,
                None
            )
            .is_some()
        );
    }

    #[test]
    fn merging_the_own_base_or_pushing_the_own_head_is_allowed() {
        let task = Own {
            head: "feat/save",
            base: "milestone/22-fiche",
        };
        assert_eq!(
            prescribed(
                "Merge origin/milestone/22-fiche into the branch and resolve Cargo.lock.",
                BASE,
                Some(task)
            ),
            None
        );
        // A landing PR's base is the integration branch, its head the
        // milestone's: its repair merges one and pushes the other.
        let landing = Own {
            head: "milestone/22-fiche",
            base: "main_agent",
        };
        assert_eq!(
            prescribed(
                "Merge main_agent in, resolve the conflict, then push to milestone/22-fiche.",
                BASE,
                Some(landing)
            ),
            None
        );
        assert!(
            prescribed("Then push to milestone/24-table.", BASE, Some(task)).is_some(),
            "another milestone's branch is never a session's to push"
        );
    }

    #[test]
    fn a_negation_or_the_harness_s_own_work_is_clean() {
        for text in [
            "Do not merge main_agent; wait on #519.",
            "Never rebase onto main_agent.",
            "Don\u{2019}t pull main_agent into the branch.",
            "Wait: the harness's catch-up brings main_agent into milestone/28 first.",
            "The harness merges main_agent into the milestone branch itself.",
            "The harness then merges main_agent into milestone/28.",
        ] {
            assert_eq!(prescribed(text, BASE, None), None, "{text}");
        }
    }

    #[test]
    fn a_negation_or_the_harness_elsewhere_in_the_clause_clears_nothing() {
        for text in [
            "Repair: merge origin/main_agent into milestone/24-table, no other change",
            "the session merges origin/main_agent into its branch since the harness's \
             catch-up is stuck",
            "Merge origin/main_agent into the branch so no conflict remains",
            "merge main_agent into milestone/24 for the harness checks",
        ] {
            assert!(prescribed(text, BASE, None).is_some(), "{text}");
        }
    }

    #[test]
    fn a_description_is_no_gesture() {
        for text in [
            "OralBrief was merged into main_agent with #519; the task goes on once \
             milestone/28 holds it",
            "the rules pull request into main_agent lands first",
            "Wait until the merge train lands main_agent's new rules.",
            "Once milestone/28 is up to date with main_agent, repair the handler.",
            "Answer: the merge conflict with main_agent is the catch-up's, not this task's.",
        ] {
            assert_eq!(prescribed(text, BASE, None), None, "{text}");
        }
    }

    #[test]
    fn a_push_to_the_own_base_is_caught_a_merge_from_it_is_not() {
        let task = Own {
            head: "feat/x",
            base: "milestone/3-x",
        };
        assert!(prescribed("Push the fix to origin/milestone/3-x.", BASE, Some(task)).is_some());
        assert!(prescribed("then git push origin HEAD:milestone/3-x", BASE, Some(task)).is_some());
        assert_eq!(
            prescribed(
                "Merge origin/milestone/3-x into the branch, resolve the lock file, then push.",
                BASE,
                Some(task)
            ),
            None
        );
        assert_eq!(
            prescribed("Do not push to milestone/3-x.", BASE, Some(task)),
            None
        );
    }

    #[test]
    fn bringing_a_branch_up_to_date_in_one_word_is_caught() {
        assert!(prescribed("Bring milestone/28 up-to-date with main_agent.", BASE, None).is_some());
        assert_eq!(
            prescribed("milestone/28 is up-to-date with main_agent.", BASE, None),
            None,
            "a description, no bring"
        );
    }

    #[test]
    fn a_longer_branch_name_is_not_the_integration_branch() {
        assert_eq!(prescribed("merge main_agent_old into it", BASE, None), None);
        assert_eq!(prescribed("pull my-main_agent", BASE, None), None);
        assert!(prescribed("pull main_agent.", BASE, None).is_some());
    }

    proptest! {
        #[test]
        fn any_verb_with_the_integration_branch_is_caught(
            verb in proptest::sample::select(vec![
                "merge", "Merge", "pull", "rebase onto", "cherry-pick from", "catch up with",
                "bring the branch up to date with",
            ]),
            origin in any::<bool>(),
            before in "[a-z ]{0,16}",
            after in "[a-z ]{0,16}",
        ) {
            let named = if origin { "origin/main_agent" } else { "main_agent" };
            let text = format!("{before} {verb} {named} {after}");
            let lower = text.to_lowercase();
            prop_assume!(
                !tokens(&lower).any(|w| NEGATIONS.contains(&w) || HARNESS.contains(&w))
            );
            prop_assert!(prescribed(&text, BASE, None).is_some(), "{}", text);
        }

        #[test]
        fn text_without_the_integration_branch_or_a_milestone_push_is_clean(
            text in "[a-zA-Z ,.;:#0-9/_-]{0,120}",
        ) {
            prop_assume!(!text.to_lowercase().contains("main_agent"));
            prop_assume!(!text.to_lowercase().contains("milestone/"));
            prop_assert_eq!(prescribed(&text, BASE, None), None);
        }

        #[test]
        fn a_push_to_another_milestone_branch_is_caught_whatever_the_own_base(
            number in 1u32..999,
            name in "[a-z]{1,8}",
            origin in any::<bool>(),
        ) {
            let base = format!("milestone/{number}-{name}");
            let own = Own { head: "feat/task", base: &base };
            let target = if origin { format!("origin/{base}") } else { base.clone() };
            let push = format!("push the fix to {target}");
            prop_assert!(prescribed(&push, BASE, Some(own)).is_some(), "{}", push);
            prop_assert_eq!(
                prescribed(&format!("merge {target} into the branch, then push"), BASE, Some(own)),
                None
            );
            let landing = Own { head: &base, base: BASE };
            prop_assert_eq!(
                prescribed(&format!("merge main_agent in, then push to {target}"), BASE, Some(landing)),
                None
            );
        }
    }
}
