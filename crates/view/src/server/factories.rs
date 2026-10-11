//! The factories this machine knows: the list the header's menu shows, and
//! the switch from one to another.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use super::{AppState, json};

/// What a page reads when the view keeps no register (`--demo`).
const NO_REGISTER: &str = "no register: the view runs with --demo";

/// Every factory, and which one this view is on.
pub(super) async fn list(State(state): State<AppState>) -> Response {
    let Some(register) = state.factories.as_ref() else {
        return json(&serde_json::json!({ "current": "", "factories": [] }));
    };
    match register.list() {
        Ok(factories) => json(&serde_json::json!({
            "current": state.factory,
            "factories": factories,
        })),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why.reason().to_string()).into_response(),
    }
}

/// Asks the view to start again in the factory `id`. Answers `202` before
/// it does: the page waits for the new view to come up, then reloads.
pub(super) async fn open(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(register) = state.factories.as_ref() else {
        return (StatusCode::NOT_FOUND, NO_REGISTER).into_response();
    };
    let factory = match register.get(&id) {
        Ok(Some(factory)) => factory,
        Ok(None) => return (StatusCode::NOT_FOUND, format!("no factory {id}")).into_response(),
        Err(why) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, why.reason().to_string()).into_response();
        }
    };
    if factory.id == state.factory {
        return json(&serde_json::json!({ "opening": false }));
    }
    if !factory.root.is_dir() {
        return (
            StatusCode::CONFLICT,
            format!("{} is gone from the disk", factory.root.display()),
        )
            .into_response();
    }
    state.switching.send_replace(Some(factory));
    (
        StatusCode::ACCEPTED,
        axum::Json(serde_json::json!({ "opening": true })),
    )
        .into_response()
}
