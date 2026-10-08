//! The round counter, and what a given round writes. Pure business logic.
//!
//! The current round is read from the issue's comments: the highest `N`
//! already commented, plus one. No state file, no label — the counter lives
//! where the human can see and correct it.

use crate::refinement::data::phase::Phase;
use crate::refinement::data::sections;

/// What a model can write to name a section: its key, or its title — plus
/// two short forms that, without them, would reopen the wrong section
/// ("technical plan" is a prefix of "Technical Implementation Plan", and
/// "technical" of "technical-plan").
const ALIASES: [(&str, &str); 2] = [
    ("technical plan", "technical-plan"),
    ("implementation plan", "technical-plan"),
];

/// The highest round of this phase already commented, `0` if none.
#[must_use]
pub fn counter(comments: &[String], phase: Phase) -> u32 {
    comments
        .iter()
        .flat_map(|body| body.lines())
        .filter_map(|line| round_in_line(line, phase))
        .max()
        .unwrap_or(0)
}

fn round_in_line(line: &str, phase: Phase) -> Option<u32> {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();
    let rest = lower.strip_prefix(phase.marker())?;
    rest.trim().parse().ok()
}

/// The round-end comment: `refinement round: 3`, or its technical form.
#[must_use]
pub fn comment(round_no: u32, phase: Phase) -> String {
    format!("{} {round_no}", phase.marker())
}

/// Does this round go through the router?
#[must_use]
pub const fn routed(round_no: u32, has_context: bool) -> bool {
    round_no >= 2 && has_context
}

/// The sections this round writes, outside router decisions.
///
/// All of the phase's sections, so one request is enough to get a complete
/// half; nothing when the router decides.
#[must_use]
pub fn planned(phase: Phase, round_no: u32, has_context: bool) -> Vec<&'static str> {
    if routed(round_no, has_context) {
        return Vec::new();
    }
    phase.keys().to_vec()
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

/// The section keys of this phase the router named, in canonical order.
///
/// Empty when nothing is recognized: the router's exit gate makes it a
/// failure rather than a silent round. A key of the other phase is ignored —
/// this round does not write it.
#[must_use]
pub fn wanted_from(answer: &str, phase: Phase) -> Vec<String> {
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
        .filter(|key| named.contains(key) && phase.keys().contains(key))
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counter_is_the_highest_round_commented_so_far() {
        let comments = vec![
            "something else".to_string(),
            "refinement round: 2".to_string(),
            "refinement round: 1".to_string(),
        ];
        assert_eq!(counter(&comments, Phase::Business), 2);
    }

    #[test]
    fn no_comment_at_all_counts_as_round_zero() {
        assert_eq!(counter(&[], Phase::Business), 0);
    }

    #[test]
    fn the_marker_is_matched_case_insensitively() {
        assert_eq!(
            counter(&["REFINEMENT ROUND: 4".to_string()], Phase::Business),
            4
        );
    }

    #[test]
    fn each_phase_counts_its_own_rounds_only() {
        let comments = vec![
            "refinement round: 1".to_string(),
            "technical refinement round: 3".to_string(),
        ];
        assert_eq!(counter(&comments, Phase::Business), 1);
        assert_eq!(counter(&comments, Phase::Technical), 3);
    }

    #[test]
    fn a_round_writes_all_the_sections_of_its_phase_and_only_those() {
        assert_eq!(
            planned(Phase::Business, 1, false),
            vec!["business-goal", "acceptance-criteria", "business-rules"]
        );
        assert_eq!(
            planned(Phase::Technical, 1, false),
            vec!["technical", "technical-plan"]
        );
    }

    #[test]
    fn a_second_round_without_context_rewrites_the_whole_phase() {
        assert_eq!(planned(Phase::Technical, 2, false), Phase::Technical.keys());
    }

    #[test]
    fn a_later_round_with_context_defers_entirely_to_the_router() {
        assert_eq!(planned(Phase::Business, 2, true), [] as [&str; 0]);
        assert!(routed(2, true));
        assert!(!routed(1, true));
    }

    #[test]
    fn the_router_names_sections_by_key_or_by_heading() {
        assert_eq!(
            wanted_from(
                "please rework business rules and acceptance-criteria",
                Phase::Business
            ),
            vec!["acceptance-criteria", "business-rules"]
        );
    }

    #[test]
    fn a_short_token_does_not_fire_inside_an_already_consumed_long_one() {
        // Without the alias, "technical plan" would match twice: once for
        // "technical", once for the long token.
        assert_eq!(
            wanted_from("rework the technical plan", Phase::Technical),
            vec!["technical-plan"]
        );
    }

    #[test]
    fn a_section_of_the_other_phase_is_ignored() {
        assert_eq!(
            wanted_from("technical", Phase::Business),
            [] as [std::string::String; 0]
        );
    }

    #[test]
    fn naming_nothing_recognisable_wants_nothing() {
        assert_eq!(
            wanted_from("do something, anything", Phase::Business),
            [] as [std::string::String; 0]
        );
    }

    #[test]
    fn the_answer_is_read_case_insensitively() {
        assert_eq!(
            wanted_from("BUSINESS GOAL", Phase::Business),
            vec!["business-goal"]
        );
    }
}
