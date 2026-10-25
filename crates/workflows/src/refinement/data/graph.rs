//! What the technical refinement's graph pass concludes: the design holds
//! the task's `## Architecture`, or it raises a question the arbiter reads.
//!
//! Pure business logic: parsing a short reply. The question itself is the
//! shared [`Question`], written as the `## Decision` section of the body;
//! the only line this module adds is the first, `graph: holds|open`.

use crate::common::decision::Question;

/// The line the verdict is written on.
const GRAPH_LINE: &str = "graph:";

/// What the graph reply says, as far as it can be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// The design stays inside the contract: nothing to raise.
    Holds,
    /// The design could not honour the contract, and this says where.
    Open(Box<Question>),
    /// The reply had no readable shape. Never read as `Open`: an invented
    /// question would pay an arbitration for nothing — nor as `Holds`
    /// silently: the publish says it in the journal.
    Unreadable,
}

/// Reads the graph pass's reply.
#[must_use]
pub fn read(text: &str) -> Read {
    let verdict = text
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(GRAPH_LINE))
        .map(|value| value.trim().to_lowercase());
    match verdict.as_deref() {
        Some("holds") => Read::Holds,
        Some("open") => Question::parse(text).map_or(Read::Unreadable, |q| Read::Open(Box::new(q))),
        _ => Read::Unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_is_holds_whatever_follows() {
        assert_eq!(read("graph: holds\n- it fits"), Read::Holds);
        assert_eq!(
            read("  Graph: HOLDS"),
            Read::Unreadable,
            "the key is lower-case"
        );
    }

    #[test]
    fn open_carries_the_question_with_its_lines() {
        let found =
            read("graph: open\nblocks: lists characters nobody produces\nneeds: Personnage\n");
        let Read::Open(question) = found else {
            panic!("open, got {found:?}");
        };
        assert_eq!(question.blocks, "lists characters nobody produces");
        assert_eq!(question.needs, ["Personnage"]);
        assert_eq!(question.round, 1);
    }

    #[test]
    fn open_without_a_reason_and_prose_without_a_verdict_are_unreadable() {
        assert_eq!(read("graph: open\nneeds: PJ"), Read::Unreadable);
        assert_eq!(read("it all looks fine to me"), Read::Unreadable);
        assert_eq!(read(""), Read::Unreadable);
    }
}
