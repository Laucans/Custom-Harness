//! A station's prompts as the pane shows them, and the changes the human
//! leaves waiting for the factory's next start.
//!
//! The view writes no prompt a run reads. What runs is the book, which only
//! the harness writes, when the watch starts; the view reads it, and writes
//! drafts beside it. What a prompt is — its key, its stations, its default —
//! is `harness-workflows`' catalog; what a draft, an edit and a revert do is
//! `harness-core`'s `prompt_book`. This module puts the three together for
//! the page.

use harness_core::domain::prompt_book::{self, BookError, Draft, Drafts, PromptBook, Version};
use harness_workflows::prompts::{self as catalog, Prompt};
use serde::{Deserialize, Serialize};

/// One text kept, as the pane lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Kept {
    /// The text.
    pub text: String,
    /// When it was kept, Unix seconds.
    pub saved_at: u64,
    /// Whether it is what the stage sends now.
    pub running: bool,
    /// Whether it is the default's text.
    pub is_default: bool,
}

/// A change waiting for the next start, as the pane shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Waiting {
    /// The text that will run — the default's, when the change goes back to it.
    pub text: String,
    /// Whether it goes back to the default.
    pub to_default: bool,
    /// Whether it puts back a text kept earlier.
    pub revert: bool,
    /// When it was proposed, Unix seconds.
    pub at: u64,
}

/// One prompt, as the pane shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptView {
    /// What the edits are kept under.
    pub key: String,
    /// What a human calls it.
    pub title: String,
    /// Every station that sends it, as `line/station` — more than one when
    /// a change here changes another station too.
    pub stations: Vec<String>,
    /// What the stage sends now.
    pub running: String,
    /// The workflow's own text.
    pub default: String,
    /// Whether what runs is a human's edit rather than the default.
    pub edited: bool,
    /// The change waiting for the next start, if any.
    pub draft: Option<Waiting>,
    /// The last edits, newest first.
    pub history: Vec<Kept>,
    /// The last texts a revert replaced, newest first.
    pub reverted: Vec<Kept>,
    /// The `{name}` placeholders the stage fills in.
    pub placeholders: Vec<String>,
    /// Placeholders of the default the next text dropped.
    pub missing: Vec<String>,
    /// Placeholders in the next text that nothing fills.
    pub unknown: Vec<String>,
}

/// One change waiting, as the wrench's pane lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pending {
    /// The prompt's key.
    pub key: String,
    /// What a human calls it.
    pub title: String,
    /// The stations it changes: `(line, station)`.
    pub stations: Vec<(String, String)>,
    /// What kind of change: `edit`, `revert` or `default`.
    pub kind: &'static str,
    /// When it was proposed, Unix seconds.
    pub at: u64,
}

/// What the page sends to propose a new text.
#[derive(Debug, Deserialize)]
pub struct Edit {
    /// The prompt's key.
    pub key: String,
    /// The new text.
    pub text: String,
}

/// Where a text put back comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum From {
    /// The workflow's own text.
    Default,
    /// The edits kept.
    History,
    /// The texts a revert replaced.
    Reverted,
}

/// What the page sends to put a text back.
#[derive(Debug, Deserialize)]
pub struct Restore {
    /// The prompt's key.
    pub key: String,
    /// Which list.
    pub from: From,
    /// The index in that list, newest first; unused for the default.
    #[serde(default)]
    pub index: usize,
}

/// What the page sends to withdraw a draft.
#[derive(Debug, Deserialize)]
pub struct Withdraw {
    /// The prompt's key.
    pub key: String,
}

/// Why a change was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// No prompt by that key.
    Unknown(String),
    /// Nothing at that place in that list.
    NoSuchVersion(usize),
    /// The drafts refused it.
    Book(BookError),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(key) => write!(f, "no prompt named {key}"),
            Self::NoSuchVersion(index) => write!(f, "nothing kept at place {index}"),
            Self::Book(why) => write!(f, "{why}"),
        }
    }
}

fn kept(list: &[Version], running: &str, default: &str) -> Vec<Kept> {
    list.iter()
        .map(|version| Kept {
            text: version.text.clone(),
            saved_at: version.saved_at,
            running: version.text == running,
            is_default: version.text == default,
        })
        .collect()
}

/// One prompt of the catalog, with what runs and what waits.
#[must_use]
pub fn view(book: &PromptBook, drafts: &Drafts, prompt: &Prompt) -> PromptView {
    let running = book.text(prompt.key, prompt.default);
    let entry = book.entry(prompt.key);
    let draft = drafts.get(prompt.key).map(|draft| Waiting {
        text: draft
            .text
            .clone()
            .unwrap_or_else(|| prompt.default.to_string()),
        to_default: draft.text.is_none(),
        revert: draft.revert,
        at: draft.at,
    });
    let next = draft
        .as_ref()
        .map_or(running.as_str(), |draft| draft.text.as_str());
    let (missing, unknown) = prompt_book::drift(prompt.default, next);
    PromptView {
        key: prompt.key.to_string(),
        title: prompt.title.to_string(),
        stations: prompt
            .stations
            .iter()
            .map(|(line, station)| format!("{line}/{station}"))
            .collect(),
        default: prompt.default.to_string(),
        edited: running != prompt.default,
        history: entry.map_or_else(Vec::new, |entry| {
            kept(&entry.history, &running, prompt.default)
        }),
        reverted: entry.map_or_else(Vec::new, |entry| {
            kept(&entry.reverted, &running, prompt.default)
        }),
        placeholders: prompt_book::placeholders(prompt.default)
            .into_iter()
            .collect(),
        missing,
        unknown,
        draft,
        running,
    }
}

/// The prompts one station sends. Empty for a station that sends none (a
/// gate, a free step).
#[must_use]
pub fn of_station(
    book: &PromptBook,
    drafts: &Drafts,
    line: &str,
    station: &str,
) -> Vec<PromptView> {
    catalog::of_station(line, station)
        .iter()
        .map(|prompt| view(book, drafts, prompt))
        .collect()
}

/// Every change waiting for the next start, newest first.
#[must_use]
pub fn pending(drafts: &Drafts) -> Vec<Pending> {
    let mut list: Vec<Pending> = drafts
        .iter()
        .filter_map(|(key, draft)| {
            let prompt = catalog::find(key)?;
            Some(Pending {
                key: key.to_string(),
                title: prompt.title.to_string(),
                stations: prompt
                    .stations
                    .iter()
                    .map(|(line, station)| ((*line).to_string(), (*station).to_string()))
                    .collect(),
                kind: kind_of(draft),
                at: draft.at,
            })
        })
        .collect();
    list.sort_by_key(|change| std::cmp::Reverse(change.at));
    list
}

const fn kind_of(draft: &Draft) -> &'static str {
    match (draft.text.is_none(), draft.revert) {
        (true, _) => "default",
        (false, true) => "revert",
        (false, false) => "edit",
    }
}

fn known(key: &str) -> Result<Prompt, Refused> {
    catalog::find(key).ok_or_else(|| Refused::Unknown(key.to_string()))
}

/// Proposes a new text for the next start.
///
/// # Errors
///
/// [`Refused::Unknown`] for a key the catalog does not hold,
/// [`Refused::Book`] for a text the drafts refuse. Nothing changes then.
pub fn propose(
    book: &PromptBook,
    drafts: &mut Drafts,
    edit: &Edit,
    now: u64,
) -> Result<PromptView, Refused> {
    let prompt = known(&edit.key)?;
    drafts
        .propose(
            book,
            prompt.key,
            prompt.default,
            Some(&edit.text),
            false,
            now,
        )
        .map_err(Refused::Book)?;
    Ok(view(book, drafts, &prompt))
}

/// Proposes, for the next start, a text kept earlier — a revert.
///
/// # Errors
///
/// As [`propose`], and [`Refused::NoSuchVersion`] past the list.
pub fn restore(
    book: &PromptBook,
    drafts: &mut Drafts,
    restore: &Restore,
    now: u64,
) -> Result<PromptView, Refused> {
    let prompt = known(&restore.key)?;
    let entry = book.entry(prompt.key);
    let list = |pick: fn(&prompt_book::Entry) -> &Vec<Version>| {
        entry
            .and_then(|entry| pick(entry).get(restore.index))
            .map(|version| version.text.clone())
            .ok_or(Refused::NoSuchVersion(restore.index))
    };
    let text = match restore.from {
        From::Default => None,
        From::History => Some(list(|entry| &entry.history)?),
        From::Reverted => Some(list(|entry| &entry.reverted)?),
    };
    let text = text.filter(|text| text != prompt.default);
    drafts
        .propose(book, prompt.key, prompt.default, text.as_deref(), true, now)
        .map_err(Refused::Book)?;
    Ok(view(book, drafts, &prompt))
}

/// Withdraws a draft: the next start keeps what runs.
///
/// # Errors
///
/// [`Refused::Unknown`] for a key the catalog does not hold.
pub fn withdraw(
    book: &PromptBook,
    drafts: &mut Drafts,
    withdraw: &Withdraw,
) -> Result<PromptView, Refused> {
    let prompt = known(&withdraw.key)?;
    drafts.withdraw(prompt.key);
    Ok(view(book, drafts, &prompt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::blueprint::{self, Kind};

    fn started(book: &mut PromptBook, drafts: &mut Drafts, at: u64) {
        book.apply(drafts, catalog::default_of, at);
        *drafts = Drafts::default();
    }

    #[test]
    fn every_robot_shows_a_prompt_and_every_prompt_stands_on_a_station() {
        let (book, drafts) = (PromptBook::default(), Drafts::default());
        let lines = blueprint::lines();
        for line in &lines {
            for station in &line.stations {
                let paid = matches!(station.kind, Kind::Builder | Kind::Inspector);
                assert_eq!(
                    paid,
                    !of_station(&book, &drafts, &line.id, &station.id).is_empty(),
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
    fn an_edit_waits_for_the_start_and_then_runs() {
        let (mut book, mut drafts) = (PromptBook::default(), Drafts::default());
        let edit = Edit {
            key: catalog::FIX.to_string(),
            text: "Repair PR #{num} {nope}".to_string(),
        };
        let waiting = propose(&book, &mut drafts, &edit, 5).expect("proposed");
        assert!(!waiting.edited, "nothing runs it yet");
        assert_eq!(
            waiting.draft.as_ref().map(|d| d.text.as_str()),
            Some("Repair PR #{num} {nope}")
        );
        assert!(waiting.missing.contains(&"title".to_string()));
        assert_eq!(waiting.unknown, ["nope"]);
        assert_eq!(pending(&drafts)[0].kind, "edit");

        started(&mut book, &mut drafts, 6);
        let now = view(&book, &drafts, &catalog::find(catalog::FIX).expect("known"));
        assert!(now.edited && now.draft.is_none());
        assert!(now.history[0].running);
    }

    #[test]
    fn a_revert_keeps_what_it_replaced_once_the_factory_starts() {
        let (mut book, mut drafts) = (PromptBook::default(), Drafts::default());
        let key = catalog::FIX.to_string();
        propose(
            &book,
            &mut drafts,
            &Edit {
                key: key.clone(),
                text: "mine".to_string(),
            },
            1,
        )
        .expect("proposed");
        started(&mut book, &mut drafts, 2);
        let back = Restore {
            key: key.clone(),
            from: From::Default,
            index: 0,
        };
        let waiting = restore(&book, &mut drafts, &back, 3).expect("restored");
        assert!(
            waiting
                .draft
                .as_ref()
                .is_some_and(|d| d.to_default && d.revert)
        );
        assert_eq!(pending(&drafts)[0].kind, "default");
        started(&mut book, &mut drafts, 4);
        let now = view(&book, &drafts, &catalog::find(&key).expect("known"));
        assert!(!now.edited);
        assert_eq!(now.reverted[0].text, "mine");

        let again = Restore {
            key,
            from: From::Reverted,
            index: 0,
        };
        let waiting = restore(&book, &mut drafts, &again, 5).expect("restored");
        assert_eq!(waiting.draft.map(|d| d.text), Some("mine".to_string()));
    }

    #[test]
    fn withdrawing_leaves_what_runs_and_an_unknown_key_changes_nothing() {
        let (book, mut drafts) = (PromptBook::default(), Drafts::default());
        let key = catalog::FIX.to_string();
        propose(
            &book,
            &mut drafts,
            &Edit {
                key: key.clone(),
                text: "x".to_string(),
            },
            1,
        )
        .expect("proposed");
        withdraw(&book, &mut drafts, &Withdraw { key }).expect("withdrawn");
        assert!(drafts.is_empty());
        let refused = propose(
            &book,
            &mut drafts,
            &Edit {
                key: "../etc".to_string(),
                text: "x".to_string(),
            },
            1,
        );
        assert_eq!(refused, Err(Refused::Unknown("../etc".to_string())));
        let far = Restore {
            key: catalog::FIX.to_string(),
            from: From::History,
            index: 2,
        };
        assert_eq!(
            restore(&book, &mut drafts, &far, 1),
            Err(Refused::NoSuchVersion(2))
        );
        assert!(drafts.is_empty());
    }
}
