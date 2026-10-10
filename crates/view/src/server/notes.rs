//! The pins on a board, their undo, and the sessions a pin opens.

use axum::body::Bytes;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::desks::{TermQuery, term};
use super::mockup::{NO_MOCKUPS, branch_or_integration};
use super::{AppState, json, not_found};
use crate::data_desk::Wanted;
use crate::domain::annotations;
use crate::domain::data_versions::{self};
use crate::domain::note_session::{self, Brief};

/// A board's pins, the deleted ones left out.
pub(super) async fn notes(State(state): State<AppState>, Path(board): Path<String>) -> Response {
    if !annotations::is_board(&board) {
        return not_found("no such board");
    }
    match state.pins.notes(&board) {
        Ok(kept) => json(
            &kept
                .into_iter()
                .filter(|k| k.deleted_at.is_none())
                .map(|k| k.note)
                .collect::<Vec<_>>(),
        ),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// The clock, as the traces write it.
pub(super) fn stamp() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Keeps one pin — new, moved, rewritten, its kind changed.
pub(super) async fn put_note(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    if !annotations::is_board(&board) || !annotations::is_id(&id) {
        return not_found("no such pin");
    }
    let note = match serde_json::from_slice::<annotations::Note>(&body) {
        Ok(note) if note.id == id => note,
        Ok(_) => {
            return (StatusCode::BAD_REQUEST, "the pin's id is not the path's").into_response();
        }
        Err(e) => return (StatusCode::BAD_REQUEST, format!("not a pin: {e}")).into_response(),
    };
    let note = match annotations::checked(note) {
        Ok(note) => note,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    match state.pins.put(&board, &note) {
        Ok(()) => json(&note),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// Deletes a pin — only marked, so ctrl+z brings it back; the deleted pins
/// past the undo depth are destroyed, conversation and all.
pub(super) async fn delete_note(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
) -> Response {
    if !annotations::is_board(&board) || !annotations::is_id(&id) {
        return not_found("no such pin");
    }
    // An open session's turns are kept with the pin before it goes.
    if let Some(desks) = &state.note_desks {
        let _ = desks.close(&board, &id, false).await;
    }
    let done = state
        .pins
        .mark_deleted(&board, &id, Some(&stamp()))
        .and_then(|()| state.pins.notes(&board))
        .and_then(|kept| {
            let deleted: Vec<(String, String)> = kept
                .into_iter()
                .filter_map(|k| Some((k.note.id, k.deleted_at?)))
                .collect();
            annotations::beyond_undo(&deleted)
                .iter()
                .try_for_each(|gone| state.pins.destroy(&board, gone))
        });
    match done {
        Ok(()) => json(&serde_json::json!({ "deleted": id })),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// Brings back the pin deleted last.
pub(super) async fn undo_note(
    State(state): State<AppState>,
    Path(board): Path<String>,
) -> Response {
    if !annotations::is_board(&board) {
        return not_found("no such board");
    }
    let last = match state.pins.notes(&board) {
        Ok(kept) => kept
            .into_iter()
            .filter(|k| k.deleted_at.is_some())
            .max_by(|a, b| a.deleted_at.cmp(&b.deleted_at)),
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let Some(last) = last else {
        return not_found("nothing to bring back");
    };
    match state.pins.mark_deleted(&board, &last.note.id, None) {
        Ok(()) => json(&last.note),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// What the page sends to open a pin's session: what it shows of the pin's
/// target, the branch the pin belongs to and the version shown.
#[derive(Debug, Default, Deserialize)]
pub(super) struct SessionAsk {
    #[serde(default)]
    context: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    dir: String,
    /// On the mock-up: the page the pin sits on.
    #[serde(default)]
    page: String,
}

/// The most of a schema a session is handed: a few hundred tables.
const SCHEMA_CHARS: usize = 60_000;

/// Why a pin has no session here.
const NO_SESSIONS: &str = "no AI session here: the view runs with --no-steward";

/// Opens a pin's session — or finds it open — and says what runs it.
pub(super) async fn open_session(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let Some(desks) = state.note_desks.clone() else {
        return not_found(NO_SESSIONS);
    };
    let ask: SessionAsk = serde_json::from_slice(&body).unwrap_or_default();
    // The page's context is a description, never a megabyte.
    let context: String = ask.context.chars().take(20_000).collect();
    let schema = match &state.data {
        Some(data) if !ask.branch.is_empty() => data
            .schema(Wanted {
                branch: Some(ask.branch.clone()),
                dir: (!ask.dir.is_empty()).then(|| ask.dir.clone()),
                fresh: false,
            })
            .await
            .map(|text| text.chars().take(SCHEMA_CHARS).collect())
            .unwrap_or_default(),
        _ => String::new(),
    };
    // A feature branch: the milestone it belongs to, as the board has it.
    let milestone = data_versions::milestone_number(&ask.branch)
        .and_then(|number| {
            let board = state.board.borrow().clone()?;
            board
                .milestones
                .iter()
                .find(|m| m.issue.number == number)
                .map(note_session::milestone_words)
        })
        .unwrap_or_default();
    let mut brief = Brief {
        context,
        branch: ask.branch.clone(),
        schema,
        milestone,
        cwd: None,
        page: String::new(),
    };
    // A pin on the mock-up: its session runs in the branch's workspace,
    // made now if it is the first.
    if board.starts_with("mockup--") {
        let Some(mockups) = state.mockups.clone() else {
            return not_found(NO_MOCKUPS);
        };
        let branch = branch_or_integration(&state, Some(ask.branch));
        let wanted = branch.clone();
        let made = tokio::task::spawn_blocking(move || mockups.workspace(&wanted)).await;
        let cwd = match made {
            Ok(Ok(cwd)) => cwd,
            Ok(Err(why)) => return (StatusCode::BAD_REQUEST, why).into_response(),
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the checkout was dropped",
                )
                    .into_response();
            }
        };
        brief.cwd = Some(cwd);
        brief.branch = branch;
        brief.page = ask.page.chars().take(200).collect();
    }
    match desks.open(&board, &id, brief).await {
        Ok(command) => json(&serde_json::json!({ "available": true, "command": command })),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

/// A pin's session's terminal.
pub(super) async fn session_term(
    ws: WebSocketUpgrade,
    Path((board, id)): Path<(String, String)>,
    Query(query): Query<TermQuery>,
    State(state): State<AppState>,
) -> Response {
    let desk = state
        .note_desks
        .as_ref()
        .and_then(|desks| desks.desk(&board, &id));
    term(ws, &query, desk, "this pin's session is not open")
}

/// Closes a pin's session; its turns join the pin's conversation.
pub(super) async fn close_session(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
) -> Response {
    let Some(desks) = state.note_desks.clone() else {
        return not_found(NO_SESSIONS);
    };
    match desks.close(&board, &id, true).await {
        Ok(turns) => json(&serde_json::json!({ "turns": turns })),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// Turns a brainstorm into a note (its conclusion) or an asking (its
/// request), the AI writing either.
pub(super) async fn turn_note(
    State(state): State<AppState>,
    Path((board, id, kind)): Path<(String, String, String)>,
) -> Response {
    let kind = match kind.as_str() {
        "note" => annotations::Kind::Note,
        "asking" => annotations::Kind::Asking,
        _ => return not_found("a brainstorm turns into a note or an asking"),
    };
    let Some(desks) = state.note_desks.clone() else {
        return not_found(NO_SESSIONS);
    };
    match desks.turn_into(&board, &id, kind).await {
        Ok(note) => json(&note),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}
