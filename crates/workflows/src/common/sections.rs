//! The sections of an issue body: read them, render them.
//!
//! Shared rather than refinement-private, because three workflows meet on this
//! model: `split` writes the first section, the refinement rewrites the five
//! that follow, and `common::hierarchy` reads a sibling's body back out of it.
//!
//! Pure business logic: no I/O, no subprocess, no network. Headings are
//! English because they're written as-is in the issue body.
//!
//! Hand-written rather than with `regex`: a `## title` line is recognized in
//! three string comparisons, and a dependency doesn't warrant that — same
//! reason as `dev_loop::tasks::closes`.
//!
//! # The two sections nobody rewrites
//!
//! [`SCOPE`] and [`ARCHITECTURE`] are written once, by `split`, and belong to
//! no refinement phase — so every round carries them through untouched. The
//! second says what the task *is* in the agent-native architecture (see
//! [`harness_architecture::vocabulary`]). The first exists because the boundary
//! a slice states ("covers this, does not cover that") was the most useful
//! sentence in a task's body and the refinement used to overwrite it: the body
//! was replaced by the five sections it knows, and the only record of what the
//! task was *not* meant to do disappeared with it.

use std::collections::HashMap;
use std::fmt::Write as _;

use harness_core::ports::agent::Reply;

/// A section: its stage key and markdown title.
#[derive(Debug, Clone, Copy)]
pub struct Section {
    /// Its stage key (`"business-goal"`, …).
    pub key: &'static str,
    /// Its markdown title (`"Business Goal"`, …).
    pub heading: &'static str,
}

/// The key of the section `split` writes and no refinement phase touches.
pub const SCOPE: &str = "scope";

/// The key of the other section `split` writes and no phase touches.
///
/// What the task is in the agent-native architecture — its unit, its
/// system, the Concept it implements, the effect of a `DataCapability`.
/// Read back by [`harness_architecture::vocabulary`]; the loop derives the
/// task's side from it, so a refinement that rewrote it could move a task
/// across the read/write frontier behind the labels' back.
pub const ARCHITECTURE: &str = "architecture";

/// The key of the section a graph question is raised in and answered under.
///
/// What the task's `## Architecture` could not hold
/// ([`crate::common::decision`]). Written by neither phase's sections: the
/// refinement's graph pass adds it, the arbiter or a human settles it, and
/// a round that finds the contract holds again drops it.
pub const DECISION: &str = "decision";

/// The canonical order: how the body is rewritten, and how stages run.
pub const SECTIONS: [Section; 8] = [
    Section {
        key: SCOPE,
        heading: "Scope",
    },
    Section {
        key: ARCHITECTURE,
        heading: "Architecture",
    },
    Section {
        key: "business-goal",
        heading: "Business Goal",
    },
    Section {
        key: "acceptance-criteria",
        heading: "Acceptance Criteria",
    },
    Section {
        key: "business-rules",
        heading: "Business Rules",
    },
    Section {
        key: "technical",
        heading: "Technical",
    },
    Section {
        key: "technical-plan",
        heading: "Technical Implementation Plan",
    },
    Section {
        key: DECISION,
        heading: "Decision",
    },
];

/// The keys, in canonical order.
pub const KEYS: [&str; 8] = [
    SCOPE,
    ARCHITECTURE,
    "business-goal",
    "acceptance-criteria",
    "business-rules",
    "technical",
    "technical-plan",
    DECISION,
];

/// The section for this key, or `None`.
#[must_use]
pub fn by_key(key: &str) -> Option<&'static Section> {
    SECTIONS.iter().find(|s| s.key == key)
}

/// The heading a key is written under, so a writer never spells it by hand.
///
/// An unknown key gives back the key itself: the alternative is a panic in a
/// writer, and a wrong heading is visible in the issue while a panic kills a
/// paid run.
#[must_use]
pub fn heading_of(key: &str) -> &str {
    by_key(key).map_or(key, |section| section.heading)
}

fn by_heading(title: &str) -> Option<&'static Section> {
    let lower = title.to_lowercase();
    SECTIONS.iter().find(|s| s.heading.to_lowercase() == lower)
}

/// A heading as a drop-list names it: lowercase, trimmed, up to any
/// parenthetical.
///
/// `## Assumptions (autonomous run)` is named by `"Assumptions"`. The
/// parenthetical is a session's own note about why the section exists, and a
/// list that had to spell it would miss the first run that worded it
/// differently.
fn comparable(title: &str) -> String {
    let head = title.split('(').next().unwrap_or(title);
    head.trim().to_lowercase()
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

/// The sections under headings this module does not know, verbatim.
///
/// What [`render`] leaves out, so a round that rewrites the body keeps what
/// it never read: a session's `## Assumptions (autonomous run)`, a human's
/// `## Owner Decisions`. Text before the first `##` is not a section and is
/// not kept. Empty when every heading is a canonical one.
#[must_use]
pub fn unknown(body: &str) -> String {
    let marks = heading_marks(body);
    let mut kept: Vec<&str> = Vec::new();
    for (i, mark) in marks.iter().enumerate() {
        if by_heading(&mark.title).is_some() {
            continue;
        }
        let end = marks.get(i + 1).map_or(body.len(), |next| next.line_start);
        kept.push(body[mark.line_start..end].trim());
    }
    kept.join("\n\n")
}

/// The body without the sections these headings name, everything else
/// verbatim.
///
/// Verbatim rather than `render(parse(…))`: the preamble before the first
/// `##`, an unknown heading and the body's own spacing all survive. The point
/// is to remove what a stage does not act on, not to reformat what it does —
/// and the body is a GitHub issue a human reads too.
///
/// A heading is named in lowercase, up to any parenthetical, and compared
/// **exactly**, not by prefix: dropping `Technical` must not take
/// `Technical Implementation Plan` with it. `/create-test` is the stage that needs precisely that distinction —
/// it tests against the plan and has no use for the design.
#[must_use]
pub fn without(body: &str, headings: &[&str]) -> String {
    if headings.is_empty() {
        return body.to_string();
    }
    let unwanted: Vec<String> = headings.iter().map(|h| comparable(h)).collect();
    let marks = heading_marks(body);
    let mut kept = String::with_capacity(body.len());
    let mut at = 0usize;
    for (i, mark) in marks.iter().enumerate() {
        if !unwanted.contains(&comparable(&mark.title)) {
            continue;
        }
        let end = marks.get(i + 1).map_or(body.len(), |next| next.line_start);
        // Marks are ordered and `end` is the next one's start, so `at` never
        // runs past the mark being dropped.
        kept.push_str(&body[at..mark.line_start]);
        at = end;
    }
    kept.push_str(&body[at..]);
    kept.trim().to_string()
}

/// The body reduced to the sections whose heading names one of these markers.
///
/// Substring and case-insensitive, because the heading is written by whoever
/// wrote the document — a roadmap is prose, not a form, and its headings are in
/// the language it was written in.
///
/// **It fails open**: a body where nothing matched comes back whole. The
/// alternative is a document written under other headings losing everything, and
/// an empty roadmap is worse than a long one — see
/// [`hierarchy::ROADMAP_KEPT`](crate::common::hierarchy).
///
/// Text before the first `##` is dropped with the rest: what names the document
/// is its title, which the block that carries it prints itself.
#[must_use]
pub fn keeping(body: &str, markers: &[&str]) -> String {
    let lowered: Vec<String> = markers.iter().map(|m| m.to_lowercase()).collect();
    let marks = heading_marks(body);
    let mut kept: Vec<&str> = Vec::new();
    for (i, mark) in marks.iter().enumerate() {
        let title = mark.title.to_lowercase();
        if !lowered.iter().any(|marker| title.contains(marker.as_str())) {
            continue;
        }
        let end = marks.get(i + 1).map_or(body.len(), |next| next.line_start);
        kept.push(body[mark.line_start..end].trim());
    }
    if kept.is_empty() {
        return body.trim().to_string();
    }
    kept.join("\n\n")
}

/// The body kept under `budget` characters, whole sections at a time.
///
/// Sections are taken in the order written — the text before the first
/// heading counts as one — until the next would not fit; what is left out is
/// named in one closing line, with `fetch` as the gesture that reads it
/// (`gh issue view 65`). A section is never cut in the middle: a plan whose
/// last steps vanished mid-sentence reads as a plan with fewer steps.
///
/// Counted in characters, never bytes: the bodies are written in French, and
/// a budget that fell inside an accented letter has already panicked a run.
#[must_use]
pub fn within(body: &str, budget: usize, fetch: &str) -> String {
    let marks = heading_marks(body);
    let mut pieces: Vec<(String, &str)> = Vec::new();
    let first = marks.first().map_or(body.len(), |mark| mark.line_start);
    if !body[..first].trim().is_empty() {
        pieces.push(("(the opening text)".to_string(), &body[..first]));
    }
    for (i, mark) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map_or(body.len(), |next| next.line_start);
        pieces.push((mark.title.clone(), &body[mark.line_start..end]));
    }
    let mut kept = String::new();
    let mut spent = 0usize;
    let mut dropped: Vec<String> = Vec::new();
    for (title, text) in pieces {
        let size = text.chars().count();
        if !dropped.is_empty() || spent + size > budget {
            dropped.push(title);
            continue;
        }
        spent += size;
        kept.push_str(text);
    }
    let mut out = kept.trim().to_string();
    if !dropped.is_empty() {
        let _ = write!(
            out,
            "\n\n(left out for room: {} — `{fetch}` reads them whole)",
            dropped.join(", ")
        );
    }
    out
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

/// The body with these sections replaced, or added in canonical order.
///
/// Everything else is kept: the other canonical sections as they stand,
/// the unknown ones verbatim after them. An empty text drops the section.
///
/// What a workflow that touches one or two sections of a task — the
/// arbiter on `## Architecture` and `## Decision` — writes with, so it
/// never erases what it did not read.
#[must_use]
pub fn replace(body: &str, changes: &[(&str, &str)]) -> String {
    let mut found = parse(body);
    for (key, text) in changes {
        if text.trim().is_empty() {
            found.remove(*key);
        } else {
            found.insert((*key).to_string(), text.trim().to_string());
        }
    }
    let rendered = render(&found);
    let extras = unknown(body);
    if extras.is_empty() {
        rendered
    } else {
        format!("{rendered}\n{extras}\n")
    }
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
    fn an_unknown_heading_is_kept_verbatim_and_a_known_one_is_not() {
        let body = "intro\n\n## Business Goal\n\nthe goal\n\n\
                    ## Assumptions (autonomous run)\n\n- one\n- two\n\n\
                    ## Technical\n\nthe design\n\n## Owner Decisions\n\nyes\n";
        assert_eq!(
            unknown(body),
            "## Assumptions (autonomous run)\n\n- one\n- two\n\n## Owner Decisions\n\nyes"
        );
        assert_eq!(unknown("## Business Goal\n\nthe goal\n"), "");
        assert_eq!(unknown("no heading at all"), "");
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
    fn merge_leaves_a_section_untouched_when_its_stage_rendered_nothing() {
        let mut found = HashMap::new();
        found.insert("technical".to_string(), "old".to_string());
        let results = HashMap::new(); // nothing rendered
        let merged = merge(&found, &["technical".to_string()], &results);
        assert_eq!(merged.get("technical").map(String::as_str), Some("old"));
    }

    #[test]
    fn the_scope_section_survives_a_round_that_rewrites_everything_else() {
        // What this exists for: the slice's own boundary used to be erased by
        // the first refinement, and nothing recorded what the task was not
        // meant to do.
        let body = "## Scope\n\nbranch: feat/x\n\nCovers: A. Does not cover: B.\n\n\
                    ## Business Goal\n\nold goal\n";
        let found = parse(body);
        assert!(found[SCOPE].contains("Does not cover: B."));
        let mut results = HashMap::new();
        results.insert(
            "business-goal".to_string(),
            Reply {
                text: "new goal".to_string(),
                stop_line: None,
                spend: harness_core::domain::Spend::default(),
            },
        );
        // Every business key rewritten, scope named by nobody.
        let wanted = vec![
            "business-goal".to_string(),
            "acceptance-criteria".to_string(),
            "business-rules".to_string(),
        ];
        let published = render(&merge(&found, &wanted, &results));
        assert!(published.contains("Does not cover: B."), "{published}");
        assert!(published.contains("new goal"));
        // And it stays at the top, where a reader finds it first.
        assert!(published.starts_with("## Scope"), "{published}");
    }

    // --- what a stage leaves out ------------------------------------------

    const REFINED: &str = "## Business Goal\n\nthe goal\n\n\
                           ## Acceptance Criteria\n\n- one\n\n\
                           ## Technical\n\nthe design\n\n\
                           ## Technical Implementation Plan\n\n1. do it\n\n\
                           ## Assumptions (autonomous run)\n\nguessed\n";

    #[test]
    fn dropping_technical_does_not_take_the_plan_with_it() {
        // The distinction `/create-test` lives on: it tests against the plan
        // and has no use for the design. A prefix comparison would take both.
        let left = without(REFINED, &["Technical"]);
        assert!(!left.contains("the design"), "{left}");
        assert!(left.contains("## Technical Implementation Plan"), "{left}");
        assert!(left.contains("1. do it"));
    }

    #[test]
    fn a_heading_is_named_without_its_parenthetical() {
        // `## Assumptions (autonomous run)` is named by `Assumptions`: a list
        // that had to spell the parenthetical would miss the first run that
        // worded it differently.
        let left = without(REFINED, &["Assumptions"]);
        assert!(!left.contains("guessed"), "{left}");
        assert!(!left.contains("Assumptions"), "{left}");
    }

    #[test]
    fn everything_not_named_survives_byte_for_byte() {
        // Not `render(parse(…))`: the body is a GitHub issue a human reads, and
        // reformatting it is not what a stage filter was asked to do.
        let left = without(REFINED, &["Business Goal"]);
        assert!(left.contains("## Acceptance Criteria\n\n- one"), "{left}");
        assert!(left.starts_with("## Acceptance Criteria"), "{left}");
    }

    #[test]
    fn two_sections_dropped_in_a_row_leave_no_hole() {
        let left = without(REFINED, &["Technical Implementation Plan", "Assumptions"]);
        assert!(left.ends_with("the design"), "{left}");
        assert!(left.contains("the goal"));
    }

    #[test]
    fn an_empty_drop_list_changes_nothing() {
        assert_eq!(without(REFINED, &[]), REFINED);
    }

    #[test]
    fn a_heading_nobody_wrote_drops_nothing() {
        assert_eq!(without(REFINED, &["Risks"]).trim(), REFINED.trim());
    }

    #[test]
    fn a_preamble_before_the_first_heading_is_kept() {
        let body = format!("a line nobody put under a heading\n\n{REFINED}");
        let left = without(&body, &["Technical"]);
        assert!(left.starts_with("a line nobody put"), "{left}");
    }

    // --- what a roadmap keeps ---------------------------------------------

    const ROADMAP: &str = "## La vision\n\nun long récit\n\n\
                           ## Le parcours\n\n1. jalon\n\n\
                           ## Frontières\n\nce qui est dehors\n\n\
                           ## Contraintes non négociables\n\npas de SaaS\n\n\
                           ## Risques ouverts\n\npeut-être\n";

    #[test]
    fn a_roadmap_keeps_its_boundaries_and_loses_its_narrative() {
        let left = keeping(ROADMAP, &["frontière", "contrainte"]);
        assert!(left.contains("ce qui est dehors"), "{left}");
        assert!(left.contains("pas de SaaS"), "{left}");
        assert!(!left.contains("un long récit"), "{left}");
        assert!(!left.contains("peut-être"), "{left}");
    }

    #[test]
    fn a_marker_matches_whatever_case_the_heading_was_written_in() {
        let left = keeping("## BOUNDARIES\n\nhere\n", &["Boundar"]);
        assert!(left.contains("here"), "{left}");
    }

    #[test]
    fn a_document_matching_no_marker_comes_back_whole() {
        // Failing open: a roadmap under other headings must lose its
        // boundaries, not everything.
        let left = keeping(ROADMAP, &["nothing here"]);
        assert!(left.contains("un long récit"), "{left}");
        assert!(left.contains("peut-être"));
    }

    #[test]
    fn within_keeps_whole_sections_in_order_until_the_budget() {
        // A and B are 11 characters each, C is 9: 25 holds two of them.
        let body = "## A\n\n1234\n\n## B\n\n5678\n\n## C\n\n90\n";
        let kept = within(body, 25, "gh issue view 7");
        assert!(kept.starts_with("## A\n\n1234"), "{kept}");
        assert!(kept.contains("## B"), "{kept}");
        assert!(!kept.contains("## C"), "the third did not fit: {kept}");
        assert!(
            kept.ends_with("(left out for room: C — `gh issue view 7` reads them whole)"),
            "{kept}"
        );
    }

    #[test]
    fn within_never_skips_a_section_to_fit_a_later_smaller_one() {
        // Order is meaning: a plan's step 3 before its step 2 is not a plan.
        let body = "## A\n\n1234\n\n## B\n\n56789012345678901234\n\n## C\n\n9\n";
        let kept = within(body, 20, "x");
        assert!(kept.contains("## A"), "{kept}");
        assert!(!kept.contains("## B"), "{kept}");
        assert!(
            !kept.contains("## C"),
            "C fit, but B before it did not: {kept}"
        );
        assert!(kept.contains("left out for room: B, C"), "{kept}");
    }

    #[test]
    fn within_leaves_a_body_that_fits_alone() {
        let body = "intro\n\n## A\n\nshort\n";
        assert_eq!(within(body, 1_000, "x"), body.trim());
    }

    #[test]
    fn within_counts_characters_not_bytes() {
        // A is 18 characters but 28 bytes; with B's 8, the two fit in 26
        // characters and would not in 26 bytes.
        let body = format!("## A\n\n{}\n\n## B\n\nx\n", "é".repeat(10));
        let kept = within(&body, 26, "x");
        assert!(kept.contains("## A"), "{kept}");
        assert!(kept.contains("## B"), "{kept}");
    }

    #[test]
    fn within_names_the_opening_text_when_it_is_the_part_left_out() {
        let body = format!("{}\n\n## A\n\nx\n", "o".repeat(30));
        let kept = within(&body, 10, "x");
        assert!(
            kept.contains("left out for room: (the opening text), A"),
            "{kept}"
        );
    }

    #[test]
    fn keeping_carries_the_heading_with_its_text() {
        // Without the heading the kept text reads as the document's only
        // paragraph rather than as one named section of it.
        let left = keeping(ROADMAP, &["frontière"]);
        assert!(left.starts_with("## Frontières"), "{left}");
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

    #[test]
    fn replacing_a_section_keeps_the_others_and_the_unknown_ones() {
        let body = "## Scope\n\nbranch: x\n\n## Architecture\n\nunit: capability\n\n\
                    ## Owner Decisions\n\nkeep me\n";
        let out = replace(body, &[(ARCHITECTURE, "unit: capability\nneeds: PJ")]);
        assert_eq!(
            out,
            "## Scope\n\nbranch: x\n\n## Architecture\n\nunit: capability\nneeds: PJ\n\n\
             ## Owner Decisions\n\nkeep me\n"
        );
    }

    #[test]
    fn replacing_adds_a_missing_section_in_canonical_order_and_an_empty_text_drops_one() {
        let body = "## Scope\n\nbranch: x\n\n## Decision\n\nblocks: y\n";
        let added = replace(body, &[(ARCHITECTURE, "unit: concept")]);
        assert_eq!(
            added,
            "## Scope\n\nbranch: x\n\n## Architecture\n\nunit: concept\n\n## Decision\n\nblocks: y\n"
        );
        let dropped = replace(body, &[(DECISION, "  ")]);
        assert_eq!(dropped, "## Scope\n\nbranch: x\n");
    }
}
