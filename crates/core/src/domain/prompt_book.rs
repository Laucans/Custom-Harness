//! The human's edits to the prompts a stage sends, and the last few kept.
//!
//! A workflow declares each stage's prompt as a constant: the *default*. A
//! human may replace it; the replacement is what the stage sends from the next
//! run on, until it is reset. Each prompt keeps its last [`KEPT`] edits,
//! newest first, so one can be put back. The default is never in that list:
//! it is always there to go back to.
//!
//! Pure: what a version is, and what an edit, a restore and a reset do. Where
//! the book is kept is the
//! [`PromptStore`](crate::ports::store::prompts::PromptStore) port's.
//!
//! **This module knows no workflow.** A prompt is a key and a default text,
//! both passed in by the caller.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// How many edits of one prompt are kept.
pub const KEPT: usize = 5;

/// The longest prompt a human may save, in characters.
///
/// The longest default is a few thousand; this leaves room to write, and
/// keeps a pasted log from becoming every session's first turn.
pub const MAX_CHARS: usize = 64 * 1024;

/// One edit, as it was saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// The prompt's text.
    pub text: String,
    /// When it was saved, in seconds since the Unix epoch.
    pub saved_at: u64,
}

/// One prompt's edits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// What the stage sends instead of its default — `None`: the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    /// The last [`KEPT`] edits, newest first, no two alike.
    #[serde(default)]
    pub history: Vec<Version>,
}

/// Every prompt a human edited, by key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptBook {
    #[serde(default)]
    prompts: BTreeMap<String, Entry>,
}

/// Why an edit was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BookError {
    /// A blank prompt: a stage sending nothing is never what a human meant.
    #[error("a prompt cannot be empty — reset it to go back to the default")]
    Empty,
    /// Past [`MAX_CHARS`].
    #[error("a prompt is at most {MAX_CHARS} characters, this one is {0}")]
    TooLong(usize),
    /// A restore named a version the history does not hold.
    #[error("no version {0} in this prompt's history")]
    NoSuchVersion(usize),
}

impl PromptBook {
    /// What the stage sends: the human's edit if there is one, else `default`.
    #[must_use]
    pub fn text(&self, key: &str, default: &str) -> String {
        self.prompts
            .get(key)
            .and_then(|entry| entry.current.clone())
            .unwrap_or_else(|| default.to_string())
    }

    /// This prompt's edits, if a human ever made one.
    #[must_use]
    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.prompts.get(key)
    }

    /// Saves `text` as what the stage sends, and keeps it in the history.
    ///
    /// A text equal to `default` is a reset: nothing to keep, the default is
    /// always there. A text already in the history moves to its head rather
    /// than appearing twice.
    ///
    /// # Errors
    ///
    /// [`BookError::Empty`] for a blank text, [`BookError::TooLong`] past
    /// [`MAX_CHARS`]. The book is unchanged.
    pub fn edit(
        &mut self,
        key: &str,
        text: &str,
        default: &str,
        saved_at: u64,
    ) -> Result<(), BookError> {
        if text.trim().is_empty() {
            return Err(BookError::Empty);
        }
        let chars = text.chars().count();
        if chars > MAX_CHARS {
            return Err(BookError::TooLong(chars));
        }
        if text == default {
            self.reset(key);
            return Ok(());
        }
        let entry = self.prompts.entry(key.to_string()).or_default();
        entry.current = Some(text.to_string());
        entry.history.retain(|version| version.text != text);
        entry.history.insert(
            0,
            Version {
                text: text.to_string(),
                saved_at,
            },
        );
        entry.history.truncate(KEPT);
        Ok(())
    }

    /// Puts back the history's `index`-th version (0 is the newest).
    ///
    /// # Errors
    ///
    /// [`BookError::NoSuchVersion`] when the history is shorter.
    pub fn restore(
        &mut self,
        key: &str,
        index: usize,
        default: &str,
        saved_at: u64,
    ) -> Result<(), BookError> {
        let text = self
            .entry(key)
            .and_then(|entry| entry.history.get(index))
            .map(|version| version.text.clone())
            .ok_or(BookError::NoSuchVersion(index))?;
        self.edit(key, &text, default, saved_at)
    }

    /// The stage sends its default again; the history stays.
    pub fn reset(&mut self, key: &str) {
        if let Some(entry) = self.prompts.get_mut(key) {
            entry.current = None;
        }
    }
}

/// The `{name}` placeholders a prompt carries — what the stage fills in.
///
/// The same reading as [`splice`](crate::domain::prompts::splice): a name is
/// what lies between a `{` and the next `}`, kept only when it looks like an
/// identifier, since prompts carry braces that are not placeholders (a JSON
/// example, a `{ }` in prose).
#[must_use]
pub fn placeholders(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let name = &after[..end];
        if is_identifier(name) {
            found.insert(name.to_string());
            rest = &after[end + 1..];
        } else {
            rest = after;
        }
    }
    found
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// What an edit loses and what it adds against the default's placeholders.
///
/// `(missing, unknown)`: a placeholder of the default the edit dropped — the
/// stage will not hear that value — and one the edit names that nothing fills
/// — it reaches the session as written.
#[must_use]
pub fn drift(default: &str, text: &str) -> (Vec<String>, Vec<String>) {
    let wanted = placeholders(default);
    let named = placeholders(text);
    (
        wanted.difference(&named).cloned().collect(),
        named.difference(&wanted).cloned().collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const DEFAULT: &str = "Issue #{num} — {title}";

    #[test]
    fn an_untouched_prompt_is_its_default() {
        let book = PromptBook::default();
        assert_eq!(book.text("a/b", DEFAULT), DEFAULT);
    }

    #[test]
    fn an_edit_is_what_the_stage_sends_and_the_history_s_head() {
        let mut book = PromptBook::default();
        book.edit("a/b", "Fix #{num}", DEFAULT, 10).expect("saved");
        assert_eq!(book.text("a/b", DEFAULT), "Fix #{num}");
        let entry = book.entry("a/b").expect("kept");
        assert_eq!(entry.history[0].text, "Fix #{num}");
        assert_eq!(entry.history[0].saved_at, 10);
    }

    #[test]
    fn only_the_last_five_edits_are_kept_newest_first() {
        let mut book = PromptBook::default();
        for n in 0..8 {
            book.edit("k", &format!("v{n}"), DEFAULT, n).expect("saved");
        }
        let texts: Vec<&str> = book
            .entry("k")
            .expect("kept")
            .history
            .iter()
            .map(|v| v.text.as_str())
            .collect();
        assert_eq!(texts, ["v7", "v6", "v5", "v4", "v3"]);
    }

    #[test]
    fn restoring_moves_a_version_to_the_head_without_a_duplicate() {
        let mut book = PromptBook::default();
        for n in 0..3 {
            book.edit("k", &format!("v{n}"), DEFAULT, n).expect("saved");
        }
        book.restore("k", 2, DEFAULT, 9).expect("restored");
        assert_eq!(book.text("k", DEFAULT), "v0");
        let texts: Vec<&str> = book
            .entry("k")
            .expect("kept")
            .history
            .iter()
            .map(|v| v.text.as_str())
            .collect();
        assert_eq!(texts, ["v0", "v2", "v1"]);
        assert_eq!(
            book.restore("k", 3, DEFAULT, 9),
            Err(BookError::NoSuchVersion(3))
        );
    }

    #[test]
    fn a_reset_sends_the_default_and_keeps_the_history() {
        let mut book = PromptBook::default();
        book.edit("k", "mine", DEFAULT, 1).expect("saved");
        book.reset("k");
        assert_eq!(book.text("k", DEFAULT), DEFAULT);
        assert_eq!(book.entry("k").expect("kept").history.len(), 1);
    }

    #[test]
    fn saving_the_default_text_is_a_reset_not_an_edit() {
        let mut book = PromptBook::default();
        book.edit("k", "mine", DEFAULT, 1).expect("saved");
        book.edit("k", DEFAULT, DEFAULT, 2).expect("saved");
        let entry = book.entry("k").expect("kept");
        assert_eq!(entry.current, None);
        assert_eq!(entry.history.len(), 1);
    }

    #[test]
    fn a_blank_or_huge_prompt_is_refused_and_changes_nothing() {
        let mut book = PromptBook::default();
        assert_eq!(book.edit("k", "  \n", DEFAULT, 1), Err(BookError::Empty));
        let huge = "x".repeat(MAX_CHARS + 1);
        assert_eq!(
            book.edit("k", &huge, DEFAULT, 1),
            Err(BookError::TooLong(MAX_CHARS + 1))
        );
        assert_eq!(book, PromptBook::default());
    }

    #[test]
    fn placeholders_are_identifiers_between_braces_never_json() {
        let found = placeholders(r#"#{num} {title} {"a": 1} { } {Upper} {x_2}"#);
        let found: Vec<&str> = found.iter().map(String::as_str).collect();
        assert_eq!(found, ["num", "title", "x_2"]);
    }

    #[test]
    fn drift_names_what_an_edit_dropped_and_what_it_invented() {
        let (missing, unknown) = drift(DEFAULT, "Issue #{num} {nmu}");
        assert_eq!(missing, ["title"]);
        assert_eq!(unknown, ["nmu"]);
    }

    proptest! {
        #[test]
        fn edits_keep_the_history_short_unique_and_headed_by_what_is_sent(
            texts in proptest::collection::vec("[a-c]{1,2}", 1..20),
        ) {
            let mut book = PromptBook::default();
            for (at, text) in texts.iter().enumerate() {
                book.edit("k", text, DEFAULT, at as u64).expect("saved");
            }
            let entry = book.entry("k").expect("kept");
            prop_assert!(entry.history.len() <= KEPT);
            let unique: BTreeSet<&str> = entry.history.iter().map(|v| v.text.as_str()).collect();
            prop_assert_eq!(unique.len(), entry.history.len());
            let last = texts.last().expect("one at least");
            prop_assert_eq!(&book.text("k", DEFAULT), last);
            prop_assert_eq!(&entry.history[0].text, last);
        }

        #[test]
        fn a_book_survives_its_own_json(
            texts in proptest::collection::vec("\\PC{1,40}", 1..8),
        ) {
            let mut book = PromptBook::default();
            for (at, text) in texts.iter().enumerate() {
                let _ = book.edit(&format!("k{}", at % 3), text, DEFAULT, at as u64);
            }
            let json = serde_json::to_string(&book).expect("encoded");
            let back: PromptBook = serde_json::from_str(&json).expect("decoded");
            prop_assert_eq!(back, book);
        }
    }
}
