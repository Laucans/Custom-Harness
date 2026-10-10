//! The janitor's pane: the yard weighed, diagnosed, swept, its settings.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::domain::cleanup::Settings;

/// What a page reads of the janitor when the view runs without him.
const NO_JANITOR: &str = "no janitor: the view runs with --no-janitor";

use super::{AppState, json};

/// The janitor's last weighing, last sweep, settings and what they are doing.
pub(super) async fn janitor(State(state): State<AppState>) -> Response {
    state.janitor.as_ref().map_or_else(
        || json(&serde_json::json!({ "available": false })),
        |janitor| json(&janitor.status()),
    )
}

/// Weighs the yard now; answers once the walk is done.
pub(super) async fn janitor_diagnose(State(state): State<AppState>) -> Response {
    let Some(janitor) = state.janitor.as_ref() else {
        return (StatusCode::NOT_FOUND, NO_JANITOR).into_response();
    };
    janitor.diagnose().await;
    json(&janitor.status())
}

/// Sweeps the yard now — a fresh weighing first, so nothing goes on an old
/// one — and answers with what went.
pub(super) async fn janitor_sweep(State(state): State<AppState>) -> Response {
    let Some(janitor) = state.janitor.as_ref() else {
        return (StatusCode::NOT_FOUND, NO_JANITOR).into_response();
    };
    janitor.sweep(false).await;
    json(&janitor.status())
}

/// New settings, as JSON: the limit, the threshold, whether to sweep on their
/// own and whether strays go. Out of bounds is a `400` with the reason.
pub(super) async fn janitor_settings(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(janitor) = state.janitor.as_ref() else {
        return (StatusCode::NOT_FOUND, NO_JANITOR).into_response();
    };
    let settings = match Settings::parse(&String::from_utf8_lossy(&body)) {
        Ok(settings) => settings,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    match janitor.set(settings) {
        Ok(status) => json(&status),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}
