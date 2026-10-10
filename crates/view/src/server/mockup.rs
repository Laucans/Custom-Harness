//! The mock-up on its easel: the pages of a branch, their preview, the
//! workspace a session edits and what saves or discards it.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::domain::annotations;
use crate::domain::mockup;
use crate::ports::MockState;

/// Why the easel is empty.
pub(super) const NO_MOCKUPS: &str =
    "nothing reads the product's mock-up: the view runs with --no-board and no --data-dir";

use super::{AppState, json, not_found};

/// A file of a branch's mock-up, under `/mock/<branch slug>/`: served so a
/// page's relative links resolve beside it.
pub(super) async fn mock_file(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
) -> Response {
    let Some(mockups) = state.mockups.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let Some(branch) = mockup::branch_of(&slug) else {
        return not_found("no such branch");
    };
    if !mockup::is_safe_path(&path) {
        return not_found("no such file");
    }
    let wanted = path.clone();
    let read = tokio::task::spawn_blocking(move || mockups.file(&branch, &wanted)).await;
    match read {
        Ok(Ok(Some(bytes))) => (
            [
                (header::CONTENT_TYPE, mockup::content_type(&path)),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Ok(Ok(None)) => not_found("no such file in the mock-up"),
        Ok(Err(why)) => (StatusCode::SERVICE_UNAVAILABLE, why).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "the read was dropped").into_response(),
    }
}

/// `?branch=`: which branch's mock-up; empty, the integration branch.
#[derive(Debug, Default, Deserialize)]
pub(super) struct MockupQuery {
    branch: Option<String>,
}

/// The mock-up of a branch, as the page asks for it.
#[derive(Serialize)]
pub(super) struct MockupReply {
    enabled: bool,
    #[serde(flatten)]
    state: MockState,
    /// What the branch's manifests say: Micro-UIs built, Capabilities and
    /// `DataCapabilities` a micro-frontend may need or trigger.
    units: mockup::Units,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// The branch's manifests, as the designer offers them; empty when nothing
/// reads them here.
pub(super) async fn mockup_units(state: &AppState, branch: &str) -> mockup::Units {
    let Some(units) = state.units.clone() else {
        return mockup::Units::default();
    };
    units.reading(false).await.map_or_else(
        |_| mockup::Units::default(),
        |reading| {
            reading
                .refs
                .iter()
                .find(|r| r.git_ref == branch)
                .map(|r| mockup::units_of(&r.files))
                .unwrap_or_default()
        },
    )
}

/// One micro-frontend alone, for the micro-frontends view: a page the
/// view makes around its `data-component` element.
pub(super) async fn mock_preview(
    State(state): State<AppState>,
    Path((slug, name)): Path<(String, String)>,
) -> Response {
    if state.mockups.is_none() {
        return not_found(NO_MOCKUPS);
    }
    if mockup::branch_of(&slug).is_none() || !annotations::is_component(&name) {
        return not_found("no such micro-frontend");
    }
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        mockup::preview_page(&name),
    )
        .into_response()
}

pub(super) fn branch_or_integration(state: &AppState, branch: Option<String>) -> String {
    branch
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| state.integration_branch.clone())
}

pub(super) async fn mockup_state(
    State(state): State<AppState>,
    Query(query): Query<MockupQuery>,
) -> Response {
    let branch = branch_or_integration(&state, query.branch);
    let Some(mockups) = state.mockups.clone() else {
        return json(&MockupReply {
            enabled: false,
            state: MockState {
                branch,
                ..MockState::default()
            },
            units: mockup::Units::default(),
            error: Some(NO_MOCKUPS.to_string()),
        });
    };
    let asked = branch.clone();
    let read = tokio::task::spawn_blocking(move || mockups.state(&asked)).await;
    let units = mockup_units(&state, &branch).await;
    match read {
        Ok(Ok(found)) => json(&MockupReply {
            enabled: true,
            state: found,
            units,
            error: None,
        }),
        Ok(Err(why)) => json(&MockupReply {
            enabled: true,
            state: MockState {
                branch,
                ..MockState::default()
            },
            units,
            error: Some(why),
        }),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "the read was dropped").into_response(),
    }
}

/// What the page sends to save or discard a branch's mock-up.
#[derive(Debug, Default, Deserialize)]
pub(super) struct MockupAsk {
    #[serde(default)]
    branch: String,
    #[serde(default)]
    name: String,
}

/// Saves what the sessions changed under `mock/` on a `mockup/<name>`
/// branch, and says which.
pub(super) async fn mockup_save(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(mockups) = state.mockups.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let ask: MockupAsk = serde_json::from_slice(&body).unwrap_or_default();
    let branch = branch_or_integration(&state, Some(ask.branch));
    let name: String = ask.name.chars().take(120).collect();
    let saved = tokio::task::spawn_blocking(move || mockups.save(&branch, &name)).await;
    match saved {
        Ok(Ok(target)) => json(&serde_json::json!({ "branch": target })),
        Ok(Err(why)) => (StatusCode::BAD_REQUEST, why).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "the save was dropped").into_response(),
    }
}

/// Drops what the sessions changed: the branch's mock-up as committed.
pub(super) async fn mockup_discard(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(mockups) = state.mockups.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let ask: MockupAsk = serde_json::from_slice(&body).unwrap_or_default();
    let branch = branch_or_integration(&state, Some(ask.branch));
    let done = tokio::task::spawn_blocking(move || mockups.discard(&branch)).await;
    match done {
        Ok(Ok(())) => json(&serde_json::json!({ "discarded": true })),
        Ok(Err(why)) => (StatusCode::BAD_REQUEST, why).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "the discard was dropped").into_response(),
    }
}
