//! Extracting a JSON array out of a model's reply text.
//!
//! Shared because two workflows now ask a session for a plan and parse it
//! back the same way: `planner` (milestones) and `split` (tasks). Each keeps
//! its own item type and its own prompt; only the extraction — tolerant of
//! prose around the array, and of a fenced ` ```json ` block — is common.

use serde::de::DeserializeOwned;

/// Why a reply's JSON could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Extracts a JSON array of `T` from a reply's text.
///
/// Tolerant of prose around it and of a fenced ` ```json ` block — a model
/// rarely answers with *only* the array even when asked to.
///
/// # Errors
/// A [`ParseError`] if no array-shaped substring can be found, or if what
/// was found does not deserialize into `Vec<T>`.
pub fn parse_array<T: DeserializeOwned>(text: &str) -> Result<Vec<T>, ParseError> {
    let candidate =
        extract(text).ok_or_else(|| ParseError("no JSON array found in the reply".to_string()))?;
    serde_json::from_str(&candidate).map_err(|e| ParseError(format!("the JSON did not parse: {e}")))
}

/// The substring most likely to be the JSON array: a fenced ` ```json ` block
/// if one exists, else the span from the first `[` to the last `]`.
fn extract(text: &str) -> Option<String> {
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if end < start {
        return None;
    }
    Some(text[start..=end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    struct Item {
        name: String,
    }

    #[test]
    fn a_bare_json_array_parses() {
        let items: Vec<Item> = parse_array(r#"[{"name":"A"}]"#).expect("parse");
        assert_eq!(
            items,
            vec![Item {
                name: "A".to_string()
            }]
        );
    }

    #[test]
    fn a_fenced_array_surrounded_by_prose_parses() {
        let text = "Here is the plan:\n```json\n[{\"name\":\"A\"}]\n```\nDone.";
        let items: Vec<Item> = parse_array(text).expect("parse");
        assert_eq!(items[0].name, "A");
    }

    #[test]
    fn an_empty_array_is_valid_and_yields_nothing() {
        assert_eq!(parse_array::<Item>("[]").expect("parse"), Vec::new());
    }

    #[test]
    fn prose_with_no_array_at_all_is_a_parse_error() {
        assert!(parse_array::<Item>("no array here").is_err());
    }

    #[test]
    fn malformed_json_inside_the_brackets_is_a_parse_error_not_a_panic() {
        assert!(parse_array::<Item>("[{\"name\": }]").is_err());
    }

    #[test]
    fn a_closing_bracket_before_the_opening_one_is_not_mistaken_for_an_array() {
        assert!(parse_array::<Item>("] stray text [").is_err());
    }

    #[test]
    fn multiple_items_keep_their_order() {
        let items: Vec<Item> = parse_array(r#"[{"name":"A"},{"name":"B"}]"#).expect("parse");
        assert_eq!(items[0].name, "A");
        assert_eq!(items[1].name, "B");
    }
}
