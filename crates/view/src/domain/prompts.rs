//! A station's prompts as the pane shows them, and the human's edits to them.
//!
//! The one place the view writes something a run reads: the prompt book
//! (`.llocal/prompts.json`, `harness_core::ports::store::prompts`). What a
//! prompt is — its key, its station, its default — is `harness-workflows`'
//! catalog; what an edit, a restore and a reset do is `harness-core`'s
//! `PromptBook`. This module only puts the two together for the page.

use harness_core::domain::prompt_book::{self, BookError, PromptBook};
use harness_workflows::prompts::{self as catalog, Prompt};
use serde::{Deserialize, Serialize};

/// One edit kept, as the pane lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Kept {
    /// The text.
    pub text: String,
    /// When it was saved, Unix seconds.
    pub saved_at: u64,
    /// Whether it is what the stage sends now.
    pub current: bool,
}

/// One prompt, as the pane shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptView {
    /// What the edits are kept under.
    pub key: String,
    /// What a human calls it.
    pub title: String,
    /// Every station that sends it, as `line/station` — more than one when
    /// an edit here changes another station too.
    pub stations: Vec<String>,
    /// What the stage sends now.
    pub text: String,
    /// The workflow's own text.
    pub default: String,
    /// Whether `text` is a human's edit rather than the default.
    pub edited: bool,
    /// The last edits, newest first.
    pub history: Vec<Kept>,
    /// The `{name}` placeholders the stage fills in.
    pub placeholders: Vec<String>,
    /// Placeholders of the default the current text dropped.
    pub missing: Vec<String>,
    /// Placeholders in the current text that nothing fills.
    pub unknown: Vec<String>,
}

/// What the page sends to save a prompt.
#[derive(Debug, Deserialize)]
pub struct Edit {
    /// The prompt's key.
    pub key: String,
    /// The new text.
    pub text: String,
}

/// What the page sends to put a version back.
#[derive(Debug, Deserialize)]
pub struct Restore {
    /// The prompt's key.
    pub key: String,
    /// The history's index, newest first — `None`: the default.
    pub version: Option<usize>,
}

/// Why a change was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// No prompt by that key.
    Unknown(String),
    /// The book refused it.
    Book(BookError),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(key) => write!(f, "no prompt named {key}"),
            Self::Book(why) => write!(f, "{why}"),
        }
    }
}

/// One prompt of the catalog, with the book's edits.
#[must_use]
pub fn view(book: &PromptBook, prompt: &Prompt) -> PromptView {
    let text = book.text(prompt.key, prompt.default);
    let entry = book.entry(prompt.key);
    let edited = entry.is_some_and(|entry| entry.current.is_some());
    let history = entry
        .map(|entry| {
            entry
                .history
                .iter()
                .map(|version| Kept {
                    text: version.text.clone(),
                    saved_at: version.saved_at,
                    current: edited && version.text == text,
                })
                .collect()
        })
        .unwrap_or_default();
    let (missing, unknown) = prompt_book::drift(prompt.default, &text);
    PromptView {
        key: prompt.key.to_string(),
        title: prompt.title.to_string(),
        stations: prompt
            .stations
            .iter()
            .map(|(line, station)| format!("{line}/{station}"))
            .collect(),
        default: prompt.default.to_string(),
        edited,
        history,
        placeholders: prompt_book::placeholders(prompt.default)
            .into_iter()
            .collect(),
        missing,
        unknown,
        text,
    }
}

/// The prompts one station sends, edits included. Empty for a station that
/// sends none (a gate, a free step).
#[must_use]
pub fn of_station(book: &PromptBook, line: &str, station: &str) -> Vec<PromptView> {
    catalog::of_station(line, station)
        .iter()
        .map(|prompt| view(book, prompt))
        .collect()
}

/// Saves a human's text.
///
/// # Errors
///
/// [`Refused::Unknown`] for a key the catalog does not hold, [`Refused::Book`]
/// for a text the book refuses. The book is then unchanged.
pub fn edit(book: &mut PromptBook, edit: &Edit, now: u64) -> Result<PromptView, Refused> {
    let prompt = catalog::find(&edit.key).ok_or_else(|| Refused::Unknown(edit.key.clone()))?;
    book.edit(prompt.key, &edit.text, prompt.default, now)
        .map_err(Refused::Book)?;
    Ok(view(book, &prompt))
}

/// Puts back a version of the history, or the default.
///
/// # Errors
///
/// As [`edit`], and [`BookError::NoSuchVersion`] past the history.
pub fn restore(book: &mut PromptBook, restore: &Restore, now: u64) -> Result<PromptView, Refused> {
    let prompt =
        catalog::find(&restore.key).ok_or_else(|| Refused::Unknown(restore.key.clone()))?;
    match restore.version {
        None => book.reset(prompt.key),
        Some(index) => book
            .restore(prompt.key, index, prompt.default, now)
            .map_err(Refused::Book)?,
    }
    Ok(view(book, &prompt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::blueprint::{self, Kind};

    #[test]
    fn every_robot_shows_a_prompt_and_every_prompt_stands_on_a_station() {
        let book = PromptBook::default();
        let lines = blueprint::lines();
        for line in &lines {
            for station in &line.stations {
                let paid = matches!(station.kind, Kind::Builder | Kind::Inspector);
                assert_eq!(
                    paid,
                    !of_station(&book, &line.id, &station.id).is_empty(),
                    "{}/{}",
                    line.id,
                    station.id
                );
            }
        }
        for prompt in catalog::catalog() {
            for (line, station) in prompt.stations {
                assert!(
                    lines
                        .iter()
                        .any(|l| l.id == *line && l.stations.iter().any(|s| s.id == *station)),
                    "{} names {line}/{station}, which the plant does not draw",
                    prompt.key
                );
            }
        }
    }

    #[test]
    fn an_edit_shows_as_the_text_with_its_drift_and_a_reset_brings_the_default_back() {
        let mut book = PromptBook::default();
        let saved = edit(
            &mut book,
            &Edit {
                key: catalog::FIX.to_string(),
                text: "Repair PR #{num} {nope}".to_string(),
            },
            5,
        )
        .expect("saved");
        assert!(saved.edited);
        assert_eq!(saved.text, "Repair PR #{num} {nope}");
        assert!(saved.missing.contains(&"title".to_string()));
        assert_eq!(saved.unknown, ["nope"]);
        assert!(saved.history[0].current);

        let back = restore(
            &mut book,
            &Restore {
                key: catalog::FIX.to_string(),
                version: None,
            },
            6,
        )
        .expect("reset");
        assert!(!back.edited);
        assert_eq!(back.text, back.default);
        assert!(back.missing.is_empty() && back.unknown.is_empty());
        assert!(!back.history[0].current, "kept, no longer sent");
    }

    #[test]
    fn an_unknown_key_changes_nothing() {
        let mut book = PromptBook::default();
        let refused = edit(
            &mut book,
            &Edit {
                key: "../etc".to_string(),
                text: "x".to_string(),
            },
            1,
        );
        assert_eq!(refused, Err(Refused::Unknown("../etc".to_string())));
        assert_eq!(book, PromptBook::default());
    }
}
