//! A pin's journal: what happened to it, in order, never cut — placed,
//! written, proposed (and what the AI said), validated, taken back, the
//! rules it moved, and each session the human had about it (a discussion
//! or a rework), marked over the pin's kept conversation. The mock-up's
//! history undoes; the journal tells.

use serde::{Deserialize, Serialize};

/// How long, in milliseconds, the words of a pin may go on being written
/// and still be one event.
pub const WORDS_MS: u64 = 120_000;

/// A session over the pin's conversation: which turns are its own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mark {
    /// `discuss` or `rework`.
    pub mode: String,
    /// The first of its turns in the pin's conversation.
    pub from: usize,
    /// One past its last turn, once it closed.
    pub to: Option<usize>,
    /// Closed validated (`true`) or dropped (`false`).
    pub kept: Option<bool>,
}

/// One thing that happened to a pin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Event {
    /// Its number in the pin's journal, in order.
    pub seq: u64,
    /// When, in milliseconds.
    pub ms: u64,
    /// What it was: `placed`, `words`, `kind`, `proposed`, `not proposed`,
    /// `validated`, `taken back`, `rules`, `session`.
    pub kind: String,
    /// What it came to, in words.
    pub said: String,
    /// The files it changed, under `mock/`.
    pub files: Vec<String>,
    /// For a session: the turns it holds.
    pub session: Option<Mark>,
}

/// What an event does to a journal (`kept`, in order): written words go
/// on in the last event while they keep coming; anything else is added
/// after the last. Answers the event to keep, numbered.
#[must_use]
pub fn added(kept: &[Event], mut new: Event) -> Event {
    let last = kept.last();
    if new.kind == "words"
        && let Some(l) =
            last.filter(|l| l.kind == "words" && new.ms.saturating_sub(l.ms) <= WORDS_MS)
    {
        new.seq = l.seq;
        return new;
    }
    new.seq = last.map_or(1, |l| l.seq.saturating_add(1));
    new
}

/// The session still open in a journal, closed: its last turn, how it
/// ended, the files it changed. Answers the event to keep; `None` when no
/// session is open.
#[must_use]
pub fn closed(kept: &[Event], to: usize, kept_it: bool, files: Vec<String>) -> Option<Event> {
    let open = kept
        .iter()
        .rev()
        .find(|e| e.session.as_ref().is_some_and(|m| m.to.is_none()))?;
    let mut done = open.clone();
    if let Some(mark) = done.session.as_mut() {
        mark.to = Some(to);
        mark.kept = Some(kept_it);
    }
    done.files = files;
    Some(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, ms: u64) -> Event {
        Event {
            kind: kind.to_string(),
            ms,
            ..Event::default()
        }
    }

    #[test]
    fn words_go_on_in_one_event_while_they_keep_coming_anything_else_follows() {
        let mut kept: Vec<Event> = Vec::new();
        let keep = |e: Event, kept: &mut Vec<Event>| {
            let e = added(kept, e);
            match kept.iter_mut().find(|k| k.seq == e.seq) {
                Some(k) => *k = e,
                None => kept.push(e),
            }
        };
        keep(event("placed", 0), &mut kept);
        keep(event("words", 1_000), &mut kept);
        keep(event("words", 60_000), &mut kept);
        assert_eq!(kept.len(), 2, "the words go on");
        keep(event("words", 60_000 + WORDS_MS + 1), &mut kept);
        assert_eq!(kept.len(), 3, "a pause past the window starts another");
        keep(event("proposed", 400_000), &mut kept);
        keep(event("words", 400_001), &mut kept);
        assert_eq!(
            kept.iter()
                .map(|e| (e.seq, e.kind.as_str()))
                .collect::<Vec<_>>(),
            [
                (1, "placed"),
                (2, "words"),
                (3, "words"),
                (4, "proposed"),
                (5, "words")
            ]
        );
    }

    #[test]
    fn a_session_is_closed_on_its_turns_once() {
        let open = Event {
            seq: 3,
            kind: "session".into(),
            session: Some(Mark {
                mode: "rework".into(),
                from: 4,
                ..Mark::default()
            }),
            ..Event::default()
        };
        let kept = vec![event("placed", 0), open];
        let done = closed(&kept, 9, true, vec!["pages/a.html".into()]).expect("closed");
        assert_eq!(done.seq, 3);
        assert_eq!(
            done.session,
            Some(Mark {
                mode: "rework".into(),
                from: 4,
                to: Some(9),
                kept: Some(true),
            })
        );
        assert_eq!(done.files, ["pages/a.html"]);
        assert!(
            closed(&[done], 10, false, Vec::new()).is_none(),
            "none open"
        );
    }
}
