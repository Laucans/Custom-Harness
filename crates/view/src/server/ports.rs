//! The Ports pane: the product's ports, the ports asked and their secrets,
//! and the human's three gestures on them — approve, refuse, change the
//! provider.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::data_desk::{DataDesk, Issue};
use crate::domain::divergence;
use crate::domain::ports_pane::{self, Approval};

use super::{AppState, json, not_found};

/// What the page sends: an approval task, a refusal's reason, a port and
/// its new provider.
#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct PortsAsk {
    approval: u64,
    why: String,
    port: String,
    provider: String,
}

/// What a handler answers when it cannot go on.
type Refusal = (StatusCode, String);

fn desk(state: &AppState) -> Result<DataDesk, Refusal> {
    state.data.clone().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "nothing reads the product here".to_string(),
        )
    })
}

/// The desk and what the page asks, both there.
fn desk_and_ask(state: &AppState, body: &Bytes) -> Result<(DataDesk, PortsAsk), Refusal> {
    let desk = desk(state)?;
    let asked = serde_json::from_slice(body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("not a ports request: {e}")))?;
    Ok((desk, asked))
}

/// The approval the page names, read again off the board: still open.
async fn approval(desk: &DataDesk, number: u64) -> Result<Approval, Refusal> {
    let read = desk
        .ports()
        .await
        .map_err(|why| (StatusCode::BAD_GATEWAY, why))?;
    read.approvals
        .into_iter()
        .find(|a| a.number == number)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("#{number} is no open port approval"),
            )
        })
}

/// The ports on the integration branch, the ports asked, their secrets.
pub(super) async fn ports(State(state): State<AppState>) -> Response {
    let desk = match desk(&state) {
        Ok(desk) => desk,
        Err(refusal) => return refusal.into_response(),
    };
    match desk.ports().await {
        Ok(read) => json(&read),
        Err(why) => (StatusCode::BAD_GATEWAY, why).into_response(),
    }
}

/// A port asked, approved: its task closed, which unblocks the one that
/// waits on it.
pub(super) async fn approve(State(state): State<AppState>, body: Bytes) -> Response {
    let (desk, asked) = match desk_and_ask(&state, &body) {
        Ok(both) => both,
        Err(refusal) => return refusal.into_response(),
    };
    let found = match approval(&desk, asked.approval).await {
        Ok(found) => found,
        Err(refusal) => return refusal.into_response(),
    };
    match desk
        .close_issue(found.number, ports_pane::approved(&found))
        .await
    {
        Ok(()) => json(&serde_json::json!({ "approved": found.number })),
        Err(why) => (StatusCode::BAD_GATEWAY, why).into_response(),
    }
}

/// A port asked, refused: the waiting task goes to the harness's decision,
/// then the approval closes.
pub(super) async fn refuse(State(state): State<AppState>, body: Bytes) -> Response {
    let (desk, asked) = match desk_and_ask(&state, &body) {
        Ok(both) => both,
        Err(refusal) => return refusal.into_response(),
    };
    let found = match approval(&desk, asked.approval).await {
        Ok(found) => found,
        Err(refusal) => return refusal.into_response(),
    };
    if let Err(why) = desk
        .to_decide(found.task, ports_pane::refused(&found.port, &asked.why))
        .await
    {
        return (StatusCode::BAD_GATEWAY, why).into_response();
    }
    let closed = format!(
        "Refused from the plant's Ports pane: #{} goes to the harness's decision.",
        found.task
    );
    match desk.close_issue(found.number, closed).await {
        Ok(()) => json(&serde_json::json!({ "refused": found.number })),
        Err(why) => (StatusCode::BAD_GATEWAY, why).into_response(),
    }
}

/// A port moved to another provider: a roadmap item, ready.
pub(super) async fn change(State(state): State<AppState>, body: Bytes) -> Response {
    let (desk, asked) = match desk_and_ask(&state, &body) {
        Ok(both) => both,
        Err(refusal) => return refusal.into_response(),
    };
    let read = match desk.ports().await {
        Ok(read) => read,
        Err(why) => return (StatusCode::BAD_GATEWAY, why).into_response(),
    };
    let Some(port) = read.ports.iter().find(|p| p.id == asked.port) else {
        return not_found(&format!("ports.json has no port `{}`", asked.port));
    };
    let (title, body) = match ports_pane::change(port, &asked.provider, &asked.why) {
        Ok(composed) => composed,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    let issue = Issue {
        title,
        body,
        labels: divergence::ISSUE_LABELS
            .iter()
            .map(|l| (*l).to_string())
            .collect(),
    };
    match desk.open_issue(issue).await {
        Ok(url) => json(&serde_json::json!({ "url": url })),
        Err(why) => (StatusCode::BAD_GATEWAY, why).into_response(),
    }
}
