//! The steward's and the doctor's desks: their status, their terminals
//! over a WebSocket, and the diagnoses the doctor is asked for.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::desk::Desk;
use crate::domain::doctor::{self, Diagnosis};
use crate::domain::steward::Status;
use crate::domain::traces::is_run_id;
/// The terminal size when the page did not say.
const TERM_DEFAULT: (u16, u16) = (120, 32);

/// How long the doctor gets to answer before a diagnosis is given up on.
const DIAGNOSIS_PATIENCE: Duration = Duration::from_mins(20);

/// How long a freshly started doctor gets to show its prompt before the
/// question is typed; typed sooner, the keystrokes land in a program that is
/// not yet reading them as text.
const DOCTOR_WARM_UP: Duration = Duration::from_secs(5);

/// A pause between the pasted question and the Enter that sends it.
const PASTE_SETTLE: Duration = Duration::from_millis(400);

/// How much of the doctor's recent output is kept to hear a mark in: a mark
/// is one line, repainted a few times at most.
const EARSHOT: usize = 32 * 1024;

use std::time::Duration;

use super::Diagnoses;
use super::{AppState, is_workflow, json, not_found};

/// Whether there is someone for this desk, and whether they sit at it.
pub(super) fn status_of(desk: Option<&Desk>) -> Status {
    desk.map_or_else(
        || Status {
            available: false,
            live: false,
            command: String::new(),
        },
        |desk| Status {
            available: true,
            live: desk.is_live(),
            command: desk.command(),
        },
    )
}

/// Whether there is a steward, and whether they are at the desk.
pub(super) async fn steward(State(state): State<AppState>) -> Response {
    json(&status_of(state.desk.as_ref()))
}

/// Whether there is a doctor, and whether they are in.
pub(super) async fn doctor(State(state): State<AppState>) -> Response {
    json(&status_of(state.doctor.as_ref()))
}

/// The diagnoses asked so far, by `workflow/run`.
pub(super) async fn diagnoses(State(state): State<AppState>) -> Response {
    let map = state
        .diagnoses
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default();
    json(&map)
}

/// Asks the doctor what happened to one run: its logs' tails are pasted into
/// the doctor's terminal as a question, and a listener marks the diagnosis
/// done when a reply ends with the run's mark.
pub(super) async fn diagnose(
    State(state): State<AppState>,
    Path((workflow, run)): Path<(String, String)>,
) -> Response {
    let Some(desk) = state.doctor.clone() else {
        return not_found("no doctor: the view runs with --no-doctor");
    };
    if !is_workflow(&workflow) || !is_run_id(&run) {
        return not_found("no such run");
    }
    let Some(run_log) = state
        .traces
        .tail(&workflow, &run, "run.log", doctor::RUN_LOG_BYTES)
    else {
        return not_found("no such trace");
    };
    let session_log = state
        .traces
        .tail(&workflow, &run, "session.log", doctor::SESSION_LOG_BYTES)
        .unwrap_or_default();
    let key = format!("{workflow}/{run}");
    let since = jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    // Already asked and not answered: the same question is not typed twice.
    let pending = state
        .diagnoses
        .lock()
        .ok()
        .and_then(|map| match map.get(&key) {
            Some(running @ Diagnosis::Running { .. }) => Some(running.clone()),
            _ => None,
        });
    if let Some(running) = pending {
        return json(&running);
    }
    // One question at a time: held until Enter is sent.
    let _turn = state.asking.lock().await;
    // Listening before the question is typed, so the answer's first bytes are
    // not missed; and before the start, so a program that dies at once is heard.
    let ears = desk.watch();
    let question = doctor::diagnosis(&workflow, &run, &run_log, &session_log);
    if let Err(why) = type_to_doctor(&desk, &question).await {
        return (StatusCode::SERVICE_UNAVAILABLE, why).into_response();
    }
    let asked = Diagnosis::Running {
        since: since.clone(),
    };
    if let Ok(mut map) = state.diagnoses.lock() {
        map.insert(key.clone(), asked.clone());
    }
    tokio::spawn(listen(ears, run, key, since, Arc::clone(&state.diagnoses)));
    json(&asked)
}

/// Types a question into the doctor's terminal — summoning the doctor first
/// when nobody sits there — as a bracketed paste, then Enter. The caller holds
/// `AppState::asking`: two questions at once would land in one input line.
pub(super) async fn type_to_doctor(desk: &Desk, question: &str) -> Result<(), String> {
    let was_live = desk.is_live();
    desk.summon(TERM_DEFAULT.0, TERM_DEFAULT.1)?;
    if !was_live {
        tokio::time::sleep(DOCTOR_WARM_UP).await;
    }
    // A bracketed paste: the newlines inside stay text, Enter comes after.
    let paste = format!("\x1b[200~{question}\x1b[201~");
    desk.write(paste.as_bytes())?;
    tokio::time::sleep(PASTE_SETTLE).await;
    desk.write(b"\r")
}

/// What the page sends when the human asks the doctor about a failed action.
#[derive(Debug, Deserialize)]
pub(super) struct ActionAsk {
    action: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    log: String,
}

/// Asks the doctor about an action that failed on the page: what it was, its
/// error, its log. The page then opens the doctor's pane.
pub(super) async fn ask_doctor(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(desk) = state.doctor.clone() else {
        return (
            StatusCode::NOT_FOUND,
            "no doctor: the view runs with --no-doctor",
        )
            .into_response();
    };
    let ask = match serde_json::from_slice::<ActionAsk>(&body) {
        Ok(ask) => ask,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("not a question: {e}")).into_response(),
    };
    let action: String = ask.action.chars().take(300).collect();
    let error: String = ask.error.chars().take(2_000).collect();
    let question = doctor::about_action(&action, &error, &ask.log);
    let _turn = state.asking.lock().await;
    match type_to_doctor(&desk, &question).await {
        Ok(()) => json(&serde_json::json!({ "asked": true })),
        Err(why) => (StatusCode::SERVICE_UNAVAILABLE, why).into_response(),
    }
}

/// Hears the doctor's output until a reply ends with the run's mark, the
/// program ends, or patience runs out; then writes the diagnosis down.
pub(super) async fn listen(
    mut ears: broadcast::Receiver<Vec<u8>>,
    run: String,
    key: String,
    since: String,
    diagnoses: Diagnoses,
) {
    let mut heard: Vec<u8> = Vec::new();
    let deadline = tokio::time::Instant::now() + DIAGNOSIS_PATIENCE;
    let clock = || {
        jiff::Timestamp::now()
            .strftime("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    };
    let outcome = loop {
        match tokio::time::timeout_at(deadline, ears.recv()).await {
            Ok(Ok(bytes)) => {
                heard.extend(bytes);
                let overflow = heard.len().saturating_sub(EARSHOT);
                heard.drain(..overflow);
                if doctor::diagnosed(&heard, &run) {
                    break Diagnosis::Done { since, at: clock() };
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => {
                break Diagnosis::Lost { since, at: clock() };
            }
        }
    };
    if let Ok(mut map) = diagnoses.lock()
        && matches!(map.get(&key), Some(Diagnosis::Running { .. }))
    {
        map.insert(key, outcome);
    }
}

/// `?cols=N&rows=N`: the size of the pane's terminal.
#[derive(Debug, Deserialize)]
pub(super) struct TermQuery {
    cols: Option<u16>,
    rows: Option<u16>,
}

/// What the page sends as text, beside the raw keystrokes it sends as bytes.
#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(super) enum Control {
    /// The pane changed size.
    Size { cols: u16, rows: u16 },
    /// Kill the program and start a fresh one.
    Restart,
}

pub(super) async fn steward_term(
    ws: WebSocketUpgrade,
    Query(query): Query<TermQuery>,
    State(state): State<AppState>,
) -> Response {
    term(
        ws,
        &query,
        state.desk,
        "no steward: the view runs with --no-steward",
    )
}

pub(super) async fn doctor_term(
    ws: WebSocketUpgrade,
    Query(query): Query<TermQuery>,
    State(state): State<AppState>,
) -> Response {
    term(
        ws,
        &query,
        state.doctor,
        "no doctor: the view runs with --no-doctor",
    )
}

/// Sits the visitor at this desk, or says why there is none.
pub(super) fn term(
    ws: WebSocketUpgrade,
    query: &TermQuery,
    desk: Option<Desk>,
    missing: &str,
) -> Response {
    let Some(desk) = desk else {
        return not_found(missing);
    };
    let cols = query.cols.unwrap_or(TERM_DEFAULT.0).max(20);
    let rows = query.rows.unwrap_or(TERM_DEFAULT.1).max(5);
    ws.on_upgrade(move |socket| attend(socket, desk, cols, rows))
}

/// A note from the desk, shown dim in the terminal.
pub(super) fn aside(text: &str) -> Message {
    Message::Text(format!("\r\n\x1b[2m[{text}]\x1b[0m\r\n").into())
}

/// One visit: the screen so far, then keystrokes one way and output the other
/// until either side leaves. The program outlives the visit.
pub(super) async fn attend(mut socket: WebSocket, desk: Desk, mut cols: u16, mut rows: u16) {
    let (mut output, replay) = match desk.attach(cols, rows) {
        Ok(attached) => attached,
        Err(why) => {
            let _ = socket.send(aside(&why)).await;
            return;
        }
    };
    if !replay.is_empty()
        && socket
            .send(Message::Binary(Bytes::from(replay)))
            .await
            .is_err()
    {
        return;
    }
    // A fresh size makes the program redraw its screen for the visitor.
    let _ = desk.resize(cols, rows);
    loop {
        tokio::select! {
            chunk = output.recv() => match chunk {
                Ok(bytes) => {
                    if socket.send(Message::Binary(Bytes::from(bytes))).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Binary(bytes))) => {
                    if desk.write(&bytes).is_err() {
                        // The program ended; a keystroke calls it back.
                        match desk.summon(cols, rows) {
                            Ok(()) => { let _ = desk.write(&bytes); }
                            Err(why) => { let _ = socket.send(aside(&why)).await; }
                        }
                    }
                }
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<Control>(&text) {
                    Ok(Control::Size { cols: c, rows: r }) => {
                        cols = c.max(20);
                        rows = r.max(5);
                        let _ = desk.resize(cols, rows);
                    }
                    Ok(Control::Restart) => {
                        if let Err(why) = desk.restart(cols, rows) {
                            let _ = socket.send(aside(&why)).await;
                        }
                    }
                    Err(_) => {}
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
            },
        }
    }
}
