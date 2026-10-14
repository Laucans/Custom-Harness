//! The mock-up's history, for Cmd+Z: one line of what the human did on a
//! branch's mock-up and its rules — pins placed, moved, written, deleted,
//! rules edited, accepted or refused, requests proposed, validated or taken
//! back, the page retouched in edit mode — each an entry of facets, every
//! thing it changed as it was and as it became. An undo writes the befores
//! back, a redo the afters; a new entry forgets what was undone. The big
//! generations (a drawing, the cut, a reading of all the rules) are not in
//! it.

use serde::{Deserialize, Serialize};

use crate::domain::annotations::Note;
use crate::domain::creation::RulesState;
use crate::domain::drafts::Draft;
use crate::domain::steps::Step;

/// How many entries a branch's history keeps: the oldest goes past it.
pub const DEPTH: usize = 30;

/// How long, in milliseconds, a pause in typing may last and still be the
/// same entry: past it, the next keystroke starts another.
pub const TYPING_MS: u64 = 1500;

/// One thing an entry changed, as it was and as it became.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Facet {
    /// A file of the mock-up (`mock/<path>`); `None`, it is not there.
    File {
        /// Its path under `mock/`.
        path: String,
        /// As it was.
        before: Option<String>,
        /// As it became.
        after: Option<String>,
    },
    /// A pin of a board; `None`, it is not on it.
    Pin {
        /// The board.
        board: String,
        /// The pin's id.
        id: String,
        /// As it was.
        before: Option<Box<Note>>,
        /// As it became.
        after: Option<Box<Note>>,
    },
    /// The rules of the creation.
    Rules {
        /// As they were.
        before: Box<RulesState>,
        /// As they became.
        after: Box<RulesState>,
    },
    /// A page's draft: the version open; `None`, there was none.
    Draft {
        /// The page's board.
        board: String,
        /// As it was.
        before: Option<Box<Draft>>,
        /// As it became.
        after: Option<Box<Draft>>,
    },
    /// The request pins' steps of the branch.
    Steps {
        /// As they were.
        before: Vec<Step>,
        /// As they became.
        after: Vec<Step>,
    },
}

impl Facet {
    /// What it is about: two facets about the same thing fold into one.
    #[must_use]
    pub fn about(&self) -> String {
        match self {
            Self::File { path, .. } => format!("file {path}"),
            Self::Pin { board, id, .. } => format!("pin {board} {id}"),
            Self::Rules { .. } => "rules".to_string(),
            Self::Draft { board, .. } => format!("draft {board}"),
            Self::Steps { .. } => "steps".to_string(),
        }
    }

    /// It changed nothing.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        match self {
            Self::File { before, after, .. } => before == after,
            Self::Pin { before, after, .. } => before == after,
            Self::Rules { before, after } => before == after,
            Self::Draft { before, after, .. } => before == after,
            Self::Steps { before, after } => before == after,
        }
    }

    /// The board it is on, when it is a pin's or a draft's.
    #[must_use]
    pub fn board(&self) -> Option<&str> {
        match self {
            Self::Pin { board, .. } | Self::Draft { board, .. } => Some(board),
            _ => None,
        }
    }

    /// The same facet with `later`'s after: the two as one change.
    #[must_use]
    pub fn folded(self, later: Self) -> Self {
        match (self, later) {
            (Self::File { path, before, .. }, Self::File { after, .. }) => Self::File {
                path,
                before,
                after,
            },
            (
                Self::Pin {
                    board, id, before, ..
                },
                Self::Pin { after, .. },
            ) => Self::Pin {
                board,
                id,
                before,
                after,
            },
            (Self::Rules { before, .. }, Self::Rules { after, .. }) => {
                Self::Rules { before, after }
            }
            (Self::Draft { board, before, .. }, Self::Draft { after, .. }) => Self::Draft {
                board,
                before,
                after,
            },
            (Self::Steps { before, .. }, Self::Steps { after, .. }) => {
                Self::Steps { before, after }
            }
            (_, later) => later,
        }
    }

    /// The facet the other way round: what an undo writes is a redo's before.
    #[must_use]
    pub fn reversed(self) -> Self {
        match self {
            Self::File {
                path,
                before,
                after,
            } => Self::File {
                path,
                before: after,
                after: before,
            },
            Self::Pin {
                board,
                id,
                before,
                after,
            } => Self::Pin {
                board,
                id,
                before: after,
                after: before,
            },
            Self::Rules { before, after } => Self::Rules {
                before: after,
                after: before,
            },
            Self::Draft {
                board,
                before,
                after,
            } => Self::Draft {
                board,
                before: after,
                after: before,
            },
            Self::Steps { before, after } => Self::Steps {
                before: after,
                after: before,
            },
        }
    }
}

/// One thing the human did, as the history keeps it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// Its number on the branch, in order.
    pub id: u64,
    /// The gesture it belongs to: the requests of one gesture make one
    /// entry. Empty: a request of its own.
    pub token: String,
    /// What it was, in words.
    pub what: String,
    /// When it was last changed, in milliseconds.
    pub at: u64,
    /// Taken back: a redo may bring it again.
    pub undone: bool,
    /// What it changed.
    pub facets: Vec<Facet>,
}

impl Entry {
    /// The boards it touched.
    #[must_use]
    pub fn boards(&self) -> Vec<String> {
        let mut boards: Vec<String> = Vec::new();
        for b in self.facets.iter().filter_map(Facet::board) {
            if !boards.iter().any(|x| x == b) {
                boards.push(b.to_string());
            }
        }
        boards
    }

    /// It touched the rules.
    #[must_use]
    pub fn touches_rules(&self) -> bool {
        self.facets.iter().any(|f| matches!(f, Facet::Rules { .. }))
    }

    /// Takes in a facet: folded into one about the same thing, else added.
    fn absorb(&mut self, facet: Facet) {
        let about = facet.about();
        if let Some(at) = self.facets.iter().position(|f| f.about() == about) {
            let was = self.facets.remove(at);
            self.facets.insert(at, was.folded(facet));
        } else {
            self.facets.push(facet);
        }
    }

    /// Typing that may go on in it: one facet, a pin's words or the rules.
    fn is_typing(&self) -> bool {
        self.token.is_empty()
            && self.facets.len() == 1
            && self.facets.iter().all(|f| match f {
                Facet::Pin { before, after, .. } => match (before, after) {
                    (Some(b), Some(a)) => {
                        let mut same = a.as_ref().clone();
                        same.text.clone_from(&b.text);
                        same.at.clone_from(&b.at);
                        same == **b
                    }
                    _ => false,
                },
                Facet::Rules { .. } | Facet::Draft { .. } => true,
                _ => false,
            })
    }
}

/// What recording an entry changes in the kept history.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    /// The entries to keep, new or changed.
    pub put: Vec<Entry>,
    /// The entries to forget, by id.
    pub remove: Vec<u64>,
}

/// An entry recorded in a branch's history (`kept`, in order): what was
/// undone is forgotten, the gesture it belongs to — or the typing it goes
/// on with — takes it in, else it is added, the oldest going past
/// [`DEPTH`]. An entry that changes nothing is no entry.
#[must_use]
pub fn record(kept: &[Entry], new: &Entry) -> Changes {
    let mut changes = Changes::default();
    let facets: Vec<Facet> = new
        .facets
        .iter()
        .filter(|f| !f.is_idle())
        .cloned()
        .collect();
    if facets.is_empty() {
        return changes;
    }
    changes.remove = kept.iter().filter(|e| e.undone).map(|e| e.id).collect();
    let done: Vec<&Entry> = kept.iter().filter(|e| !e.undone).collect();
    let last = done.last().copied();
    let mut entry = Entry {
        facets: Vec::new(),
        ..new.clone()
    };
    for f in facets.clone() {
        entry.absorb(f);
    }
    let goes_on = last.is_some_and(|l| {
        (!new.token.is_empty() && l.token == new.token)
            || (new.token.is_empty()
                && l.is_typing()
                && entry.is_typing()
                && l.facets
                    .iter()
                    .map(Facet::about)
                    .eq(entry.facets.iter().map(Facet::about))
                && new.at.saturating_sub(l.at) <= TYPING_MS)
    });
    if let (true, Some(l)) = (goes_on, last) {
        let mut grown = l.clone();
        for f in facets {
            grown.absorb(f);
        }
        grown.at = new.at;
        grown.facets.retain(|f| !f.is_idle());
        if grown.facets.is_empty() {
            changes.remove.push(grown.id);
        } else {
            changes.put.push(grown);
        }
        return changes;
    }
    entry.id = kept
        .iter()
        .map(|e| e.id)
        .max()
        .map_or(1, |m| m.saturating_add(1));
    let over = done.len().saturating_add(1).saturating_sub(DEPTH);
    changes.remove.extend(done.iter().take(over).map(|e| e.id));
    changes.put.push(entry);
    changes
}

/// The entry an undo takes back: the last one done.
#[must_use]
pub fn next_undo(kept: &[Entry]) -> Option<&Entry> {
    kept.iter().rev().find(|e| !e.undone)
}

/// The entry a redo brings again: the first of those undone.
#[must_use]
pub fn next_redo(kept: &[Entry]) -> Option<&Entry> {
    kept.iter().find(|e| e.undone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn note(text: &str, x: f64) -> Box<Note> {
        Box::new(Note {
            id: "p".to_string(),
            text: text.to_string(),
            x,
            ..Note::default()
        })
    }

    fn pin(before: Option<Box<Note>>, after: Option<Box<Note>>) -> Facet {
        Facet::Pin {
            board: "mockup--main--home".to_string(),
            id: "p".to_string(),
            before,
            after,
        }
    }

    fn file(path: &str, before: Option<&str>, after: Option<&str>) -> Facet {
        Facet::File {
            path: path.to_string(),
            before: before.map(ToString::to_string),
            after: after.map(ToString::to_string),
        }
    }

    fn entry(token: &str, at: u64, facets: Vec<Facet>) -> Entry {
        Entry {
            token: token.to_string(),
            what: "w".to_string(),
            at,
            facets,
            ..Entry::default()
        }
    }

    /// The history as it stands once `changes` are kept.
    fn kept_after(kept: &[Entry], changes: Changes) -> Vec<Entry> {
        let mut out: Vec<Entry> = kept
            .iter()
            .filter(|e| !changes.remove.contains(&e.id))
            .cloned()
            .collect();
        for e in changes.put {
            match out.iter_mut().find(|x| x.id == e.id) {
                Some(x) => *x = e,
                None => out.push(e),
            }
        }
        out.sort_by_key(|e| e.id);
        out
    }

    #[test]
    fn a_gesture_s_requests_make_one_entry_and_typing_goes_on_until_a_pause() {
        let mut kept = Vec::new();
        // One gesture: the draft, then the pin's state.
        let c = record(
            &kept,
            &entry(
                "g1",
                0,
                vec![file("components/X.draft.html", None, Some("d"))],
            ),
        );
        kept = kept_after(&kept, c);
        let c = record(
            &kept,
            &entry(
                "g1",
                9_000,
                vec![pin(Some(note("ask", 0.0)), Some(note("ask", 1.0)))],
            ),
        );
        kept = kept_after(&kept, c);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].facets.len(), 2);
        assert_eq!(kept[0].boards(), ["mockup--main--home"]);
        assert!(!kept[0].touches_rules());

        // Typing: the words of one pin, keystroke after keystroke.
        for (i, text) in ["a", "ab", "abc"].iter().enumerate() {
            let before = ["", "a", "ab"][i];
            let c = record(
                &kept,
                &entry(
                    "",
                    10_000 + i as u64 * 500,
                    vec![pin(Some(note(before, 1.0)), Some(note(text, 1.0)))],
                ),
            );
            kept = kept_after(&kept, c);
        }
        assert_eq!(kept.len(), 2, "one entry for the typing");
        assert_eq!(
            kept[1].facets,
            [pin(Some(note("", 1.0)), Some(note("abc", 1.0)))]
        );
        // A pause past the window: another entry.
        let c = record(
            &kept,
            &entry(
                "",
                20_000,
                vec![pin(Some(note("abc", 1.0)), Some(note("abcd", 1.0)))],
            ),
        );
        kept = kept_after(&kept, c);
        assert_eq!(kept.len(), 3);
        // A move is never typing.
        let c = record(
            &kept,
            &entry(
                "",
                20_100,
                vec![pin(Some(note("abcd", 1.0)), Some(note("abcd", 5.0)))],
            ),
        );
        kept = kept_after(&kept, c);
        assert_eq!(kept.len(), 4);
        // Nothing changed: no entry.
        assert_eq!(
            record(
                &kept,
                &entry(
                    "",
                    30_000,
                    vec![pin(Some(note("x", 1.0)), Some(note("x", 1.0)))]
                )
            ),
            Changes::default()
        );
    }

    #[test]
    fn what_was_undone_is_forgotten_by_a_new_entry_and_the_oldest_goes_past_the_depth() {
        let mut kept: Vec<Entry> = (1..=DEPTH as u64)
            .map(|i| Entry {
                id: i,
                facets: vec![file("f", None, Some("x"))],
                ..Entry::default()
            })
            .collect();
        kept[DEPTH - 1].undone = true;
        assert_eq!(next_undo(&kept).map(|e| e.id), Some(DEPTH as u64 - 1));
        assert_eq!(next_redo(&kept).map(|e| e.id), Some(DEPTH as u64));
        let c = record(&kept, &entry("", 0, vec![file("g", None, Some("y"))]));
        assert_eq!(c.remove, [DEPTH as u64], "the undone is forgotten");
        assert_eq!(c.put.len(), 1);
        assert_eq!(c.put[0].id, DEPTH as u64 + 1);
        kept = kept_after(&kept, c);
        let c = record(&kept, &entry("", 0, vec![file("h", None, Some("z"))]));
        assert_eq!(c.remove, [1], "the oldest goes past the depth");
        assert!(next_redo(&kept_after(&kept, c)).is_none());
    }

    #[test]
    fn a_facet_folds_and_reverses() {
        let f = file("f", Some("a"), Some("b")).folded(file("f", Some("b"), Some("c")));
        assert_eq!(f, file("f", Some("a"), Some("c")));
        assert_eq!(f.reversed(), file("f", Some("c"), Some("a")));
        assert!(file("f", Some("a"), Some("a")).is_idle());
        assert_eq!(pin(None, None).about(), "pin mockup--main--home p");
    }

    proptest! {
        #[test]
        fn the_history_never_keeps_more_than_its_depth(count in 1usize..80) {
            let mut kept: Vec<Entry> = Vec::new();
            for i in 0..count {
                let c = record(&kept, &entry("", (i as u64) * 10_000, vec![file(&format!("f{i}"), None, Some("x"))]));
                kept = kept_after(&kept, c);
            }
            prop_assert!(kept.len() <= DEPTH);
            prop_assert_eq!(kept.len(), count.min(DEPTH));
        }
    }
}
