//! The factories this machine knows: the list the header's menu shows, and
//! the switch from one to another.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use harness_core::ports::store::factories::Factory;

use crate::removal::{Degree, Refusal};

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
            "factories": factories.iter().map(|f| described(&state, f)).collect::<Vec<_>>(),
        })),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why.reason().to_string()).into_response(),
    }
}

/// A factory as the menu reads it: the register's fields, and whether its
/// folder is one the view made (and so one a delink removes).
fn described(state: &AppState, factory: &Factory) -> serde_json::Value {
    let mut value = serde_json::to_value(factory).unwrap_or_default();
    if let Some(fields) = value.as_object_mut() {
        let owned = state
            .removal
            .as_ref()
            .is_some_and(|removal| removal.is_ours(&factory.root));
        fields.insert("owned".to_string(), owned.into());
    }
    value
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

/// What a destruction carries: the name the human typed to confirm it.
#[derive(Debug, Default, Deserialize)]
pub(super) struct Confirmed {
    #[serde(default)]
    confirm: String,
}

/// Lets the harness go of the factory `id`: its watch asked to stop, its
/// folder removed, the register forgetting it. Its repository stays.
pub(super) async fn delink(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    take_down(&state, id, Degree::Delink, String::new()).await
}

/// Destroys the factory `id`: as a delink, and its repository deleted from
/// GitHub. Only when the body carries the repository's name, typed.
pub(super) async fn destroy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<Confirmed>>,
) -> Response {
    let Json(Confirmed { confirm }) = body.unwrap_or_default();
    take_down(&state, id, Degree::Destroy, confirm).await
}

async fn take_down(state: &AppState, id: String, degree: Degree, confirm: String) -> Response {
    let Some(removal) = state.removal.clone() else {
        return (StatusCode::NOT_FOUND, NO_REGISTER).into_response();
    };
    // `ps`, `kill`, `gh` and `git` run to the end: not on the server's thread.
    match tokio::task::spawn_blocking(move || removal.take_down(&id, degree, &confirm)).await {
        Ok(Ok(report)) => json(&report),
        Ok(Err(refusal)) => {
            let status = match refusal {
                Refusal::Unknown(_) => StatusCode::NOT_FOUND,
                Refusal::Current => StatusCode::CONFLICT,
                Refusal::Confirmation(_) => StatusCode::BAD_REQUEST,
                Refusal::Failed(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, refusal.reason()).into_response()
        }
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why.to_string()).into_response(),
    }
}
