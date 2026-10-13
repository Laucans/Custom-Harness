//! The factory-creation page: a new factory founded or a repository
//! connected, then the steps of the new factory's creation — the kind, the
//! brief, the grilling, the scope, the design system, the mock-ups, the
//! review, the initialization.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::creation_desk::{CreationDesk, Founding};
use crate::domain::creation::{BRIEF_PARTS, Creation, Edit, Step};

use super::{AppState, json};

/// Why a page asks in vain.
const NO_DESK: &str = "no factory register: the view runs with --demo";

fn desk_of(state: &AppState) -> Result<&CreationDesk, Box<Response>> {
    state
        .creation
        .as_ref()
        .ok_or_else(|| Box::new((StatusCode::NOT_FOUND, NO_DESK).into_response()))
}

fn refused(why: String) -> Response {
    (StatusCode::BAD_REQUEST, why).into_response()
}

/// What the page draws from: this factory's creation when there is one,
/// the job under way, how far the gates let it go.
async fn view(desk: &CreationDesk, record: Option<Creation>) -> Response {
    let kit = desk.kit().await;
    let facts = CreationDesk::facts_of(kit);
    json(&serde_json::json!({
        "record": record,
        "reachable": record.as_ref().map(|r| r.reachable(facts)),
        "scopeStale": record.as_ref().is_some_and(Creation::scope_stale),
        "designValidated": facts.design_validated,
        "kit": kit,
        "job": desk.job(),
        "branch": desk.integration_branch(),
        "steps": Step::ALL,
        "briefParts": BRIEF_PARTS,
    }))
}

/// This factory's creation, if any, and the job under way.
pub(super) async fn creation(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.record() {
        Ok(record) => view(desk, record).await,
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// The human's edits, and the step they move to.
#[derive(Debug, Deserialize)]
pub(super) struct Saved {
    #[serde(flatten)]
    edit: Edit,
    #[serde(default)]
    step: Option<Step>,
}

pub(super) async fn save(State(state): State<AppState>, Json(saved): Json<Saved>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.save(saved.edit, saved.step).await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The accounts a repository may be created under.
pub(super) async fn owners(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.owners().await {
        Ok(owners) => json(&serde_json::json!({ "owners": owners })),
        Err(why) => (StatusCode::BAD_GATEWAY, why).into_response(),
    }
}

/// A new factory founded: answers at once, the job follows.
pub(super) async fn found(State(state): State<AppState>, Json(ask): Json<Founding>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.found(ask) {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "job": desk.job() })),
        )
            .into_response(),
        Err(why) => refused(why),
    }
}

/// An existing repository to connect.
#[derive(Debug, Deserialize)]
pub(super) struct Connect {
    url: String,
}

/// An existing repository connected as a factory: answers at once.
pub(super) async fn connect(State(state): State<AppState>, Json(ask): Json<Connect>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.connect(&ask.url) {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "job": desk.job() })),
        )
            .into_response(),
        Err(why) => refused(why),
    }
}

/// The description the human wrote at large.
#[derive(Debug, Deserialize)]
pub(super) struct Described {
    description: String,
}

/// The description kept, and sorted by the AI into the brief's parts.
pub(super) async fn decompose(
    State(state): State<AppState>,
    Json(said): Json<Described>,
) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.decompose(said.description).await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The next grilling round.
pub(super) async fn grill(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.grill().await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The scope, proposed again.
pub(super) async fn scope(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.propose_scope().await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// What the brief allows, drawn while the human answers the grilling:
/// answers at once, the job follows.
pub(super) async fn prepare(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.prepare().await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "job": desk.job() })),
        )
            .into_response(),
        Err(why) => refused(why),
    }
}

/// The accepted mock-up made again of micro-frontends, before the review:
/// answers at once, the job follows — `200` when it is done already.
pub(super) async fn componentize(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.componentize_now().await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "job": desk.job() })),
        )
            .into_response(),
        Err(why) => refused(why),
    }
}

/// The requirements, read off the scope and the mock-up.
pub(super) async fn requirements(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.read_requirements().await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The mock-ups drawn: answers at once, the job follows.
pub(super) async fn mockups(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.draw_mockups().await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "job": desk.job() })),
        )
            .into_response(),
        Err(why) => refused(why),
    }
}

/// Back to the brief.
pub(super) async fn regenerate(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.regenerate().await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The proof of concept accepted: the initialization starts.
pub(super) async fn accept(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.accept().await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The creation is over.
pub(super) async fn finish(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.finish().await {
        Ok(()) => json(&serde_json::json!({ "finished": true })),
        Err(why) => refused(why),
    }
}
