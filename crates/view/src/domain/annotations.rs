//! The human's pins: a note dropped on a part of a board — a table, a field,
//! a spot of the canvas — kept for the next visit.
//!
//! A pin is one of five kinds. A **note** is the human's words. A
//! **brainstorm** has an AI session behind it, opened from the pin and
//! remembered between visits (`note_session`). A **request** is an
//! intention for the next roadmap — a change, a behaviour — discussed in a
//! session, and on the mock-up applied to the page's draft by an AI in one
//! go (`drafts`); it is `pending` until applied, `simulated` after, `stale`
//! when the page moved under it. On the mock-up, a **link** leads to
//! another page of the product — or creates one, from an objective — and a
//! **design** is a micro-frontend to make or to inject.
//!
//! A pin may be a **zone** instead of a point: a rectangle, a circle or a
//! hand-drawn outline around several parts, annotated as one.
//!
//! A deleted pin is only marked: the last [`UNDO_DEPTH`] of a board come back
//! with ctrl+z, the older ones are destroyed with their conversation.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// The longest note, in characters.
pub const MAX_TEXT: usize = 4_000;

/// How many deleted pins a board can still bring back.
pub const UNDO_DEPTH: usize = 10;

/// The longest name of what a pin sits on.
const MAX_TARGET: usize = 200;

/// The longest page a link leads to.
const MAX_PAGE: usize = 200;

/// The most points a zone's outline may have.
pub const MAX_POINTS: usize = 4_000;

/// The longest reference link a pin carries.
const MAX_SOURCE: usize = 500;

/// What a pin is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The human's words. A yellow pin.
    #[default]
    Note,
    /// A conversation with an AI session about what the pin sits on. Green.
    Brainstorm,
    /// An intention for the next roadmap: a change, a behaviour. Orange.
    /// Discussed in a session; on the mock-up, applied to the draft by an
    /// AI in one go. The old `asking` and `simulate` pins read as this.
    #[serde(alias = "asking", alias = "simulate")]
    Request,
    /// On the mock-up: leads to another page, or creates one. Blue.
    Link,
    /// On the mock-up: a micro-frontend to design — created by an AI
    /// session from its spec, or injected in the page it sits on. Teal.
    Design,
}

impl Kind {
    /// The kind's name, as the page and the sessions say it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Brainstorm => "brainstorm",
            Self::Request => "request",
            Self::Link => "link",
            Self::Design => "design",
        }
    }

    /// A kind whose session needs the note written first: its first prompt
    /// (a brainstorm). A request may be left empty — a pin turned into one
    /// before its words are found; nothing applies an empty one.
    #[must_use]
    pub const fn speaks_first(self) -> bool {
        matches!(self, Self::Brainstorm)
    }

    /// A kind whose session edits the mock-up rather than talks about it.
    #[must_use]
    pub const fn edits(self) -> bool {
        matches!(self, Self::Link | Self::Design)
    }

    /// A kind whose text is the human's own — a page's objective, a
    /// micro-frontend's spec — and never rewritten by a compact.
    #[must_use]
    pub const fn keeps_its_text(self) -> bool {
        matches!(self, Self::Link | Self::Design)
    }
}

/// Where a request stands against the page's draft.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    /// Not applied to the draft yet.
    #[default]
    Pending,
    /// Applied: the draft carries the operations it produced.
    Simulated,
    /// Applied once, but the page moved under it since.
    Stale,
    /// Applied, then validated: the mock-up keeps what it changed.
    Validated,
}

/// The outline of a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    /// Two corners.
    Rect,
    /// A centre and a point on the edge.
    Circle,
    /// A hand-drawn outline, closed.
    Path,
}

/// A zone: several parts annotated as one, by an outline in board units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    /// What outline.
    pub kind: ShapeKind,
    /// Its points, `[x, y]`, in board units.
    pub points: Vec<[f64; 2]>,
}

/// One pin.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// A note, a brainstorm, an asking, a link, a simulate.
    #[serde(default)]
    pub kind: Kind,
    /// For a link: the page it leads to, as `mock/pages.json` lists it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub page: String,
    /// For a design: the micro-frontend's name, `RiskBadge`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub component: String,
    /// The part of its target the pin is about, when the human picked one —
    /// an element of a kit component, `table.kit-table:nth-child(2)>thead:nth-child(1)`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub part: String,
    /// A reference the session draws from: a Figma frame's link, a page
    /// elsewhere. A URL, or empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// For a request on the mock-up: where it stands against the draft.
    #[serde(default, skip_serializing_if = "is_pending")]
    pub state: RequestState,
    /// The backlog issue opened from this pin, if one was — its URL.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub issue: String,
    /// A zone's outline — `None` for a point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Shape>,
    /// How many turns its conversation holds — filled by the server, never
    /// read from the page.
    #[serde(default)]
    pub turns: usize,
}

/// Every board's pins, as the old `.llocal/annotations.json` kept them.
pub type Shelf = BTreeMap<String, Vec<Note>>;

/// A board's name: lower case, digits, dashes, at most 120 — a mock-up's
/// board names its branch and its page.
#[must_use]
pub fn is_board(name: &str) -> bool {
    (1..=120).contains(&name.len())
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

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's `skip_serializing_if` hands a reference
fn is_pending(state: &RequestState) -> bool {
    *state == RequestState::Pending
}

/// A micro-frontend's name: a Micro-UI's, `RiskBadge` — a capital, then
/// letters and digits, at most 80.
#[must_use]
pub fn is_component(name: &str) -> bool {
    (1..=80).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
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
    if note.page.chars().count() > MAX_PAGE || note.page.contains("..") {
        return Err(format!("pin {}: its page is not a page", note.id));
    }
    if !note.component.is_empty() && !is_component(&note.component) {
        return Err(format!(
            "pin {}: a micro-frontend is named like a Micro-UI, `RiskBadge`",
            note.id
        ));
    }
    if note.kind == Kind::Design && note.component.is_empty() {
        return Err(format!("pin {}: name the micro-frontend first", note.id));
    }
    if !note.source.is_empty()
        && (note.source.len() > MAX_SOURCE
            || !(note.source.starts_with("https://") || note.source.starts_with("http://")))
    {
        return Err(format!(
            "pin {}: a reference is a link (https://…)",
            note.id
        ));
    }
    if let Some(shape) = &note.shape {
        let enough = match shape.kind {
            ShapeKind::Rect | ShapeKind::Circle => shape.points.len() == 2,
            ShapeKind::Path => shape.points.len() >= 3,
        };
        if !enough || shape.points.len() > MAX_POINTS {
            return Err(format!("pin {}: its zone has no outline", note.id));
        }
        if !shape.points.iter().flatten().all(|v| v.is_finite()) {
            return Err(format!("pin {}: its zone is not a shape", note.id));
        }
    }
    // A brainstorm, an asking or a simulate opens its session on what the
    // note says: there must be something to say first.
    if note.kind.speaks_first() && note.text.trim().is_empty() {
        return Err(format!(
            "pin {}: a brainstorm or an asking starts from the note's text — write it first",
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

/// The longest context a page may add to a backlog issue, in characters.
pub const MAX_ISSUE_CONTEXT: usize = 20_000;

/// The issue a pin becomes on the product's backlog: its title is the
/// pin's first line, its body the pin's text, where it sits (the board,
/// the part), and what the page adds — the draft's summary, the page's
/// objective. Answers `(title, body)`, or why there is nothing to open.
///
/// # Errors
///
/// The pin has no text, or is a plain note.
pub fn backlog_issue(board: &str, note: &Note, context: &str) -> Result<(String, String), String> {
    if note.kind == Kind::Note {
        return Err("a note is not a backlog item: make it a request first".into());
    }
    let text = note.text.trim();
    if text.is_empty() {
        return Err("write the pin first: its first line becomes the issue's title".into());
    }
    let first = text.lines().next().unwrap_or_default().trim();
    let title: String = first.chars().take(120).collect();
    let mut body = String::new();
    body.push_str(text);
    body.push_str("\n\n---\n\n");
    let _ = write!(
        body,
        "Opened from a **{}** pin on `{board}`",
        note.kind.name()
    );
    if !note.target.is_empty() {
        let _ = write!(body, ", on `{}`", note.target);
    }
    if !note.page.is_empty() {
        let _ = write!(body, ", leading to the page `{}`", note.page);
    }
    if !note.component.is_empty() {
        let _ = write!(body, ", about the micro-frontend `{}`", note.component);
    }
    body.push_str(".\n");
    if !note.source.is_empty() {
        let _ = writeln!(body, "Reference: {}", note.source);
    }
    let context = context.trim();
    if !context.is_empty() {
        body.push('\n');
        body.push_str(&context.chars().take(MAX_ISSUE_CONTEXT).collect::<String>());
        body.push('\n');
    }
    Ok((title, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_request_validated_is_said_so_and_read_back() {
        let said = serde_json::to_string(&RequestState::Validated).expect("json");
        assert_eq!(said, "\"validated\"");
        assert_eq!(
            serde_json::from_str::<RequestState>(&said).expect("read"),
            RequestState::Validated
        );
    }

    #[test]
    fn a_request_becomes_an_issue_titled_by_its_first_line() {
        let note = Note {
            id: "n1".into(),
            kind: Kind::Request,
            text: "Put the total above the lines\nso the eye lands on it".into(),
            target: "id:total".into(),
            ..Note::default()
        };
        let (title, body) =
            backlog_issue("mockup--main-agent--campagne", &note, "Draft v3: 4 ops").unwrap();
        assert_eq!(title, "Put the total above the lines");
        assert!(body.starts_with("Put the total above the lines\nso the eye lands on it"));
        assert!(body.contains("**request** pin on `mockup--main-agent--campagne`, on `id:total`"));
        assert!(body.ends_with("Draft v3: 4 ops\n"));
    }

    #[test]
    fn a_note_or_an_empty_pin_opens_nothing() {
        let empty = Note {
            kind: Kind::Request,
            ..Note::default()
        };
        assert!(backlog_issue("b", &empty, "").is_err());
        let note = Note {
            kind: Kind::Note,
            text: "x".into(),
            ..Note::default()
        };
        assert!(backlog_issue("b", &note, "").is_err());
    }

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
            page: String::new(),
            component: String::new(),
            part: String::new(),
            source: String::new(),
            state: RequestState::Pending,
            issue: String::new(),
            shape: None,
            turns: 0,
        }
    }

    #[test]
    fn a_board_is_named_plainly() {
        assert!(is_board("data-model"));
        assert!(is_board("mockup--milestone-3-foo--credit-review"));
        assert!(!is_board(""));
        assert!(!is_board("../etc"));
        assert!(!is_board("Data"));
        assert!(!is_board(&"a".repeat(121)));
    }

    #[test]
    fn a_zone_needs_an_outline_and_a_link_needs_no_words() {
        let mut zone = note("z");
        zone.shape = Some(Shape {
            kind: ShapeKind::Rect,
            points: vec![[0.0, 0.0], [10.0, 10.0]],
        });
        assert!(checked(zone.clone()).is_ok());
        zone.shape = Some(Shape {
            kind: ShapeKind::Path,
            points: vec![[0.0, 0.0], [10.0, 10.0]],
        });
        assert!(
            checked(zone.clone()).is_err(),
            "a path is three points at least"
        );
        zone.shape = Some(Shape {
            kind: ShapeKind::Circle,
            points: vec![[0.0, 0.0], [f64::INFINITY, 1.0]],
        });
        assert!(checked(zone).is_err());
        let mut link = note("l");
        link.kind = Kind::Link;
        link.text = String::new();
        link.page = "pages/home.html".to_string();
        assert!(
            checked(link.clone()).is_ok(),
            "a link is its page, not its words"
        );
        link.page = "../x".to_string();
        assert!(checked(link).is_err());
        let mut request = note("s");
        request.kind = Kind::Request;
        request.text = " ".to_string();
        assert!(
            checked(request).is_ok(),
            "a request may be made before its words are found"
        );
        let old: Note = serde_json::from_str(r#"{"id":"n1","kind":"simulate","text":"t"}"#)
            .expect("an old simulate reads");
        assert_eq!(old.kind, Kind::Request);
        let old: Note = serde_json::from_str(r#"{"id":"n1","kind":"asking","text":"t"}"#)
            .expect("an old asking reads");
        assert_eq!(old.kind, Kind::Request);
        assert_eq!(old.state, RequestState::Pending);
        let json = serde_json::to_string(&note("p")).expect("json");
        assert!(!json.contains("shape") && !json.contains("page"), "{json}");
        let mut design = note("d");
        design.kind = Kind::Design;
        assert!(checked(design.clone()).is_err(), "a design is named");
        design.component = "risk-badge".to_string();
        assert!(checked(design.clone()).is_err(), "named like a Micro-UI");
        design.component = "RiskBadge".to_string();
        assert!(checked(design).is_ok());
        let mut referenced = note("r");
        referenced.source = "ftp://x".to_string();
        assert!(
            checked(referenced.clone()).is_err(),
            "a reference is a link"
        );
        referenced.source = "https://www.figma.com/design/abc/Frame?node-id=1-2".to_string();
        assert!(checked(referenced).is_ok());
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
        let mut silent = note("a");
        silent.kind = Kind::Brainstorm;
        silent.text = "  ".to_string();
        assert!(
            checked(silent.clone())
                .unwrap_err()
                .contains("write it first")
        );
        silent.kind = Kind::Request;
        assert!(
            checked(silent.clone()).is_ok(),
            "a request may wait for its words"
        );
        silent.kind = Kind::Note;
        assert!(checked(silent).is_ok(), "an empty note is still a note");
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
            text in "[a-z]\\PC{0,199}",
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
