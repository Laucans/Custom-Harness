//! The five sections of a refined issue body: read them, render them.
//!
//! Pure business logic: no I/O, no subprocess, no network. Headings are
//! English because they're written as-is in the issue body.
//!
//! Hand-written rather than with `regex`: a `## title` line is recognized in
//! three string comparisons, and a dependency doesn't warrant that — same
//! reason as `dev_loop::tasks::closes`.

use std::collections::HashMap;

use harness_core::adapters::agent::Reply;

/// A section: its stage key, markdown title, and round.
#[derive(Debug, Clone, Copy)]
pub struct Section {
    /// Its stage key (`"business-goal"`, …).
    pub key: &'static str,
    /// Its markdown title (`"Business Goal"`, …).
    pub heading: &'static str,
    /// The round that introduces it.
    pub round: u32,
}

/// The canonical order: how the body is rewritten, and how stages run.
pub const SECTIONS: [Section; 5] = [
    Section {
        key: "business-goal",
        heading: "Business Goal",
        round: 1,
    },
    Section {
        key: "technical",
        heading: "Technical",
        round: 1,
    },
    Section {
        key: "acceptance-criteria",
        heading: "Acceptance Criteria",
        round: 1,
    },
    Section {
        key: "business-rules",
        heading: "Business Rules",
        round: 2,
    },
    Section {
        key: "technical-plan",
        heading: "Technical Implementation Plan",
        round: 2,
    },
];

/// The keys, in canonical order.
pub const KEYS: [&str; 5] = [
    "business-goal",
    "technical",
    "acceptance-criteria",
    "business-rules",
    "technical-plan",
];

/// The section for this key, or `None`.
#[must_use]
pub fn by_key(key: &str) -> Option<&'static Section> {
    SECTIONS.iter().find(|s| s.key == key)
}

fn by_heading(title: &str) -> Option<&'static Section> {
    let lower = title.to_lowercase();
    SECTIONS.iter().find(|s| s.heading.to_lowercase() == lower)
}

/// The keys of sections this round introduces.
#[must_use]
pub fn keys_of_round(round_no: u32) -> Vec<&'static str> {
    SECTIONS
        .iter()
        .filter(|s| s.round == round_no)
        .map(|s| s.key)
        .collect()
}

/// A `## title` line, and where its text starts and ends.
struct Mark {
    /// The start of the line — where the next section stops the previous.
    line_start: usize,
    /// The end of the title line — where the section text starts.
    text_end: usize,
    title: String,
}

/// The `## title` lines in the body, recognized or not.
///
/// The space after `##` is required: without it, a `###` in section text
/// would read as a boundary and cut the section in two. An empty title
/// (`##` alone or followed only by spaces) is not a boundary.
fn heading_marks(body: &str) -> Vec<Mark> {
    let mut marks = Vec::new();
    let mut offset = 0usize;
    for line in body.split('\n') {
        if let Some(after_hashes) = line.strip_prefix("##") {
            let after_space = after_hashes.trim_start_matches([' ', '\t']);
            // At least one space/tab consumed between `##` and title.
            if after_space.len() < after_hashes.len() {
                let title = after_space.trim_end_matches([' ', '\t']);
                if !title.is_empty() {
                    marks.push(Mark {
                        line_start: offset,
                        text_end: offset + line.len(),
                        title: title.to_string(),
                    });
                }
            }
        }
        offset += line.len() + 1;
    }
    marks
}

/// The canonical sections of the body, by key.
///
/// Text before the first `##` and unknown titles are ignored, but an unknown
/// title is still a boundary: it closes the previous section.
#[must_use]
pub fn parse(body: &str) -> HashMap<String, String> {
    let marks = heading_marks(body);
    let mut found = HashMap::new();
    for (i, mark) in marks.iter().enumerate() {
        let Some(section) = by_heading(&mark.title) else {
            continue;
        };
        let end = marks.get(i + 1).map_or(body.len(), |next| next.line_start);
        let text = body[mark.text_end..end].trim();
        if !text.is_empty() {
            found.insert(section.key.to_string(), text.to_string());
        }
    }
    found
}

/// The canonical body: non-empty sections, in order.
// Never called with a hasher other than the default throughout the crate:
// generalizing over `BuildHasher` would add only a type parameter nobody
// fills — same reason for the two following functions.
#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn render(found: &HashMap<String, String>) -> String {
    let blocks: Vec<String> = SECTIONS
        .iter()
        .filter_map(|s| {
            let text = found.get(s.key)?.trim();
            if text.is_empty() {
                return None;
            }
            Some(format!("## {}\n\n{text}\n", s.heading))
        })
        .collect();
    blocks.join("\n")
}

/// Round 1 keys missing from the body or empty.
#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn missing(found: &HashMap<String, String>) -> Vec<&'static str> {
    keys_of_round(1)
        .into_iter()
        .filter(|key| {
            found
                .get(*key)
                .map(|t| t.trim())
                .unwrap_or_default()
                .is_empty()
        })
        .collect()
}

/// The body this round would render: `found`, replaced for the target keys.
///
/// A stage that rendered nothing leaves the previous section in place instead
/// of erasing it — `results` carries what a resumption or dry-run didn't pay
/// for as well as what a session rendered empty.
#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn merge(
    found: &HashMap<String, String>,
    wanted: &[String],
    results: &HashMap<String, Reply>,
) -> HashMap<String, String> {
    let mut merged = found.clone();
    for key in wanted {
        if let Some(reply) = results.get(key) {
            let text = reply.text.trim();
            if !text.is_empty() {
                merged.insert(key.clone(), text.to_string());
            }
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_with_two_known_sections_parses_both() {
        let body = "## Business Goal\n\nthe goal\n\n## Technical\n\nthe design\n";
        let found = parse(body);
        assert_eq!(
            found.get("business-goal").map(String::as_str),
            Some("the goal")
        );
        assert_eq!(
            found.get("technical").map(String::as_str),
            Some("the design")
        );
    }

    #[test]
    fn a_level_three_heading_does_not_cut_a_section_in_two() {
        let body = "## Technical\n\nintro\n\n### Detail\n\nrest\n";
        let found = parse(body);
        assert_eq!(
            found.get("technical").map(String::as_str),
            Some("intro\n\n### Detail\n\nrest")
        );
    }

    #[test]
    fn an_unknown_heading_closes_the_previous_section_but_opens_nothing() {
        let body = "## Business Goal\n\nthe goal\n\n## Miscellaneous Notes\n\noff topic\n";
        let found = parse(body);
        assert_eq!(
            found.get("business-goal").map(String::as_str),
            Some("the goal")
        );
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn text_before_the_first_heading_is_ignored() {
        let found = parse("noise\n\n## Technical\n\nthe design\n");
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn render_keeps_the_canonical_order_regardless_of_insertion_order() {
        let mut found = HashMap::new();
        found.insert("technical".to_string(), "design".to_string());
        found.insert("business-goal".to_string(), "goal".to_string());
        let said = render(&found);
        assert!(said.find("Business Goal").unwrap() < said.find("Technical").unwrap());
    }

    #[test]
    fn missing_lists_only_round_one_keys_that_are_absent_or_blank() {
        let mut found = HashMap::new();
        found.insert("business-goal".to_string(), "goal".to_string());
        found.insert("technical".to_string(), "   ".to_string());
        let said = missing(&found);
        assert_eq!(said, vec!["technical", "acceptance-criteria"]);
    }

    #[test]
    fn merge_leaves_a_section_untouched_when_its_stage_rendered_nothing() {
        let mut found = HashMap::new();
        found.insert("technical".to_string(), "old".to_string());
        let results = HashMap::new(); // nothing rendered
        let merged = merge(&found, &["technical".to_string()], &results);
        assert_eq!(merged.get("technical").map(String::as_str), Some("old"));
    }

    #[test]
    fn merge_overwrites_only_the_wanted_keys() {
        let mut found = HashMap::new();
        found.insert("technical".to_string(), "old".to_string());
        found.insert("business-goal".to_string(), "unchanged".to_string());
        let mut results = HashMap::new();
        results.insert(
            "technical".to_string(),
            Reply {
                text: "new".to_string(),
                stop_line: None,
                spend: harness_core::domain::Spend::default(),
            },
        );
        let merged = merge(&found, &["technical".to_string()], &results);
        assert_eq!(merged.get("technical").map(String::as_str), Some("new"));
        assert_eq!(
            merged.get("business-goal").map(String::as_str),
            Some("unchanged")
        );
    }
}
