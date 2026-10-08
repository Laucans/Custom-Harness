//! The planner's own item type, parsed via the shared extraction.
//!
//! The session is asked to answer with a JSON array and nothing else, but a
//! model's reply is text, not structure — [`crate::common::json_reply`] is
//! the boundary where that text either becomes a plan or a reason it
//! couldn't; this module only names what one milestone looks like.

use serde::Deserialize;

use crate::common::architecture::Side;
pub use crate::common::json_reply::ParseError;
use crate::common::sections;

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
    /// that mutates, an invariant, a migration. Such a milestone comes
    /// before the readers that depend on it, and its tasks wait for a human
    /// merge.
    #[serde(default)]
    pub writes: bool,
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
        format!(
            "{}\n\n## {}\n\nsystems: {}\nconcepts: {}\nside: {}",
            self.goal.trim(),
            sections::heading_of(sections::ARCHITECTURE),
            list(&self.systems),
            list(&self.concepts),
            self.side().label()
        )
    }
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
        assert_eq!(items[0].systems, [] as [std::string::String; 0]);
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
        };
        assert_eq!(
            item.body(),
            "Score every customer.\n\n## Architecture\n\nsystems: credit\nconcepts: Risk@3\n\
             side: harness:write-side"
        );
        let bare = MilestoneItem {
            writes: false,
            systems: Vec::new(),
            concepts: Vec::new(),
            ..item
        };
        assert!(
            bare.body()
                .ends_with("systems: -\nconcepts: -\nside: harness:read-side")
        );
        assert_eq!(bare.side(), Side::Read);
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
