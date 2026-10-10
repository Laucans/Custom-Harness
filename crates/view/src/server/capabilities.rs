//! The capabilities list and the fusion test opened from it.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::domain::capabilities::{self, Catalogue};
use crate::domain::fusion;

use super::{AppState, json, not_found};

/// What `/api/capabilities` is asked.
#[derive(Deserialize)]
pub(super) struct CatalogueAsk {
    /// Read the product again, rather than the kept reading.
    #[serde(default)]
    fresh: bool,
    /// One branch's units only; every branch when absent.
    #[serde(default)]
    branch: Option<String>,
}

/// The capabilities list, from the kept reading and the latest board.
pub(super) async fn catalogue_of(
    state: &AppState,
    fresh: bool,
    branch: Option<&str>,
) -> Result<Catalogue, (StatusCode, String)> {
    let Some(desk) = state.units.clone() else {
        return Err((
            StatusCode::NOT_FOUND,
            "nothing reads the product here".to_string(),
        ));
    };
    let reading = desk
        .reading(fresh)
        .await
        .map_err(|why| (StatusCode::BAD_GATEWAY, why))?;
    let board = state.board.borrow().clone();
    let issues: Vec<harness_core::domain::Issue> = board
        .map(|b| {
            b.milestones
                .iter()
                .filter(|m| m.issue.is_open())
                .flat_map(|m| m.tasks.iter().cloned())
                .chain(b.bugs.iter().cloned())
                .collect()
        })
        .unwrap_or_default();
    Ok(capabilities::catalogue(
        &reading,
        &desk.integration,
        &issues,
        branch.filter(|b| !b.is_empty()),
    ))
}

/// Every Capability and `DataCapability` of the product.
pub(super) async fn capability_list(
    State(state): State<AppState>,
    Query(ask): Query<CatalogueAsk>,
) -> Response {
    match catalogue_of(&state, ask.fresh, ask.branch.as_deref()).await {
        Ok(catalogue) => json(&catalogue),
        Err(refused) => refused.into_response(),
    }
}

/// The two units a fusion test compares.
#[derive(Deserialize)]
pub(super) struct FusionAsk {
    a: String,
    b: String,
    /// Ask the model to word the answer; without it, the tooling's alone.
    #[serde(default)]
    ai: bool,
    /// Compare the two as this branch has them; every branch when absent.
    #[serde(default)]
    branch: Option<String>,
}

/// Could two units be one? The tooling compares; the model, if asked,
/// words the answer from the comparison alone.
pub(super) async fn capability_fusion(State(state): State<AppState>, body: Bytes) -> Response {
    let ask = match serde_json::from_slice::<FusionAsk>(&body) {
        Ok(ask) => ask,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("not a fusion request: {e}"),
            )
                .into_response();
        }
    };
    if ask.a == ask.b {
        return (StatusCode::BAD_REQUEST, "pick two different units").into_response();
    }
    let catalogue = match catalogue_of(&state, false, ask.branch.as_deref()).await {
        Ok(catalogue) => catalogue,
        Err(refused) => return refused.into_response(),
    };
    let find = |id: &str| catalogue.entries.iter().find(|e| e.id == id);
    let (Some(a), Some(b)) = (find(&ask.a), find(&ask.b)) else {
        return not_found("no such unit in the list");
    };
    let comparison = fusion::compare(a, b);
    let (answer, error) = match (ask.ai, state.units.as_ref()) {
        (true, Some(desk)) => match desk.ask(fusion::question(&comparison, a, b)).await {
            Ok(answer) => (Some(answer.trim().to_string()), None),
            Err(why) => (None, Some(why)),
        },
        _ => (None, None),
    };
    json(&serde_json::json!({ "comparison": comparison, "answer": answer, "error": error }))
}
