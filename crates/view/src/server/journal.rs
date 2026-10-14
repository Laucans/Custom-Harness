//! A pin's journal over HTTP (`domain::pin_journal`): its events written
//! where they happen — the server's own, and what a page tells of a
//! proposal — and read back with the pin's kept conversation. A journal
//! that cannot be written never fails what happened: it only forgets it.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::domain::annotations;
use crate::domain::note_session::Conversation;
use crate::domain::pin_journal::{self, Event, Mark};

use super::mock_history::now_ms;
use super::{AppState, json, not_found};

/// Writes one event in a pin's journal.
pub(super) fn note(state: &AppState, pin: &str, kind: &str, said: &str, files: Vec<String>) {
    let Ok(kept) = state.journal.events(pin) else {
        return;
    };
    let event = pin_journal::added(
        &kept,
        Event {
            ms: now_ms(),
            kind: kind.to_string(),
            said: said.to_string(),
            files,
            ..Event::default()
        },
    );
    let _ = state.journal.put(pin, &event);
}

/// A session opened on a pin: its turns start at `from` in the pin's
/// conversation.
pub(super) fn session_opened(state: &AppState, pin: &str, mode: &str, from: usize) {
    let Ok(kept) = state.journal.events(pin) else {
        return;
    };
    // A session still marked open (a view stopped meanwhile) is open no more.
    if let Some(stale) = pin_journal::closed(&kept, from, false, Vec::new()) {
        let _ = state.journal.put(pin, &stale);
    }
    let Ok(kept) = state.journal.events(pin) else {
        return;
    };
    let event = pin_journal::added(
        &kept,
        Event {
            ms: now_ms(),
            kind: "session".to_string(),
            said: if mode == "rework" {
                "reworked with the AI".to_string()
            } else {
                "discussed with the AI".to_string()
            },
            session: Some(Mark {
                mode: mode.to_string(),
                from,
                ..Mark::default()
            }),
            ..Event::default()
        },
    );
    let _ = state.journal.put(pin, &event);
}

/// The session open on a pin, closed: its turns end at `to`, kept or
/// dropped, the files it changed.
pub(super) fn session_closed(
    state: &AppState,
    pin: &str,
    to: usize,
    kept: bool,
    files: Vec<String>,
) {
    let Ok(events) = state.journal.events(pin) else {
        return;
    };
    if let Some(done) = pin_journal::closed(&events, to, kept, files) {
        let _ = state.journal.put(pin, &done);
    }
}

/// A pin's journal, with its kept conversation (the turns its sessions
/// point into).
pub(super) async fn read(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
) -> Response {
    if !annotations::is_board(&board) || !annotations::is_id(&id) {
        return not_found("no such pin");
    }
    let events = match state.journal.events(&id) {
        Ok(events) => events,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let conversation: Conversation = state
        .pins
        .conversation(&board, &id)
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    json(&serde_json::json!({ "events": events, "turns": conversation.turns }))
}

/// What a page tells of a pin: a proposal and what it came to.
#[derive(Debug, Default, Deserialize)]
pub(super) struct Told {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    said: String,
}

/// The kinds a page may tell.
const TOLD: [&str; 2] = ["proposed", "not proposed"];

/// Writes what a page tells of a pin.
pub(super) async fn write(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
    Json(told): Json<Told>,
) -> Response {
    if !annotations::is_board(&board) || !annotations::is_id(&id) {
        return not_found("no such pin");
    }
    if !TOLD.contains(&told.kind.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            format!("a page does not tell `{}`", told.kind),
        )
            .into_response();
    }
    let said: String = told.said.chars().take(2_000).collect();
    note(&state, &id, &told.kind, &said, Vec::new());
    json(&serde_json::json!({ "kept": true }))
}
