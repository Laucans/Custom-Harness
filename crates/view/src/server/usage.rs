//! What the view's own AI calls cost — the design system's, a factory's
//! creation, a micro-frontend's, a pin's — never a run's ledger, which the
//! dashboards' *Spending* screen already reads.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::domain::usage::summarize;

use super::{AppState, json};

/// The view's own spend, summed and the newest calls in full.
pub(super) async fn usage(State(state): State<AppState>) -> Response {
    let Some(store) = state.usage.clone() else {
        return json(&summarize(&[]));
    };
    match tokio::task::spawn_blocking(move || store.all()).await {
        Ok(Ok(spends)) => json(&summarize(&spends)),
        Ok(Err(why)) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the read did not finish: {e}"),
        )
            .into_response(),
    }
}
