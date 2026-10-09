//! The human's pins: a note dropped on a part of a board — a table, a field,
//! a spot of the canvas — kept for the next visit.
//!
//! A pin is one of three kinds. A **note** is the human's words. A
//! **brainstorm** has an AI session behind it, opened from the pin and
//! remembered between visits (`note_session`). An **asking** is a change the
//! human wants considered when the next roadmap is built — its conversation
//! kept too; how it reaches the planner is still to be designed
//! (`docs/PARKED.md`).
//!
//! A deleted pin is only marked: the last [`UNDO_DEPTH`] of a board come back
//! with ctrl+z, the older ones are destroyed with their conversation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The longest note, in characters.
pub const MAX_TEXT: usize = 4_000;

/// How many deleted pins a board can still bring back.
pub const UNDO_DEPTH: usize = 10;

/// The longest name of what a pin sits on.
const MAX_TARGET: usize = 200;

/// What a pin is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The human's words. A yellow pin.
    #[default]
    Note,
    /// A conversation with an AI session about what the pin sits on. Green.
    Brainstorm,
    /// A change to consider when the next roadmap is built. Orange.
    Asking,
}

/// One pin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Chosen by the page; unique on its board.
    pub id: String,
    /// What it sits on, as the board names it — `field:character.name` —
    /// or empty for a spot of the canvas.
    #[serde(default)]
    pub target: String,
    /// Where on the target, from its top left corner, in board units.
    #[serde(default)]
    pub dx: f64,
    /// Where on the target, from its top left corner, in board units.
    #[serde(default)]
    pub dy: f64,
    /// Where on the board it was last seen, for a target gone since.
    #[serde(default)]
    pub x: f64,
    /// Where on the board it was last seen, for a target gone since.
    #[serde(default)]
    pub y: f64,
    /// What the human wrote — or the conclusion, or the request, an AI drew.
    #[serde(default)]
    pub text: String,
    /// When last written, as the page's clock says.
    #[serde(default)]
    pub at: String,
    /// A note, a brainstorm, an asking.
    #[serde(default)]
    pub kind: Kind,
    /// How many turns its conversation holds — filled by the server, never
    /// read from the page.
    #[serde(default)]
    pub turns: usize,
}

/// Every board's pins, as the old `.llocal/annotations.json` kept them.
pub type Shelf = BTreeMap<String, Vec<Note>>;

/// A board's name: lower case, digits, dashes, at most 40.
#[must_use]
pub fn is_board(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A pin's id: letters, digits, dashes, underscores, at most 40.
#[must_use]
pub fn is_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The old file's shelf; an unreadable file is an empty shelf.
#[must_use]
pub fn shelf(json: Option<&str>) -> Shelf {
    json.and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default()
}

/// Checks one pin before it is kept.
///
/// # Errors
///
/// The sentence the page shows: a bad id, a target or a note too long, a
/// position that is not a number.
pub fn checked(note: Note) -> Result<Note, String> {
    if !is_id(&note.id) {
        return Err(format!(
            "a pin's id is letters, digits and dashes: {:?}",
            note.id
        ));
    }
    if note.target.chars().count() > MAX_TARGET {
        return Err(format!(
            "pin {}: what it sits on is named too long",
            note.id
        ));
    }
    if note.text.chars().count() > MAX_TEXT {
        return Err(format!(
            "pin {}: a note is at most {MAX_TEXT} characters",
            note.id
        ));
    }
    if note.at.len() > 40 {
        return Err(format!("pin {}: its date is too long", note.id));
    }
    if ![note.dx, note.dy, note.x, note.y]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(format!("pin {}: its position is not a number", note.id));
    }
    Ok(note)
}

/// The deleted pins past the undo depth — destroyed for good, conversation
/// included. `deleted` is every deleted pin of a board with when it went.
#[must_use]
pub fn beyond_undo(deleted: &[(String, String)]) -> Vec<String> {
    let mut newest_first: Vec<&(String, String)> = deleted.iter().collect();
    newest_first.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    newest_first
        .into_iter()
        .skip(UNDO_DEPTH)
        .map(|(id, _)| id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn note(id: &str) -> Note {
        Note {
            id: id.to_string(),
            target: "table:campaign".to_string(),
            dx: 4.0,
            dy: 2.0,
            x: 10.0,
            y: 20.0,
            text: "ask about soft delete".to_string(),
            at: "2026-10-09T12:00:00Z".to_string(),
            kind: Kind::Note,
            turns: 0,
        }
    }

    #[test]
    fn a_board_is_named_plainly() {
        assert!(is_board("data-model"));
        assert!(!is_board(""));
        assert!(!is_board("../etc"));
        assert!(!is_board("Data"));
        assert!(!is_board(&"a".repeat(41)));
    }

    #[test]
    fn a_pin_is_checked_before_it_is_kept() {
        assert!(checked(note("a")).is_ok());
        assert!(checked(note("a b")).is_err());
        let mut long = note("a");
        long.text = "x".repeat(MAX_TEXT + 1);
        assert!(checked(long).is_err());
        let mut lost = note("a");
        lost.dx = f64::NAN;
        assert!(checked(lost).is_err());
    }

    #[test]
    fn an_old_pin_reads_as_a_note() {
        let old = r#"{"id":"n1","target":"","dx":0,"dy":0,"x":1,"y":2,"text":"t","at":""}"#;
        let read: Note = serde_json::from_str(old).expect("an old pin reads");
        assert_eq!(read.kind, Kind::Note);
        let kept = shelf(Some(&format!("{{\"data-model\":[{old}]}}")));
        assert_eq!(kept["data-model"][0].text, "t");
        assert!(shelf(Some("not json")).is_empty());
    }

    #[test]
    fn only_the_ten_newest_deleted_pins_can_come_back() {
        let deleted: Vec<(String, String)> = (0..13)
            .map(|k| (format!("n{k:02}"), format!("2026-10-09T12:{k:02}:00Z")))
            .collect();
        assert_eq!(beyond_undo(&deleted), ["n02", "n01", "n00"]);
        assert!(beyond_undo(&deleted[..10]).is_empty());
    }

    proptest! {
        #[test]
        fn a_kept_pin_comes_back_as_written(
            text in "\\PC{0,200}",
            dx in (-1_000_000_i32..1_000_000).prop_map(f64::from),
        ) {
            let mut n = note("p-1");
            n.text = text;
            n.dx = dx;
            n.kind = Kind::Brainstorm;
            let kept = checked(n.clone()).expect("a plain note is kept");
            let json = serde_json::to_string(&kept).expect("serializes");
            let back: Note = serde_json::from_str(&json).expect("reads");
            prop_assert_eq!(back, n);
        }

        #[test]
        fn never_more_than_the_undo_depth_survives(count in 0_usize..40) {
            let deleted: Vec<(String, String)> = (0..count)
                .map(|k| (format!("n{k}"), format!("2026-10-09T{:02}:00:00Z", k % 24)))
                .collect();
            prop_assert_eq!(beyond_undo(&deleted).len(), count.saturating_sub(UNDO_DEPTH));
        }
    }
}
