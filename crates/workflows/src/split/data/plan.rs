//! Split's own item type, parsed via the shared extraction.
//!
//! The session is asked to answer with a JSON array and nothing else, but a
//! model's reply is text, not structure — [`crate::common::json_reply`] is
//! the boundary where that text either becomes a plan or a reason it
//! couldn't; this module only names what one task slice looks like.

use serde::Deserialize;

pub use crate::common::json_reply::ParseError;

/// One task slice the plan wants opened, in the order tasks must be taken.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TaskItem {
    /// A short, specific title — becomes the task issue's title.
    pub title: String,
    /// What this slice covers and does not cover — becomes its body,
    /// alongside the `branch:` line.
    pub brief: String,
    /// `<type>/<slug>` — written into the body as a parsable line.
    pub branch: String,
    /// True only for account creation, an interactive login, a payment
    /// decision, or anything else only a human in a browser can do.
    #[serde(default)]
    pub needs_human: bool,
}

/// Extracts the JSON array of task slices from a reply's text.
///
/// # Errors
/// A [`ParseError`] if no array-shaped substring can be found, or if what
/// was found does not deserialize into `Vec<TaskItem>`.
pub fn parse(text: &str) -> Result<Vec<TaskItem>, ParseError> {
    crate::common::json_reply::parse_array(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_json_array_parses() {
        let items =
            parse(r#"[{"title":"A","brief":"do A","branch":"feat/a","needs_human":false}]"#)
                .expect("parse");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "A");
        assert_eq!(items[0].branch, "feat/a");
        assert!(!items[0].needs_human);
    }

    #[test]
    fn needs_human_defaults_to_false_when_absent() {
        let items = parse(r#"[{"title":"A","brief":"do A","branch":"feat/a"}]"#).expect("parse");
        assert!(!items[0].needs_human);
    }

    #[test]
    fn an_empty_array_is_valid_and_yields_nothing_to_create() {
        assert_eq!(parse("[]").expect("parse"), Vec::new());
    }

    #[test]
    fn multiple_slices_keep_their_order() {
        let items = parse(
            r#"[{"title":"A","brief":"a","branch":"feat/a"},{"title":"B","brief":"b","branch":"feat/b"}]"#,
        )
        .expect("parse");
        assert_eq!(items[0].title, "A");
        assert_eq!(items[1].title, "B");
    }
}
