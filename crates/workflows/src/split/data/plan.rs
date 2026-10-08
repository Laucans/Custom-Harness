//! Split's own item type, parsed via the shared extraction.
//!
//! The session is asked to answer with a JSON array and nothing else, but a
//! model's reply is text, not structure — [`crate::common::json_reply`] is
//! the boundary where that text either becomes a plan or a reason it
//! couldn't; this module only names what one task slice looks like.

use serde::Deserialize;

use crate::common::architecture::{Declaration, Effect, Unit};
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
    /// What the slice builds in the agent-native architecture, as
    /// [`Unit::key`] spells it. Required: a slice that is not one unit of
    /// the architecture is not a slice the plant can place.
    pub unit: String,
    /// The bounded context it belongs to (`credit`, `orders`).
    #[serde(default)]
    pub system: String,
    /// The versioned Concept it implements or defines (`Risk@3`), if any.
    #[serde(default)]
    pub concept: Option<String>,
    /// For a `data-capability`: `insert`, `update`, `delete` or `upsert`.
    #[serde(default)]
    pub effect: Option<String>,
    /// For a `data-capability`: the existing fields it touches.
    #[serde(default)]
    pub touches: Vec<String>,
    /// The 0-based indexes, in the same array, of the slices this one builds
    /// on. Empty: none — a read-side slice then runs in parallel with its
    /// peers.
    #[serde(default)]
    pub depends_on: Vec<usize>,
}

impl TaskItem {
    /// The slice's place in the architecture.
    ///
    /// A unit the architecture does not name is read as `infrastructure`:
    /// the write side, where a human looks before anything merges — the
    /// safe reading of a slice the session could not place, and one that
    /// shows in the issue's own section rather than being silently fixed.
    #[must_use]
    pub fn declaration(&self) -> Declaration {
        Declaration {
            unit: Unit::parse(&self.unit).unwrap_or(Unit::Infrastructure),
            system: self.system.trim().to_string(),
            concept: self
                .concept
                .as_deref()
                .map(str::trim)
                .filter(|concept| !concept.is_empty() && *concept != "-")
                .map(ToString::to_string),
            effect: self.effect.as_deref().and_then(Effect::parse),
            touches: self
                .touches
                .iter()
                .map(|field| field.trim().to_string())
                .filter(|field| !field.is_empty() && field != "-")
                .collect(),
        }
    }
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

    use crate::common::architecture::Side;

    #[test]
    fn a_bare_json_array_parses() {
        let items = parse(
            r#"[{"title":"A","brief":"do A","branch":"feat/a","needs_human":false,"unit":"capability","system":"credit"}]"#,
        )
        .expect("parse");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "A");
        assert_eq!(items[0].branch, "feat/a");
        assert!(!items[0].needs_human);
        assert_eq!(items[0].declaration().unit, Unit::Capability);
        assert_eq!(items[0].declaration().system, "credit");
    }

    #[test]
    fn needs_human_and_the_architecture_details_default_when_absent() {
        let items = parse(r#"[{"title":"A","brief":"do A","branch":"feat/a","unit":"micro-ui"}]"#)
            .expect("parse");
        assert!(!items[0].needs_human);
        assert!(items[0].depends_on.is_empty());
        let declaration = items[0].declaration();
        assert_eq!(declaration.unit, Unit::MicroUi);
        assert_eq!(declaration.concept, None);
        assert_eq!(declaration.side(), Side::Read);
    }

    #[test]
    fn a_slice_without_a_unit_does_not_parse() {
        assert!(parse(r#"[{"title":"A","brief":"do A","branch":"feat/a"}]"#).is_err());
    }

    #[test]
    fn a_unit_the_architecture_does_not_name_is_read_as_infrastructure() {
        let items = parse(r#"[{"title":"A","brief":"do A","branch":"feat/a","unit":"widget"}]"#)
            .expect("parse");
        let declaration = items[0].declaration();
        assert_eq!(declaration.unit, Unit::Infrastructure);
        assert_eq!(
            declaration.side(),
            Side::Write,
            "a human looks at what nobody could place"
        );
    }

    #[test]
    fn a_data_capability_carries_its_effect_and_touches() {
        let items = parse(
            r#"[{"title":"A","brief":"a","branch":"feat/a","unit":"data-capability","system":"credit",
                "effect":"update","touches":["Account.creditLimit"," "],"depends_on":[0]}]"#,
        )
        .expect("parse");
        let declaration = items[0].declaration();
        assert_eq!(declaration.effect, Some(Effect::Update));
        assert_eq!(declaration.touches, ["Account.creditLimit"]);
        assert_eq!(declaration.side(), Side::Write);
        assert_eq!(items[0].depends_on, [0]);
    }

    #[test]
    fn an_empty_array_is_valid_and_yields_nothing_to_create() {
        assert_eq!(parse("[]").expect("parse"), Vec::new());
    }

    #[test]
    fn multiple_slices_keep_their_order() {
        let items = parse(
            r#"[{"title":"A","brief":"a","branch":"feat/a","unit":"concept"},{"title":"B","brief":"b","branch":"feat/b","unit":"capability"}]"#,
        )
        .expect("parse");
        assert_eq!(items[0].title, "A");
        assert_eq!(items[1].title, "B");
    }
}
