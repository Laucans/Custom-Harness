//! The round counter, and what a given round writes. Pure business logic.
//!
//! The current round is read from the issue's comments: the highest `N`
//! already commented, plus one. No state file, no label — the counter lives
//! where the human can see and correct it.

use std::collections::HashMap;

use crate::refinement::data::sections;

/// The round-end comment marker, case-insensitive.
pub const MARKER: &str = "refinement round:";

/// What a model can write to name a section: its key, or its title — plus
/// two short forms that, without them, would reopen the wrong section
/// ("technical plan" is a prefix of "Technical Implementation Plan", and
/// "technical" of "technical-plan").
const ALIASES: [(&str, &str); 2] = [
    ("technical plan", "technical-plan"),
    ("implementation plan", "technical-plan"),
];

/// The highest round already commented, `0` if none.
#[must_use]
pub fn counter(comments: &[String]) -> u32 {
    comments
        .iter()
        .flat_map(|body| body.lines())
        .filter_map(round_in_line)
        .max()
        .unwrap_or(0)
}

fn round_in_line(line: &str) -> Option<u32> {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();
    let rest = lower.strip_prefix(MARKER)?;
    rest.trim().parse().ok()
}

/// The round-end comment: `refinement round: 3`.
#[must_use]
pub fn comment(round_no: u32) -> String {
    format!("{MARKER} {round_no}")
}

/// Does this round go through the router?
#[must_use]
pub const fn routed(round_no: u32, has_context: bool) -> bool {
    round_no >= 3 && has_context
}

/// The sections this round writes, outside router decisions.
///
/// Round 1: the three of round 1. Round 2: the missing from round 1, then
/// round 2's two. Round >= 3: all five, or nothing when the router decides.
// Never called with a hasher other than the default throughout the crate:
// generalizing over `BuildHasher` would add only a type parameter nobody
// fills.
#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn planned(
    round_no: u32,
    found: &HashMap<String, String>,
    has_context: bool,
) -> Vec<&'static str> {
    if round_no <= 1 {
        return sections::keys_of_round(1);
    }
    if round_no == 2 {
        let mut said = sections::missing(found);
        said.extend(sections::keys_of_round(2));
        return said;
    }
    if routed(round_no, has_context) {
        return Vec::new();
    }
    sections::KEYS.to_vec()
}

/// A token, longest first: "technical plan" must be tried before "technical",
/// else the short token would consume it first and reopen the wrong section.
fn tokens() -> Vec<(String, &'static str)> {
    let mut said: Vec<(String, &'static str)> = sections::SECTIONS
        .iter()
        .flat_map(|s| {
            [
                (s.key.to_string(), s.key),
                (s.heading.to_lowercase(), s.key),
            ]
        })
        .collect();
    said.extend(ALIASES.iter().map(|(token, key)| (token.to_string(), *key)));
    said.sort_by_key(|(token, _)| std::cmp::Reverse(token.len()));
    said
}

const fn is_word_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

/// Does this text name the token completely — not stuck to a letter before,
/// or after.
fn names(haystack: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(token) {
        let at = start + pos;
        let before_ok = haystack[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word_char(c));
        let after = at + token.len();
        let after_ok = haystack[after..]
            .chars()
            .next()
            .is_none_or(|c| !is_word_char(c));
        if before_ok && after_ok {
            return true;
        }
        start = at + 1;
    }
    false
}

/// The section keys the router named, in canonical order.
///
/// Empty when nothing is recognized: the router's exit gate makes it a
/// failure rather than a silent round.
#[must_use]
pub fn wanted_from(answer: &str) -> Vec<String> {
    let mut said = answer.to_lowercase();
    let mut named: Vec<&'static str> = Vec::new();
    for (token, key) in tokens() {
        if names(&said, &token) {
            if !named.contains(&key) {
                named.push(key);
            }
            // Consumed, so a short token doesn't end up inside an already-recognized
            // long token.
            said = said.replace(&token, " ");
        }
    }
    sections::KEYS
        .iter()
        .filter(|key| named.contains(key))
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn the_counter_is_the_highest_round_commented_so_far() {
        let comments = vec![
            "something else".to_string(),
            "refinement round: 2".to_string(),
            "refinement round: 1".to_string(),
        ];
        assert_eq!(counter(&comments), 2);
    }

    #[test]
    fn no_comment_at_all_counts_as_round_zero() {
        assert_eq!(counter(&[]), 0);
    }

    #[test]
    fn the_marker_is_matched_case_insensitively() {
        assert_eq!(counter(&["REFINEMENT ROUND: 4".to_string()]), 4);
    }

    #[test]
    fn round_one_writes_exactly_the_three_round_one_sections() {
        assert_eq!(
            planned(1, &HashMap::new(), false),
            vec!["business-goal", "technical", "acceptance-criteria"]
        );
    }

    #[test]
    fn round_two_writes_whats_missing_from_round_one_then_round_twos_own() {
        let mut found = HashMap::new();
        found.insert("business-goal".to_string(), "already there".to_string());
        assert_eq!(
            planned(2, &found, false),
            vec![
                "technical",
                "acceptance-criteria",
                "business-rules",
                "technical-plan"
            ]
        );
    }

    #[test]
    fn round_three_without_context_rewrites_all_five() {
        assert_eq!(planned(3, &HashMap::new(), false), sections::KEYS.to_vec());
    }

    #[test]
    fn round_three_with_context_defers_entirely_to_the_router() {
        assert!(planned(3, &HashMap::new(), true).is_empty());
        assert!(routed(3, true));
    }

    #[test]
    fn the_router_names_sections_by_key_or_by_heading() {
        assert_eq!(
            wanted_from("please rework technical and acceptance-criteria"),
            vec!["technical", "acceptance-criteria"]
        );
    }

    #[test]
    fn a_short_token_does_not_fire_inside_an_already_consumed_long_one() {
        // Without the alias, "technical plan" would match twice: once for
        // "technical", once for the long token.
        assert_eq!(
            wanted_from("rework the technical plan"),
            vec!["technical-plan"]
        );
    }

    #[test]
    fn naming_nothing_recognisable_wants_nothing() {
        assert!(wanted_from("do something, anything").is_empty());
    }

    #[test]
    fn the_answer_is_read_case_insensitively() {
        assert_eq!(wanted_from("BUSINESS GOAL"), vec!["business-goal"]);
    }
}
