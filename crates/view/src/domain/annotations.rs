//! The human's pins: a short note dropped on a part of a board — a table, a
//! field, a spot of the canvas — kept for the next visit.
//!
//! Personal notes for now: nothing the harness reads, nothing sent anywhere.
//! One file holds every board's pins, keyed by the board; a board replaces
//! its own list whole, after the checks below.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The most pins one board keeps.
pub const MAX_NOTES: usize = 500;

/// The longest note, in characters.
pub const MAX_TEXT: usize = 4_000;

/// The longest name of what a pin sits on.
const MAX_TARGET: usize = 200;

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
    /// What the human wrote.
    #[serde(default)]
    pub text: String,
    /// When last written, as the page's clock says.
    #[serde(default)]
    pub at: String,
}

/// Every board's pins.
pub type Shelf = BTreeMap<String, Vec<Note>>;

/// A board's name: lower case, digits, dashes, at most 40.
#[must_use]
pub fn is_board(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The shelf as kept; an unreadable file is an empty shelf.
#[must_use]
pub fn shelf(json: Option<&str>) -> Shelf {
    json.and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default()
}

/// Checks a board's new list of pins.
///
/// # Errors
///
/// The sentence the page shows: too many pins, an id twice, a bad id, a
/// target or a note too long, a position that is not a number.
pub fn checked(notes: Vec<Note>) -> Result<Vec<Note>, String> {
    if notes.len() > MAX_NOTES {
        return Err(format!("at most {MAX_NOTES} pins on a board"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for note in &notes {
        if !is_id(&note.id) {
            return Err(format!(
                "a pin's id is letters, digits and dashes: {:?}",
                note.id
            ));
        }
        if !seen.insert(note.id.as_str()) {
            return Err(format!("two pins share the id {}", note.id));
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
    }
    Ok(notes)
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
    fn a_list_is_checked_before_it_is_kept() {
        assert!(checked(vec![note("a"), note("b")]).is_ok());
        assert!(
            checked(vec![note("a"), note("a")])
                .unwrap_err()
                .contains("share")
        );
        assert!(checked(vec![note("a b")]).is_err());
        let mut long = note("a");
        long.text = "x".repeat(MAX_TEXT + 1);
        assert!(checked(vec![long]).is_err());
        let mut lost = note("a");
        lost.dx = f64::NAN;
        assert!(checked(vec![lost]).is_err());
        assert!(checked((0..=MAX_NOTES).map(|k| note(&k.to_string())).collect()).is_err());
    }

    #[test]
    fn an_unreadable_shelf_is_empty() {
        assert!(shelf(None).is_empty());
        assert!(shelf(Some("not json")).is_empty());
        let kept = serde_json::to_string(&Shelf::from([("data-model".into(), vec![note("a")])]))
            .expect("a shelf serializes");
        assert_eq!(
            shelf(Some(&kept))["data-model"][0].text,
            "ask about soft delete"
        );
    }

    proptest! {
        #[test]
        fn a_kept_note_comes_back_as_written(text in "\\PC{0,200}", dx in (-1_000_000_i32..1_000_000).prop_map(f64::from)) {
            let mut n = note("p-1");
            n.text = text;
            n.dx = dx;
            let kept = checked(vec![n.clone()]).expect("a plain note is kept");
            let json = serde_json::to_string(&Shelf::from([("b".into(), kept)])).expect("serializes");
            prop_assert_eq!(&shelf(Some(&json))["b"][0], &n);
        }
    }
}
