//! The planner's own item type, parsed via the shared extraction.
//!
//! The session is asked to answer with a JSON array and nothing else, but a
//! model's reply is text, not structure — [`crate::common::json_reply`] is
//! the boundary where that text either becomes a plan or a reason it
//! couldn't; this module only names what one milestone looks like.

use serde::Deserialize;

pub use crate::common::json_reply::ParseError;

/// One milestone the plan wants opened, in delivery order.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MilestoneItem {
    /// A short, specific title — becomes the milestone issue's title.
    pub title: String,
    /// A few sentences on what this milestone achieves — becomes its body.
    pub goal: String,
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
