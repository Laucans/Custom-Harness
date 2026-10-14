//! The pins on a board, their undo, and the sessions a pin opens.

use axum::body::Bytes;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::data_desk::{Issue, Wanted};
use crate::domain::annotations;
use crate::domain::data_versions::{self};
use crate::domain::design;
use crate::domain::divergence;
use crate::domain::note_session::{self, Brief};
use harness_architecture::kit;

use super::desks::{TermQuery, term};
use super::mockup::{NO_MOCKUPS, branch_or_integration};
use super::{AppState, json, not_found};

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

/// `?forever=1`: a pin destroyed at once rather than marked deleted.
#[derive(Debug, Default, Deserialize)]
pub(super) struct DeleteQuery {
    #[serde(default)]
    forever: Option<String>,
}

/// Deletes a pin — only marked, so ctrl+z brings it back; the deleted pins
/// past the undo depth are destroyed, conversation and all. `forever`
/// destroys it at once: a pin just placed and taken back by undo, which no
/// later undo should bring back.
pub(super) async fn delete_note(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
    Query(query): Query<DeleteQuery>,
) -> Response {
    if !annotations::is_board(&board) || !annotations::is_id(&id) {
        return not_found("no such pin");
    }
    // An open session's turns are kept with the pin before it goes.
    if let Some(desks) = &state.note_desks {
        let _ = desks.close(&board, &id, false).await;
    }
    if query.forever.as_deref() == Some("1") {
        return match state.pins.destroy(&board, &id) {
            Ok(()) => json(&serde_json::json!({ "destroyed": id })),
            Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
        };
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
pub(super) const NO_SESSIONS: &str = "no AI session here: the view runs with --no-steward";

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
    // A pin on the kit's canvas: its session runs in the design system's
    // workspace, where the fragments are.
    if board == design::BOARD {
        let Some(mockups) = state.mockups.clone() else {
            return not_found(NO_MOCKUPS);
        };
        let made = tokio::task::spawn_blocking(move || mockups.workspace(kit::DESIGN_BRANCH)).await;
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
        brief.branch = kit::DESIGN_BRANCH.to_string();
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

pub(super) async fn close_session(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
    Query(query): Query<CloseQuery>,
) -> Response {
    let Some(desks) = state.note_desks.clone() else {
        return not_found(NO_SESSIONS);
    };
    if query.keep == Some(false) {
        return match desks.discard(&board, &id) {
            Ok(turns) => json(&serde_json::json!({ "turns": turns })),
            Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
        };
    }
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
        "request" | "asking" => annotations::Kind::Request,
        _ => return not_found("a brainstorm turns into a note or a request"),
    };
    let Some(desks) = state.note_desks.clone() else {
        return not_found(NO_SESSIONS);
    };
    match desks.turn_into(&board, &id, kind).await {
        Ok(note) => json(&note),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

/// One board and the pins on it, for the office's pin board.
#[derive(Serialize)]
pub(super) struct BoardNotes {
    board: String,
    notes: Vec<annotations::Note>,
}

/// Every pin on every board — what the office's pin board lists.
pub(super) async fn all_notes(State(state): State<AppState>) -> Response {
    let boards = match state.pins.boards() {
        Ok(boards) => boards,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let mut all = Vec::new();
    for board in boards {
        match state.pins.notes(&board) {
            Ok(kept) => all.push(BoardNotes {
                board,
                notes: kept
                    .into_iter()
                    .filter(|k| k.deleted_at.is_none())
                    .map(|k| k.note)
                    .collect(),
            }),
            Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
        }
    }
    json(&all)
}

/// Puts a pin on the product's backlog: an issue carrying `harness:roadmap`
/// and `harness:ready`, titled by the pin's first line, that the planner
/// takes from there; the pin remembers the issue's URL.
pub(super) async fn backlog_note(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    if !annotations::is_board(&board) || !annotations::is_id(&id) {
        return not_found("no such pin");
    }
    let Some(desk) = state.data.clone() else {
        return (StatusCode::NOT_FOUND, "nothing reads the product here").into_response();
    };
    let ask: BacklogAsk = serde_json::from_slice(&body).unwrap_or_default();
    let kept = match state.pins.notes(&board) {
        Ok(kept) => kept,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let Some(mut note) = kept
        .into_iter()
        .filter(|k| k.deleted_at.is_none())
        .map(|k| k.note)
        .find(|n| n.id == id)
    else {
        return not_found("no such pin");
    };
    if !note.issue.is_empty() {
        return (
            StatusCode::CONFLICT,
            format!("already on the backlog: {}", note.issue),
        )
            .into_response();
    }
    let (title, text) = match annotations::backlog_issue(&board, &note, &ask.context) {
        Ok(composed) => composed,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    let issue = Issue {
        title,
        body: text,
        labels: divergence::ISSUE_LABELS
            .iter()
            .map(|l| (*l).to_string())
            .collect(),
    };
    let url = match desk.open_issue(issue).await {
        Ok(url) => url,
        Err(why) => return (StatusCode::BAD_GATEWAY, why).into_response(),
    };
    note.issue = url;
    note.at = stamp();
    match state.pins.put(&board, &note) {
        Ok(()) => json(&note),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// Closes a pin's session; its turns join the pin's conversation.
/// `?keep=false` quits without validating: the session's turns are dropped
/// instead of joining the pin's conversation.
#[derive(Debug, Default, Deserialize)]
pub(super) struct CloseQuery {
    keep: Option<bool>,
}

/// What a page adds to the issue a pin becomes: markdown, the draft's
/// summary or the page's objective.
#[derive(Debug, Default, Deserialize)]
pub(super) struct BacklogAsk {
    #[serde(default)]
    context: String,
}
