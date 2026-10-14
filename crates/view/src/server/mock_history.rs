//! The mock-up's history over HTTP (`domain::mock_history`): what a change
//! of the mock-up or of its rules records — when the page says which
//! branch's mock-up it is at (`x-harness-branch`), and which gesture
//! (`x-harness-step`, `x-harness-what`) — then Cmd+Z and its redo.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::domain::annotations::Note;
use crate::domain::drafts::{self, Draft};
use crate::domain::mock_history::{self, Entry, Facet};
use crate::domain::steps::Step;
use crate::ports::Mockups;

use super::notes::stamp;
use super::{AppState, json};

/// The branch of the mock-up a request changes, as the page says it.
const BRANCH: &str = "x-harness-branch";
/// The gesture a request belongs to: one entry for all its requests.
const STEP: &str = "x-harness-step";
/// The gesture, in words.
const WHAT: &str = "x-harness-what";

/// Where a change goes in the history, and as what.
#[derive(Debug, Clone, Default)]
pub(super) struct Gesture {
    /// The branch whose history it goes in; empty, it goes in none.
    pub branch: String,
    token: String,
    what: String,
}

impl Gesture {
    /// A gesture the server makes itself, on a branch's history.
    pub fn new(branch: &str, what: &str) -> Self {
        Self {
            branch: branch.to_string(),
            token: String::new(),
            what: what.to_string(),
        }
    }

    /// The gesture the page says a request belongs to; `what` when it says
    /// not what it was.
    pub fn of(headers: &HeaderMap, what: &str) -> Self {
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.chars().take(200).collect::<String>())
                .unwrap_or_default()
        };
        let said = header(WHAT);
        Self {
            branch: header(BRANCH),
            token: header(STEP),
            what: if said.is_empty() {
                what.to_string()
            } else {
                said
            },
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// Keeps what a change did in its branch's history. A history that cannot
/// be written never fails the change: it only forgets it.
pub(super) fn record(state: &AppState, gesture: &Gesture, facets: Vec<Facet>) {
    if gesture.branch.is_empty() || facets.is_empty() {
        return;
    }
    let Ok(kept) = state.history.entries(&gesture.branch) else {
        return;
    };
    let changes = mock_history::record(
        &kept,
        &Entry {
            token: gesture.token.clone(),
            what: gesture.what.clone(),
            at: now_ms(),
            facets,
            ..Entry::default()
        },
    );
    for id in changes.remove {
        let _ = state.history.remove(&gesture.branch, id);
    }
    for entry in changes.put {
        let _ = state.history.put(&gesture.branch, &entry);
    }
}

/// The files of the mock-up as they are: each path with what it holds.
///
/// # Errors
///
/// The repository could not be read.
pub(super) async fn files(
    mockups: Arc<dyn Mockups>,
    branch: String,
    paths: Vec<String>,
) -> Result<Vec<(String, Option<String>)>, String> {
    tokio::task::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|path| {
                let text = mockups
                    .file(&branch, &path)?
                    .map(|b: Vec<u8>| String::from_utf8_lossy(&b).into_owned());
                Ok((path, text))
            })
            .collect()
    })
    .await
    .map_err(|e| format!("the read was dropped: {e}"))?
}

/// The files a change wrote, as facets: each as it was and as it became.
pub(super) fn file_facets(
    before: &[(String, Option<String>)],
    after: &[(String, Option<String>)],
) -> Vec<Facet> {
    before
        .iter()
        .map(|(path, was)| Facet::File {
            path: path.clone(),
            before: was.clone(),
            after: after
                .iter()
                .find(|(p, _)| p == path)
                .and_then(|(_, now)| now.clone()),
        })
        .collect()
}

/// A pin as its board keeps it now; `None`, it is not on it. Its turns
/// are the store's count, not the pin's: left out of what is compared.
pub(super) fn pin_now(state: &AppState, board: &str, id: &str) -> Option<Note> {
    state
        .pins
        .notes(board)
        .ok()?
        .into_iter()
        .find(|k| k.note.id == id && k.deleted_at.is_none())
        .map(|k| Note { turns: 0, ..k.note })
}

/// A pin's change, as a facet.
pub(super) fn pin_facet(board: &str, id: &str, before: Option<Note>, after: Option<Note>) -> Facet {
    Facet::Pin {
        board: board.to_string(),
        id: id.to_string(),
        before: before.map(Box::new),
        after: after.map(|n| Box::new(Note { turns: 0, ..n })),
    }
}

/// A page's draft version open now; `None`, none is.
pub(super) fn draft_now(state: &AppState, board: &str) -> Option<Draft> {
    state
        .drafts
        .versions(board)
        .ok()?
        .into_iter()
        .rev()
        .find(|d| d.state == drafts::State::Open)
}

/// The request pins' steps of a branch now.
pub(super) fn steps_now(state: &AppState, branch: &str) -> Vec<Step> {
    state.steps.steps(branch).unwrap_or_default()
}

/// The rules now, as a facet's side; `None`, no creation.
pub(super) async fn rules_now(state: &AppState) -> Option<crate::domain::creation::RulesState> {
    match &state.creation {
        Some(desk) => desk.rules().await,
        None => None,
    }
}

/// Which history, as the page asks.
#[derive(Debug, Default, Deserialize)]
pub(super) struct BranchQuery {
    #[serde(default)]
    branch: String,
}

/// An entry as the page is told of it: enough to say it and to know where
/// it happened.
fn told(entry: Option<&Entry>) -> serde_json::Value {
    entry.map_or(serde_json::Value::Null, |e| {
        serde_json::json!({
            "id": e.id,
            "what": e.what,
            "boards": e.boards(),
            "rules": e.touches_rules(),
        })
    })
}

/// What Cmd+Z and its redo would do next on a branch's mock-up.
pub(super) async fn history(
    State(state): State<AppState>,
    Query(query): Query<BranchQuery>,
) -> Response {
    let kept = match state.history.entries(&query.branch) {
        Ok(kept) => kept,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    json(&serde_json::json!({
        "undo": told(mock_history::next_undo(&kept)),
        "redo": told(mock_history::next_redo(&kept)),
    }))
}

/// Takes back the last entry of a branch's history.
pub(super) async fn undo(State(state): State<AppState>, body: Bytes) -> Response {
    step(state, body, true).await
}

/// Brings back the first entry taken back.
pub(super) async fn redo(State(state): State<AppState>, body: Bytes) -> Response {
    step(state, body, false).await
}

async fn step(state: AppState, body: Bytes, back: bool) -> Response {
    let ask: BranchQuery = serde_json::from_slice(&body).unwrap_or_default();
    let kept = match state.history.entries(&ask.branch) {
        Ok(kept) => kept,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let next = if back {
        mock_history::next_undo(&kept)
    } else {
        mock_history::next_redo(&kept)
    };
    let Some(entry) = next.cloned() else {
        return (
            StatusCode::CONFLICT,
            if back {
                "nothing to undo"
            } else {
                "nothing to redo"
            },
        )
            .into_response();
    };
    // An undo writes the befores, latest facet first; a redo the afters.
    let facets: Vec<Facet> = if back {
        entry
            .facets
            .iter()
            .rev()
            .cloned()
            .map(Facet::reversed)
            .collect()
    } else {
        entry.facets.clone()
    };
    if let Err(why) = still(&state, &ask.branch, &facets).await {
        return (StatusCode::CONFLICT, why).into_response();
    }
    if let Err(why) = write(&state, &ask.branch, facets).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
    }
    let mut moved = entry.clone();
    moved.undone = back;
    if let Err(why) = state.history.put(&ask.branch, &moved) {
        return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
    }
    json(&serde_json::json!({ "done": told(Some(&entry)) }))
}

/// Everything a step would write over is still as the entry left it:
/// else something outside the history moved it, and the step would undo
/// that too.
async fn still(state: &AppState, branch: &str, facets: &[Facet]) -> Result<(), String> {
    for f in facets {
        let same = match f {
            Facet::File { path, before, .. } => {
                let Some(mockups) = state.mockups.clone() else {
                    return Err("nothing reads the mock-up".to_string());
                };
                files(mockups, branch.to_string(), vec![path.clone()])
                    .await?
                    .first()
                    .map(|(_, now)| now.clone())
                    == Some(before.clone())
            }
            Facet::Pin {
                board, id, before, ..
            } => pin_now(state, board, id).as_ref() == before.as_deref(),
            Facet::Rules { before, .. } => rules_now(state).await.as_ref() == Some(before.as_ref()),
            Facet::Draft { board, before, .. } => draft_now(state, board).map(Box::new) == *before,
            Facet::Steps { before, .. } => steps_now(state, branch) == *before,
        };
        if !same {
            return Err(format!(
                "{} moved since, outside the history: it is left as it is",
                f.about()
            ));
        }
    }
    Ok(())
}

/// Writes each facet's after.
async fn write(state: &AppState, branch: &str, facets: Vec<Facet>) -> Result<(), String> {
    for f in facets {
        match f {
            Facet::File { path, after, .. } => {
                let mockups = state.mockups.clone().ok_or("nothing reads the mock-up")?;
                let at = branch.to_string();
                tokio::task::spawn_blocking(move || {
                    after.map_or_else(
                        || mockups.remove(&at, &path),
                        |text| mockups.write(&at, &path, text.as_bytes()),
                    )
                })
                .await
                .map_err(|e| format!("the write was dropped: {e}"))??;
            }
            Facet::Pin {
                board, id, after, ..
            } => match after {
                Some(note) => {
                    state.pins.put(&board, &note)?;
                    state.pins.mark_deleted(&board, &id, None)?;
                }
                None => state.pins.mark_deleted(&board, &id, Some(&stamp()))?,
            },
            Facet::Rules { after, .. } => {
                let desk = state
                    .creation
                    .as_ref()
                    .ok_or("no creation to keep rules in")?;
                desk.set_rules(*after).await?;
            }
            Facet::Draft { board, after, .. } => match after {
                Some(draft) => state.drafts.put(&board, &draft)?,
                None => {
                    if let Some(open) = draft_now(state, &board) {
                        state.drafts.destroy(&board, open.version)?;
                    }
                }
            },
            Facet::Steps { after, .. } => {
                for s in steps_now(state, branch) {
                    state.steps.remove(branch, &s.pin)?;
                }
                for s in &after {
                    state.steps.put(branch, s)?;
                }
            }
        }
    }
    Ok(())
}
