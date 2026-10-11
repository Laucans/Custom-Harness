//! The session's verdict, parsed via the shared extraction.
//!
//! The session is asked for a JSON object and nothing else; its reply is
//! text, and [`crate::common::json_reply`] is where that text becomes a
//! verdict or a reason it could not.

use serde::Deserialize;

pub use crate::common::json_reply::ParseError;

/// From which confidence up a verdict is applied; under it, the human
/// reads the verdict as advice.
pub const CONFIDENT: u8 = 4;

/// What the arbiter decided.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Verdict {
    /// `resolve` — every open id below is produced by the task named — or
    /// `human`.
    pub decision: String,
    /// How sure it is, `1..=5`.
    #[serde(default)]
    pub confidence: u8,
    /// Each open id, with the task of the milestone that produces it.
    #[serde(default)]
    pub needs: Vec<Chain>,
    /// One or two sentences: why.
    #[serde(default)]
    pub reason: String,
}

/// One id and the task that produces it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Chain {
    /// The id, as the manifests name it.
    pub id: String,
    /// The task's number.
    pub producer: u64,
}

impl Verdict {
    /// Whether the verdict is one the harness applies: a resolution it is
    /// sure of. Anything else is handed to a human with the reason.
    #[must_use]
    pub fn settles(&self) -> bool {
        self.decision.trim().eq_ignore_ascii_case("resolve") && self.confidence >= CONFIDENT
    }

    /// The chain named for this id, if any — ids compare case-insensitively.
    #[must_use]
    pub fn chain_for(&self, id: &str) -> Option<&Chain> {
        self.needs
            .iter()
            .find(|chain| chain.id.trim().eq_ignore_ascii_case(id.trim()))
    }
}

/// Extracts the verdict from a reply's text.
///
/// # Errors
/// A [`ParseError`] if no object-shaped substring can be found, or if
/// what was found does not deserialize into a [`Verdict`].
pub fn parse(text: &str) -> Result<Verdict, ParseError> {
    crate::common::json_reply::parse_object(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_confident_resolution_settles_and_names_its_chains() {
        let found = parse(
            r#"{"decision":"resolve","confidence":5,"needs":[{"id":"Personnage","producer":12}],"reason":"task 12 shapes it"}"#,
        )
        .expect("parse");
        assert!(found.settles());
        assert_eq!(found.chain_for("personnage").map(|c| c.producer), Some(12));
        assert_eq!(found.chain_for("Campagne"), None);
    }

    #[test]
    fn a_hesitant_or_human_verdict_does_not_settle() {
        let hesitant = parse(r#"{"decision":"resolve","confidence":3}"#).expect("parse");
        assert!(!hesitant.settles());
        let human =
            parse(r#"{"decision":"human","confidence":5,"reason":"a Concept"}"#).expect("parse");
        assert!(!human.settles());
        assert_eq!(human.reason, "a Concept");
    }

    #[test]
    fn prose_with_no_object_is_a_parse_error() {
        assert!(parse("I would chain it on #12").is_err());
        assert!(
            parse(r#"{"confidence": 5}"#).is_err(),
            "a decision is required"
        );
    }
}
