//! The planner's own item type, parsed via the shared extraction.
//!
//! The session is asked to answer with a JSON array and nothing else, but a
//! model's reply is text, not structure — [`crate::common::json_reply`] is
//! the boundary where that text either becomes a plan or a reason it
//! couldn't; this module only names what one milestone looks like.

use serde::Deserialize;

pub use crate::common::json_reply::ParseError;
use crate::common::sections;
use harness_architecture::vocabulary::Side;

/// One milestone the plan wants opened, in delivery order.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MilestoneItem {
    /// A short, specific title — becomes the milestone issue's title.
    pub title: String,
    /// A few sentences on what this milestone achieves — becomes its body.
    pub goal: String,
    /// The systems (bounded contexts) it works in — one, where possible.
    #[serde(default)]
    pub systems: Vec<String>,
    /// The versioned Concepts it defines or implements (`Risk@3`).
    #[serde(default)]
    pub concepts: Vec<String>,
    /// Whether any of its work is on the write side — a `DataCapability`
    /// that mutates, an aggregate shaped. Such a milestone's tasks run one at
    /// a time, each merged once its agent review is clean.
    #[serde(default)]
    pub writes: bool,
    /// The aggregates it shapes or writes (`PJ`, `Campagne`): what chains
    /// it after the milestones that touch one of them, and lets two
    /// milestones on different aggregates open together.
    #[serde(default)]
    pub aggregates: Vec<String>,
    /// The rules (`R3`) whose tests turn green when it lands, as
    /// `rules.json` names them: what `split` hands its tasks.
    #[serde(default)]
    pub rules: Vec<String>,
}

impl MilestoneItem {
    /// Which side of the architecture the milestone's work is on.
    #[must_use]
    pub const fn side(&self) -> Side {
        if self.writes { Side::Write } else { Side::Read }
    }

    /// The body: the goal, then the `## Architecture` section.
    #[must_use]
    pub fn body(&self) -> String {
        let list = |items: &[String]| {
            let kept: Vec<&str> = items
                .iter()
                .map(|item| item.trim())
                .filter(|item| !item.is_empty())
                .collect();
            if kept.is_empty() {
                "-".to_string()
            } else {
                kept.join(", ")
            }
        };
        let body = format!(
            "{}\n\n## {}\n\nsystems: {}\nconcepts: {}\naggregates: {}\nside: {}",
            self.goal.trim(),
            sections::heading_of(sections::ARCHITECTURE),
            list(&self.systems),
            list(&self.concepts),
            list(&self.aggregates),
            self.side().label()
        );
        let rules = list(&self.rules);
        if rules == "-" {
            body
        } else {
            format!(
                "{body}\n\n## {}\n\n{rules}",
                sections::heading_of(sections::RULES)
            )
        }
    }

    /// The aggregates, trimmed and lower-cased: what two milestones compare.
    #[must_use]
    pub fn aggregate_keys(&self) -> Vec<String> {
        self.aggregates
            .iter()
            .map(|a| a.trim().to_lowercase())
            .filter(|a| !a.is_empty() && a != "-")
            .collect()
    }
}

/// The aggregates a milestone's body names (`aggregates: PJ, Campagne`
/// under `## Architecture`), lower-cased — what an already open milestone
/// is compared on.
#[must_use]
pub fn aggregates_of(body: &str) -> Vec<String> {
    sections::parse(body)
        .get(sections::ARCHITECTURE)
        .map(|text| {
            text.lines()
                .filter_map(|line| line.trim().strip_prefix("aggregates:"))
                .flat_map(|value| value.split(','))
                .map(|a| a.trim().trim_matches('`').to_lowercase())
                .filter(|a| !a.is_empty() && a != "-")
                .collect()
        })
        .unwrap_or_default()
}

/// Extracts the JSON array of milestones from a reply's text.
///
/// # Errors
/// A [`ParseError`] if no array-shaped substring can be found, or if what
/// was found does not deserialize into `Vec<MilestoneItem>`.
pub fn parse(text: &str) -> Result<Vec<MilestoneItem>, ParseError> {
    crate::common::json_reply::parse_array(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_json_array_parses() {
        let items = parse(r#"[{"title":"A","goal":"do A"}]"#).expect("parse");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "A");
        assert_eq!(items[0].goal, "do A");
        assert_eq!(items[0].systems, [] as [String; 0]);
        assert!(!items[0].writes, "read-side unless said otherwise");
    }

    #[test]
    fn the_body_carries_the_goal_and_the_architecture_section() {
        let item = MilestoneItem {
            title: "Risk".to_string(),
            goal: "Score every customer.".to_string(),
            systems: vec!["credit".to_string(), " ".to_string()],
            concepts: vec!["Risk@3".to_string()],
            writes: true,
            aggregates: vec!["Account".to_string(), " ".to_string()],
            rules: Vec::new(),
        };
        assert_eq!(
            item.body(),
            "Score every customer.\n\n## Architecture\n\nsystems: credit\nconcepts: Risk@3\n\
             aggregates: Account\nside: harness:write-side"
        );
        assert_eq!(item.aggregate_keys(), ["account"]);
        assert_eq!(aggregates_of(&item.body()), ["account"]);
        assert_eq!(aggregates_of("no section"), [] as [String; 0]);
        let bare = MilestoneItem {
            writes: false,
            aggregates: Vec::new(),
            systems: Vec::new(),
            concepts: Vec::new(),
            ..item
        };
        assert!(
            bare.body()
                .ends_with("systems: -\nconcepts: -\naggregates: -\nside: harness:read-side")
        );
        assert_eq!(bare.side(), Side::Read);
    }

    #[test]
    fn the_rules_a_milestone_turns_green_follow_its_architecture() {
        let items =
            parse(r#"[{"title": "Campaigns", "goal": "List them.", "rules": ["R3", " ", "R5"]}]"#)
                .expect("parse");
        let body = items[0].body();
        assert!(
            body.ends_with("side: harness:read-side\n\n## Rules\n\nR3, R5"),
            "{body}"
        );
        assert_eq!(
            sections::parse(&body)
                .get(sections::RULES)
                .map(String::as_str),
            Some("R3, R5")
        );
        let none = parse(r#"[{"title": "t", "goal": "g"}]"#).expect("parse");
        assert!(!none[0].body().contains("## Rules"), "no rule, no section");
    }

    #[test]
    fn an_empty_array_is_valid_and_yields_nothing_to_create() {
        assert_eq!(parse("[]").expect("parse"), Vec::new());
    }

    #[test]
    fn multiple_milestones_keep_their_delivery_order() {
        let items = parse(r#"[{"title":"A","goal":"a"},{"title":"B","goal":"b"}]"#).expect("parse");
        assert_eq!(items[0].title, "A");
        assert_eq!(items[1].title, "B");
    }
}
