//! The human's edits to the prompts a stage sends: what runs now, what waits
//! for the next start, and the last few kept.
//!
//! A workflow declares each stage's prompt as a constant: the *default*. A
//! human may replace it, in two steps:
//!
//! - a **draft** ([`Drafts`]) is what the human proposed and nothing runs yet.
//!   Proposing replaces the draft of that prompt; a draft equal to what runs
//!   is no draft at all.
//! - the **book** ([`PromptBook`]) is what runs. Drafts reach it only through
//!   [`PromptBook::apply`], which the harness calls itself when the factory
//!   starts — so whatever proposes a change never writes what a run reads.
//!
//! A draft the human sets aside is **deactivated**: it leaves the drafts for
//! the [`Shelf`], where it waits to be reactivated or deleted. The shelf is
//! kept apart from the drafts because the harness takes every draft at its
//! start, and a deactivated one must outlive that.
//!
//! Each prompt of the book keeps its last [`KEPT`] edits, newest first, and
//! its last [`REVERTS_KEPT`] reverted texts — what a revert replaced — so
//! either can be put back. The default is never in a list: it is always there
//! to go back to.
//!
//! Pure: the stores are the
//! [`prompts`](crate::ports::store::prompts) ports'.
//!
//! **This module knows no workflow.** A prompt is a key and a default text,
//! both passed in by the caller.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// How many edits of one prompt are kept.
pub const KEPT: usize = 5;

/// How many reverted texts of one prompt are kept.
pub const REVERTS_KEPT: usize = 3;

/// How many deactivated drafts of one prompt are kept.
pub const SHELVED_KEPT: usize = 10;

/// The longest prompt a human may save, in characters.
///
/// The longest default is a few thousand; this leaves room to write, and
/// keeps a pasted log from becoming every session's first turn.
pub const MAX_CHARS: usize = 64 * 1024;

/// One text, as it was kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// The prompt's text.
    pub text: String,
    /// When it was kept, in seconds since the Unix epoch.
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
    /// The last [`REVERTS_KEPT`] texts a revert replaced, newest first.
    #[serde(default)]
    pub reverted: Vec<Version>,
}

/// What runs: every prompt a human edited, by key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptBook {
    #[serde(default)]
    prompts: BTreeMap<String, Entry>,
}

/// One change a human proposed, waiting for the factory to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// The text proposed — `None`: back to the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Whether it puts back a text kept earlier (a version, a reverted text,
    /// the default) rather than a new one: what it replaces is then kept
    /// among the reverted.
    #[serde(default)]
    pub revert: bool,
    /// When it was proposed, in seconds since the Unix epoch.
    pub at: u64,
}

/// What waits for the next start: one draft per prompt at most.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drafts {
    #[serde(default)]
    prompts: BTreeMap<String, Draft>,
}

/// The drafts a human deactivated, by prompt, newest first: kept to be
/// reactivated or deleted. Nothing that runs reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shelf {
    #[serde(default)]
    prompts: BTreeMap<String, Vec<Draft>>,
}

/// Where a prompt's dry-run stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialState {
    /// The session is open, or about to be.
    #[default]
    Running,
    /// It answered.
    Done,
    /// It could not run, or the session failed.
    Failed,
}

/// A prompt's dry-run: what it was asked, and what it answered. Written by
/// the trial itself, read by whoever asked for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrialReport {
    /// Where it stands.
    pub state: TrialState,
    /// The prompt tried.
    pub key: String,
    /// Which version of it, as the human named it (`draft`, `edit 2`, …).
    pub version: String,
    /// The line of the run replayed.
    pub workflow: String,
    /// The run replayed.
    pub run: String,
    /// The model it ran on.
    pub model: String,
    /// Its effort.
    pub effort: String,
    /// When it started, Unix seconds.
    pub started_at: u64,
    /// When it ended, Unix seconds.
    pub finished_at: Option<u64>,
    /// Why it failed.
    pub reason: Option<String>,
    /// What the session answered.
    pub output: Option<String>,
    /// What it cost, in dollars — the carrier's estimate.
    pub cost_usd: Option<f64>,
    /// How long the turn took, end to end.
    pub duration_ms: Option<u64>,
    /// How many round-trips with the model it took.
    pub turns: Option<u32>,
    /// The prompt's length, in characters.
    pub prompt_chars: Option<usize>,
}

/// Why a change was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BookError {
    /// A blank prompt: a stage sending nothing is never what a human meant.
    #[error("a prompt cannot be empty — restore the default to go back to it")]
    Empty,
    /// Past [`MAX_CHARS`].
    #[error("a prompt is at most {MAX_CHARS} characters, this one is {0}")]
    TooLong(usize),
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

    /// The keys whose stage sends a human's edit rather than its default.
    pub fn edited(&self) -> impl Iterator<Item = &str> {
        self.prompts
            .iter()
            .filter(|(_, entry)| entry.current.is_some())
            .map(|(key, _)| key.as_str())
    }

    /// Makes every draft what runs, and answers the keys it changed.
    ///
    /// `default_of` names each prompt's default; a draft for a key it does not
    /// know — a prompt removed since — is dropped. A new text goes to the head
    /// of the history; a revert keeps what it replaced among the reverted.
    pub fn apply(
        &mut self,
        drafts: &Drafts,
        default_of: impl Fn(&str) -> Option<String>,
        at: u64,
    ) -> Vec<String> {
        let mut applied = Vec::new();
        for (key, draft) in &drafts.prompts {
            let Some(default) = default_of(key) else {
                continue;
            };
            let replaced = self.text(key, &default);
            let target = draft.text.clone().unwrap_or_else(|| default.clone());
            if target == replaced {
                continue;
            }
            if target == default {
                if let Some(entry) = self.prompts.get_mut(key) {
                    entry.current = None;
                }
            } else {
                let entry = self.prompts.entry(key.clone()).or_default();
                entry.current = Some(target.clone());
                keep(&mut entry.history, &target, at, KEPT);
            }
            if draft.revert {
                let entry = self.prompts.entry(key.clone()).or_default();
                keep(&mut entry.reverted, &replaced, at, REVERTS_KEPT);
            }
            applied.push(key.clone());
        }
        applied
    }
}

/// Puts `text` at the head of `list`, once, and keeps `most` of them.
fn keep(list: &mut Vec<Version>, text: &str, at: u64, most: usize) {
    list.retain(|version| version.text != text);
    list.insert(
        0,
        Version {
            text: text.to_string(),
            saved_at: at,
        },
    );
    list.truncate(most);
}

impl Drafts {
    /// The draft waiting for this prompt.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Draft> {
        self.prompts.get(key)
    }

    /// Every draft, by key.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Draft)> {
        self.prompts
            .iter()
            .map(|(key, draft)| (key.as_str(), draft))
    }

    /// Whether nothing waits.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.prompts.is_empty()
    }

    /// Proposes `text` (`None`: the default) for `key`, replacing its draft.
    ///
    /// A proposal equal to what `book` runs withdraws the draft instead: there
    /// is nothing left to wait for.
    ///
    /// # Errors
    ///
    /// [`BookError::Empty`] for a blank text, [`BookError::TooLong`] past
    /// [`MAX_CHARS`]. The drafts are unchanged.
    pub fn propose(
        &mut self,
        book: &PromptBook,
        key: &str,
        default: &str,
        text: Option<&str>,
        revert: bool,
        at: u64,
    ) -> Result<(), BookError> {
        if let Some(text) = text {
            if text.trim().is_empty() {
                return Err(BookError::Empty);
            }
            let chars = text.chars().count();
            if chars > MAX_CHARS {
                return Err(BookError::TooLong(chars));
            }
        }
        if text.unwrap_or(default) == book.text(key, default) {
            self.prompts.remove(key);
            return Ok(());
        }
        self.prompts.insert(
            key.to_string(),
            Draft {
                text: text.map(str::to_string),
                revert,
                at,
            },
        );
        Ok(())
    }

    /// Withdraws the draft of `key`, if there is one.
    pub fn withdraw(&mut self, key: &str) {
        self.prompts.remove(key);
    }

    /// Deactivates the draft of `key`: it leaves the drafts for the head of
    /// `aside`. Nothing happens when no draft waits.
    pub fn deactivate(&mut self, aside: &mut Shelf, key: &str) {
        if let Some(draft) = self.prompts.remove(key) {
            aside.put(key, draft);
        }
    }

    /// Makes the deactivated draft at `index` of `key` the one that waits
    /// again; the draft it replaces, if any, is deactivated in its turn.
    ///
    /// Answers `false` when nothing is kept at `index`. A text equal to what
    /// `book` runs leaves nothing waiting, as [`Drafts::propose`] does.
    ///
    /// # Errors
    ///
    /// As [`Drafts::propose`]; the shelf and the drafts are then unchanged.
    #[allow(clippy::too_many_arguments)] // every argument is one `propose` needs
    pub fn reactivate(
        &mut self,
        aside: &mut Shelf,
        book: &PromptBook,
        key: &str,
        default: &str,
        index: usize,
        at: u64,
    ) -> Result<bool, BookError> {
        let Some(draft) = aside.remove(key, index) else {
            return Ok(false);
        };
        let mut next = self.clone();
        next.prompts.remove(key);
        if let Err(why) = next.propose(book, key, default, draft.text.as_deref(), draft.revert, at)
        {
            aside.insert(key, index, draft);
            return Err(why);
        }
        self.deactivate(aside, key);
        *self = next;
        Ok(true)
    }
}

impl Shelf {
    /// The deactivated drafts of `key`, newest first.
    #[must_use]
    pub fn of(&self, key: &str) -> &[Draft] {
        self.prompts.get(key).map_or(&[], Vec::as_slice)
    }

    /// Whether nothing is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.prompts.is_empty()
    }

    /// Keeps `draft` at the head of `key`'s list, once per text, and the
    /// last [`SHELVED_KEPT`] of them.
    pub fn put(&mut self, key: &str, draft: Draft) {
        let list = self.prompts.entry(key.to_string()).or_default();
        list.retain(|kept| kept.text != draft.text);
        list.insert(0, draft);
        list.truncate(SHELVED_KEPT);
    }

    /// Deletes the deactivated draft at `index` of `key` and answers it;
    /// `None` when nothing is kept there.
    pub fn remove(&mut self, key: &str, index: usize) -> Option<Draft> {
        let list = self.prompts.get_mut(key)?;
        if index >= list.len() {
            return None;
        }
        let draft = list.remove(index);
        if list.is_empty() {
            self.prompts.remove(key);
        }
        Some(draft)
    }

    /// Puts `draft` back at `index` of `key`'s list — the undo of
    /// [`Shelf::remove`].
    fn insert(&mut self, key: &str, index: usize, draft: Draft) {
        let list = self.prompts.entry(key.to_string()).or_default();
        list.insert(index.min(list.len()), draft);
    }
}

/// One piece of a template: prose, or a `{name}` the stage fills in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece<'a> {
    Literal(&'a str),
    Placeholder(&'a str),
}

/// A template cut into prose and placeholders, the way
/// [`splice`](crate::domain::prompts::splice) reads it: a name is what lies
/// between a `{` and the next `}`, kept only when it looks like an
/// identifier, since prompts carry braces that are not placeholders (a JSON
/// example, a `{ }` in prose).
fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut literal_from = 0;
    let mut at = 0;
    while let Some(open) = text[at..].find('{').map(|i| at + i) {
        let Some(close) = text[open + 1..].find('}').map(|i| open + 1 + i) else {
            break;
        };
        let name = &text[open + 1..close];
        if is_identifier(name) {
            if open > literal_from {
                out.push(Piece::Literal(&text[literal_from..open]));
            }
            out.push(Piece::Placeholder(name));
            literal_from = close + 1;
            at = close + 1;
        } else {
            at = open + 1;
        }
    }
    if literal_from < text.len() {
        out.push(Piece::Literal(&text[literal_from..]));
    }
    out
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The `{name}` placeholders a prompt carries — what the stage fills in.
#[must_use]
pub fn placeholders(text: &str) -> BTreeSet<String> {
    pieces(text)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Placeholder(name) => Some(name.to_string()),
            Piece::Literal(_) => None,
        })
        .collect()
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

/// Why a past prompt could not be replayed with another text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReplayError {
    /// The text the stage sent then is not in the prompt.
    #[error(
        "the prompt sent then does not carry this text — the run used another version, or none"
    )]
    NotFound,
    /// The text sent then starts or ends with a placeholder, or carries two in
    /// a row: where one value ends is unknowable.
    #[error(
        "the text sent then has a placeholder at an edge or two in a row — its values cannot be told apart"
    )]
    Ambiguous,
}

/// The prompt `sent` once carried, with `new` in place of `old`, filled with
/// the values `old` was filled with then.
///
/// `old` is the template the stage used, `sent` the whole prompt it sent —
/// the framework's blocks around it included. The values are read back from
/// between `old`'s prose pieces; `new` is filled with them (a placeholder it
/// drops loses its value, one it invents stays as written), and the blocks
/// around are kept as they were.
///
/// # Errors
///
/// [`ReplayError::NotFound`] when `sent` does not carry `old`,
/// [`ReplayError::Ambiguous`] when `old`'s shape hides where a value ends.
pub fn replay(sent: &str, old: &str, new: &str) -> Result<String, ReplayError> {
    let cut = pieces(old);
    let edges_are_prose = matches!(cut.first(), Some(Piece::Literal(_)))
        && matches!(cut.last(), Some(Piece::Literal(_)));
    let in_a_row = cut
        .windows(2)
        .any(|pair| matches!(pair, [Piece::Placeholder(_), Piece::Placeholder(_)]));
    if !edges_are_prose || in_a_row {
        return Err(ReplayError::Ambiguous);
    }
    let Some(Piece::Literal(head)) = cut.first() else {
        return Err(ReplayError::Ambiguous);
    };
    for (start, _) in sent.match_indices(head) {
        if let Some((end, values)) = read_values(sent, start + head.len(), &cut[1..]) {
            let pairs: Vec<(&str, &str)> = values
                .iter()
                .map(|(name, value)| (*name, value.as_str()))
                .collect();
            return Ok(format!(
                "{}{}{}",
                &sent[..start],
                crate::domain::prompts::splice(new, &pairs),
                &sent[end..]
            ));
        }
    }
    Err(ReplayError::NotFound)
}

/// Walks `rest` of a template from `at` in `sent`: each placeholder's value
/// runs to the next prose piece. Answers where the template ends, and the
/// values — `None` when a piece is missing or a name got two values.
fn read_values<'a>(
    sent: &str,
    mut at: usize,
    rest: &[Piece<'a>],
) -> Option<(usize, BTreeMap<&'a str, String>)> {
    let mut values: BTreeMap<&str, String> = BTreeMap::new();
    let mut pending: Option<&str> = None;
    for piece in rest {
        match piece {
            Piece::Placeholder(name) => pending = Some(name),
            Piece::Literal(text) => {
                let found = at + sent.get(at..)?.find(text)?;
                if let Some(name) = pending.take() {
                    let value = sent[at..found].to_string();
                    if values.get(name).is_some_and(|seen| *seen != value) {
                        return None;
                    }
                    values.insert(name, value);
                } else if found != at {
                    return None;
                }
                at = found + text.len();
            }
        }
    }
    Some((at, values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const DEFAULT: &str = "Issue #{num} — {title}.";

    // The shape `PromptBook::apply` asks for, though every key is known here.
    #[allow(clippy::unnecessary_wraps)]
    fn default_of(_: &str) -> Option<String> {
        Some(DEFAULT.to_string())
    }

    fn applied(book: &mut PromptBook, key: &str, text: Option<&str>, revert: bool, at: u64) {
        let mut drafts = Drafts::default();
        drafts
            .propose(book, key, DEFAULT, text, revert, at)
            .expect("proposed");
        book.apply(&drafts, default_of, at);
    }

    fn draft(text: &str, at: u64) -> Draft {
        Draft {
            text: Some(text.to_string()),
            revert: false,
            at,
        }
    }

    #[test]
    fn a_deactivated_draft_waits_on_the_shelf_not_in_the_drafts() {
        let (book, mut drafts, mut shelf) =
            (PromptBook::default(), Drafts::default(), Shelf::default());
        drafts
            .propose(&book, "k", DEFAULT, Some("mine"), false, 1)
            .expect("proposed");
        drafts.deactivate(&mut shelf, "k");
        assert!(drafts.is_empty(), "nothing waits for the start any more");
        assert_eq!(shelf.of("k"), [draft("mine", 1)]);
        // Nothing to deactivate is no change.
        drafts.deactivate(&mut shelf, "k");
        assert_eq!(shelf.of("k").len(), 1);
    }

    #[test]
    fn reactivating_swaps_the_waiting_draft_onto_the_shelf() {
        let (book, mut drafts, mut shelf) =
            (PromptBook::default(), Drafts::default(), Shelf::default());
        shelf.put("k", draft("old", 1));
        drafts
            .propose(&book, "k", DEFAULT, Some("new"), false, 2)
            .expect("proposed");
        assert!(
            drafts
                .reactivate(&mut shelf, &book, "k", DEFAULT, 0, 3)
                .expect("reactivated")
        );
        assert_eq!(drafts.get("k").and_then(|d| d.text.as_deref()), Some("old"));
        assert_eq!(shelf.of("k"), [draft("new", 2)]);
        assert!(
            !drafts
                .reactivate(&mut shelf, &book, "k", DEFAULT, 5, 4)
                .expect("an answer"),
            "nothing kept at 5"
        );
    }

    #[test]
    fn a_refused_reactivation_changes_nothing() {
        let (book, mut drafts, mut shelf) =
            (PromptBook::default(), Drafts::default(), Shelf::default());
        shelf.put("k", draft("   ", 1));
        shelf.put("k", draft("kept", 2));
        drafts
            .propose(&book, "k", DEFAULT, Some("waits"), false, 3)
            .expect("proposed");
        let (before_drafts, before_shelf) = (drafts.clone(), shelf.clone());
        assert_eq!(
            drafts.reactivate(&mut shelf, &book, "k", DEFAULT, 1, 4),
            Err(BookError::Empty)
        );
        assert_eq!((drafts, shelf), (before_drafts, before_shelf));
    }

    #[test]
    fn the_shelf_keeps_each_text_once_and_the_last_ten() {
        let mut shelf = Shelf::default();
        for at in 0..12 {
            shelf.put("k", draft(&format!("t{at}"), at));
        }
        shelf.put("k", draft("t11", 20));
        let texts: Vec<&str> = shelf
            .of("k")
            .iter()
            .filter_map(|d| d.text.as_deref())
            .collect();
        assert_eq!(texts.len(), SHELVED_KEPT);
        assert_eq!(texts[0], "t11");
        assert_eq!(texts.iter().filter(|t| **t == "t11").count(), 1);
        assert_eq!(shelf.remove("k", 0).map(|d| d.at), Some(20));
        assert!(shelf.remove("k", 99).is_none());
        assert!(shelf.remove("other", 0).is_none());
    }

    #[test]
    fn an_untouched_prompt_is_its_default() {
        let book = PromptBook::default();
        assert_eq!(book.text("a/b", DEFAULT), DEFAULT);
    }

    #[test]
    fn a_draft_runs_nothing_until_applied() {
        let book = PromptBook::default();
        let mut drafts = Drafts::default();
        drafts
            .propose(&book, "k", DEFAULT, Some("Fix #{num}."), false, 3)
            .expect("proposed");
        assert_eq!(book.text("k", DEFAULT), DEFAULT);
        let mut book = book;
        assert_eq!(book.apply(&drafts, default_of, 4), ["k"]);
        assert_eq!(book.text("k", DEFAULT), "Fix #{num}.");
        assert_eq!(book.entry("k").expect("kept").history[0].saved_at, 4);
    }

    #[test]
    fn proposing_what_runs_withdraws_the_draft() {
        let mut book = PromptBook::default();
        applied(&mut book, "k", Some("mine"), false, 1);
        let mut drafts = Drafts::default();
        drafts
            .propose(&book, "k", DEFAULT, Some("other"), false, 2)
            .expect("proposed");
        drafts
            .propose(&book, "k", DEFAULT, Some("mine"), false, 3)
            .expect("proposed");
        assert!(drafts.is_empty());
    }

    #[test]
    fn only_the_last_five_edits_are_kept_newest_first() {
        let mut book = PromptBook::default();
        for n in 0..8 {
            applied(&mut book, "k", Some(&format!("v{n}")), false, n);
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
    fn a_revert_keeps_what_it_replaced_and_only_the_last_three() {
        let mut book = PromptBook::default();
        for n in 0..5 {
            applied(&mut book, "k", Some(&format!("v{n}")), false, n);
            applied(&mut book, "k", None, true, n);
        }
        let entry = book.entry("k").expect("kept");
        assert_eq!(entry.current, None);
        let reverted: Vec<&str> = entry.reverted.iter().map(|v| v.text.as_str()).collect();
        assert_eq!(reverted, ["v4", "v3", "v2"]);
    }

    #[test]
    fn a_new_text_is_not_a_revert() {
        let mut book = PromptBook::default();
        applied(&mut book, "k", Some("a"), false, 1);
        applied(&mut book, "k", Some("b"), false, 2);
        assert!(book.entry("k").expect("kept").reverted.is_empty());
    }

    #[test]
    fn a_draft_for_a_prompt_gone_since_is_dropped() {
        let mut book = PromptBook::default();
        let mut drafts = Drafts::default();
        drafts
            .propose(&book, "gone", DEFAULT, Some("x"), false, 1)
            .expect("proposed");
        assert!(book.apply(&drafts, |_| None, 2).is_empty());
        assert_eq!(book, PromptBook::default());
    }

    #[test]
    fn a_blank_or_huge_prompt_is_refused_and_changes_nothing() {
        let book = PromptBook::default();
        let mut drafts = Drafts::default();
        assert_eq!(
            drafts.propose(&book, "k", DEFAULT, Some("  \n"), false, 1),
            Err(BookError::Empty)
        );
        let huge = "x".repeat(MAX_CHARS + 1);
        assert_eq!(
            drafts.propose(&book, "k", DEFAULT, Some(&huge), false, 1),
            Err(BookError::TooLong(MAX_CHARS + 1))
        );
        assert!(drafts.is_empty());
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

    #[test]
    fn a_replay_swaps_the_stage_s_text_and_keeps_the_blocks_around() {
        let sent = "/code\n--- CONTEXT ---\nIssue #42 — Add a login. Then {x}.\n--- SCOPE ---";
        let old = "Issue #{num} — {title}. Then {x}.";
        let new = "Fix #{num} ({title}), {nope}.";
        assert_eq!(
            replay(sent, old, new),
            Ok("/code\n--- CONTEXT ---\nFix #42 (Add a login), {nope}.\n--- SCOPE ---".to_string())
        );
    }

    #[test]
    fn a_replay_refuses_a_prompt_that_never_carried_the_text() {
        assert_eq!(
            replay("something else", DEFAULT, "x"),
            Err(ReplayError::NotFound)
        );
        assert_eq!(
            replay("Issue #1 — t.", "{num} — {title}.", "x"),
            Err(ReplayError::Ambiguous)
        );
    }

    proptest! {
        #[test]
        fn edits_keep_the_history_short_unique_and_headed_by_what_runs(
            texts in proptest::collection::vec("[a-c]{1,2}", 1..20),
        ) {
            let mut book = PromptBook::default();
            for (at, text) in texts.iter().enumerate() {
                applied(&mut book, "k", Some(text), false, at as u64);
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
        fn a_book_and_its_drafts_survive_their_own_json(
            texts in proptest::collection::vec("\\PC{1,40}", 1..8),
        ) {
            let mut book = PromptBook::default();
            let mut drafts = Drafts::default();
            for (at, text) in texts.iter().enumerate() {
                let key = format!("k{}", at % 3);
                applied(&mut book, &key, Some(text), at % 2 == 0, at as u64);
                let _ = drafts.propose(&book, &key, DEFAULT, Some("draft"), true, at as u64);
            }
            let json = serde_json::to_string(&book).expect("encoded");
            prop_assert_eq!(serde_json::from_str::<PromptBook>(&json).expect("decoded"), book);
            let json = serde_json::to_string(&drafts).expect("encoded");
            prop_assert_eq!(serde_json::from_str::<Drafts>(&json).expect("decoded"), drafts);
        }

        #[test]
        fn a_replay_of_the_same_text_gives_the_prompt_back(
            num in "[0-9]{1,4}",
            title in "[0-9 ]{0,12}",
            before in "[x-z\n]{0,10}",
            after in "[x-z\n]{0,10}",
        ) {
            // Templates, not format strings: the braces are the placeholders
            // under test.
            const OLD: &str = "Issue #{num} — {title}.";
            const NEW: &str = "N={num} T={title}";
            let old = OLD;
            let filled = crate::domain::prompts::splice(old, &[("num", &num), ("title", &title)]);
            let sent = format!("{before}{filled}{after}");
            prop_assert_eq!(replay(&sent, old, old), Ok(sent.clone()));
            let swapped = replay(&sent, old, NEW).expect("replayed");
            prop_assert_eq!(swapped, format!("{before}N={num} T={title}{after}"));
        }
    }
}
