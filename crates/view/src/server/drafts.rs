//! A page's draft on the mock-up: its versions, opened, kept, validated,
//! restored, and every board's history for the pin board.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::domain::annotations;
use crate::domain::drafts::{self, Draft};

use super::notes::stamp;
use super::{AppState, json, not_found};

/// What the page sends when it opens a draft, or writes it.
#[derive(Debug, Default, Deserialize)]
pub(super) struct DraftAsk {
    #[serde(default)]
    base: String,
    #[serde(default)]
    ops: Vec<serde_json::Value>,
    #[serde(default)]
    viewport: u32,
    #[serde(default)]
    name: String,
}

pub(super) fn draft_board(board: &str) -> Result<(), Box<Response>> {
    if annotations::is_board(board) {
        Ok(())
    } else {
        Err(Box::new(not_found("no such board")))
    }
}

pub(super) fn history_of(state: &AppState, board: &str) -> Result<drafts::History, Box<Response>> {
    state
        .drafts
        .versions(board)
        .map(drafts::history)
        .map_err(|why| Box::new((StatusCode::INTERNAL_SERVER_ERROR, why).into_response()))
}

/// A board's draft versions: the open one and the kept ones.
pub(super) async fn draft_history(
    State(state): State<AppState>,
    Path(board): Path<String>,
) -> Response {
    if let Err(r) = draft_board(&board) {
        return *r;
    }
    history_of(&state, &board).map_or_else(|r| *r, |h| json(&h))
}

/// Opens the page's draft: continues the open version, or starts one from
/// the latest — its operations kept when the page is still the one they
/// were made on.
pub(super) async fn open_draft(
    State(state): State<AppState>,
    Path(board): Path<String>,
    body: Bytes,
) -> Response {
    if let Err(r) = draft_board(&board) {
        return *r;
    }
    let ask: DraftAsk = serde_json::from_slice(&body).unwrap_or_default();
    let versions = match state.drafts.versions(&board) {
        Ok(v) => v,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    if let Some(fresh) = drafts::opened(&versions, &ask.base, &stamp())
        && let Err(why) = state.drafts.put(&board, &fresh)
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
    }
    history_of(&state, &board).map_or_else(|r| *r, |h| json(&h))
}

/// Writes the open version's operations, width and name.
pub(super) async fn put_draft(
    State(state): State<AppState>,
    Path(board): Path<String>,
    body: Bytes,
) -> Response {
    if let Err(r) = draft_board(&board) {
        return *r;
    }
    let ask: DraftAsk = match serde_json::from_slice(&body) {
        Ok(ask) => ask,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("not a draft: {e}")).into_response(),
    };
    if let Err(why) = drafts::checked_ops(&ask.ops) {
        return (StatusCode::BAD_REQUEST, why).into_response();
    }
    let history = match history_of(&state, &board) {
        Ok(h) => h,
        Err(r) => return *r,
    };
    let Some(mut current) = history.current else {
        return (
            StatusCode::CONFLICT,
            "no open version: open the draft first",
        )
            .into_response();
    };
    current.ops = ask.ops;
    current.viewport = ask.viewport;
    current.name = ask.name.chars().take(80).collect();
    match state.drafts.put(&board, &current) {
        Ok(()) => json(&current),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// Closes the open version into the kept ones, the oldest beyond the kept
/// count destroyed — validated ones never.
pub(super) fn close_draft(state: &AppState, board: &str, as_state: drafts::State) -> Response {
    let history = match history_of(state, board) {
        Ok(h) => h,
        Err(r) => return *r,
    };
    if let Some(mut current) = history.current {
        current.state = as_state;
        current.closed_at = stamp();
        if let Err(why) = state.drafts.put(board, &current) {
            return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
        }
    }
    let versions = match state.drafts.versions(board) {
        Ok(v) => v,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    for gone in drafts::beyond_kept(&versions) {
        if let Err(why) = state.drafts.destroy(board, gone) {
            return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
        }
    }
    history_of(state, board).map_or_else(|r| *r, |h| json(&h))
}

pub(super) async fn keep_draft(
    State(state): State<AppState>,
    Path(board): Path<String>,
) -> Response {
    if let Err(r) = draft_board(&board) {
        return *r;
    }
    close_draft(&state, &board, drafts::State::Kept)
}

pub(super) async fn validate_draft(
    State(state): State<AppState>,
    Path(board): Path<String>,
    Query(query): Query<VersionQuery>,
) -> Response {
    if let Err(r) = draft_board(&board) {
        return *r;
    }
    let Some(version) = query.version else {
        return close_draft(&state, &board, drafts::State::Validated);
    };
    set_state(&state, &board, version, drafts::State::Validated)
}

pub(super) async fn unvalidate_draft(
    State(state): State<AppState>,
    Path(board): Path<String>,
    Query(query): Query<VersionQuery>,
) -> Response {
    if let Err(r) = draft_board(&board) {
        return *r;
    }
    let Some(version) = query.version else {
        return (StatusCode::BAD_REQUEST, "which version?").into_response();
    };
    set_state(&state, &board, version, drafts::State::Kept)
}

pub(super) fn set_state(
    state: &AppState,
    board: &str,
    version: u32,
    to: drafts::State,
) -> Response {
    let versions = match state.drafts.versions(board) {
        Ok(v) => v,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let Some(mut found) = versions.into_iter().find(|d| d.version == version) else {
        return not_found("no such version");
    };
    if found.state == drafts::State::Open {
        return (
            StatusCode::CONFLICT,
            "the open version is kept or validated by closing it",
        )
            .into_response();
    }
    found.state = to;
    match state.drafts.put(board, &found) {
        Ok(()) => history_of(state, board).map_or_else(|r| *r, |h| json(&h)),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// Restores a version: a new open one with its operations, the former open
/// one kept.
pub(super) async fn restore_draft(
    State(state): State<AppState>,
    Path((board, version)): Path<(String, u32)>,
) -> Response {
    if let Err(r) = draft_board(&board) {
        return *r;
    }
    let history = match history_of(&state, &board) {
        Ok(h) => h,
        Err(r) => return *r,
    };
    if let Some(mut current) = history.current {
        current.state = drafts::State::Kept;
        current.closed_at = stamp();
        if let Err(why) = state.drafts.put(&board, &current) {
            return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
        }
    }
    let versions = match state.drafts.versions(&board) {
        Ok(v) => v,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let fresh = match drafts::restored(&versions, version, &stamp()) {
        Ok(d) => d,
        Err(why) => return not_found(&why),
    };
    if let Err(why) = state.drafts.put(&board, &fresh) {
        return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
    }
    for gone in drafts::beyond_kept(&versions) {
        let _ = state.drafts.destroy(&board, gone);
    }
    history_of(&state, &board).map_or_else(|r| *r, |h| json(&h))
}

/// Starts again from the page as it is: the open version closed, a new one
/// with no operation on the page's current fingerprint.
pub(super) async fn reset_draft(
    State(state): State<AppState>,
    Path(board): Path<String>,
    body: Bytes,
) -> Response {
    if let Err(r) = draft_board(&board) {
        return *r;
    }
    let ask: DraftAsk = serde_json::from_slice(&body).unwrap_or_default();
    let history = match history_of(&state, &board) {
        Ok(h) => h,
        Err(r) => return *r,
    };
    if let Some(mut current) = history.current {
        current.state = drafts::State::Kept;
        current.closed_at = stamp();
        if let Err(why) = state.drafts.put(&board, &current) {
            return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
        }
    }
    let versions = match state.drafts.versions(&board) {
        Ok(v) => v,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let next = versions
        .iter()
        .map(|d| d.version)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let fresh = Draft {
        version: next,
        state: drafts::State::Open,
        base: ask.base,
        created_at: stamp(),
        ..Draft::default()
    };
    if let Err(why) = state.drafts.put(&board, &fresh) {
        return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
    }
    for gone in drafts::beyond_kept(&versions) {
        let _ = state.drafts.destroy(&board, gone);
    }
    history_of(&state, &board).map_or_else(|r| *r, |h| json(&h))
}

/// One board's drafts in short, for the office's pin board.
#[derive(Serialize)]
pub(super) struct DraftSummary {
    board: String,
    current: Option<Draft>,
    kept: usize,
    validated: Vec<u32>,
}

/// Every board that has a draft: its open version and what is kept.
pub(super) async fn all_drafts(State(state): State<AppState>) -> Response {
    let boards = match state.drafts.boards() {
        Ok(b) => b,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let mut all = Vec::new();
    for board in boards {
        let versions = match state.drafts.versions(&board) {
            Ok(v) => v,
            Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
        };
        let history = drafts::history(versions);
        all.push(DraftSummary {
            board,
            validated: history
                .kept
                .iter()
                .filter(|d| d.state == drafts::State::Validated)
                .map(|d| d.version)
                .collect(),
            kept: history.kept.len(),
            current: history.current,
        });
    }
    json(&all)
}

/// Validates the open version — closed and marked as the next round — or,
/// with `?version=`, a kept one.
#[derive(Debug, Default, Deserialize)]
pub(super) struct VersionQuery {
    version: Option<u32>,
}
