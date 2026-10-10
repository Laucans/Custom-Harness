//! The robots' prompts: a station's texts, the drafts and their history,
//! the dry-runs.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::{AppState, json, unix_now};
use crate::domain::blueprint;
use crate::domain::prompts::{self, Deactivate, Edit, OnShelf, Refused, Restore};
use crate::domain::traces::is_run_id;
use crate::ports::TrialAsk;

/// The prompts a station sends: what runs, what waits, what was kept.
pub(super) async fn station_prompts(
    State(state): State<AppState>,
    Path((line, station)): Path<(String, String)>,
) -> Response {
    match state
        .prompts
        .book()
        .and_then(|book| Ok((book, state.prompts.drafts()?, state.prompts.shelf()?)))
    {
        Ok((book, drafts, shelf)) => json(&prompts::of_station(
            &book, &drafts, &shelf, &line, &station,
        )),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// Proposes a new text for the next start.
pub(super) async fn edit_prompt(State(state): State<AppState>, body: Bytes) -> Response {
    match serde_json::from_slice::<Edit>(&body) {
        Ok(edit) => change_drafts(&state, |book, drafts, shelf| {
            prompts::propose(book, drafts, shelf, &edit, unix_now())
        }),
        Err(e) => (StatusCode::BAD_REQUEST, format!("not an edit: {e}")).into_response(),
    }
}

/// Proposes, for the next start, a text kept earlier.
pub(super) async fn restore_prompt(State(state): State<AppState>, body: Bytes) -> Response {
    match serde_json::from_slice::<Restore>(&body) {
        Ok(restore) => change_drafts(&state, |book, drafts, shelf| {
            prompts::restore(book, drafts, shelf, &restore, unix_now())
        }),
        Err(e) => (StatusCode::BAD_REQUEST, format!("not a restore: {e}")).into_response(),
    }
}

/// Deactivates the draft waiting: it stays in the history.
pub(super) async fn deactivate_prompt(State(state): State<AppState>, body: Bytes) -> Response {
    match serde_json::from_slice::<Deactivate>(&body) {
        Ok(ask) => change_drafts(&state, |book, drafts, shelf| {
            prompts::deactivate(book, drafts, shelf, &ask)
        }),
        Err(e) => (StatusCode::BAD_REQUEST, format!("not a deactivation: {e}")).into_response(),
    }
}

/// Makes a deactivated draft the one that waits again.
pub(super) async fn reactivate_prompt(State(state): State<AppState>, body: Bytes) -> Response {
    match serde_json::from_slice::<OnShelf>(&body) {
        Ok(pick) => change_drafts(&state, |book, drafts, shelf| {
            prompts::reactivate(book, drafts, shelf, &pick, unix_now())
        }),
        Err(e) => (StatusCode::BAD_REQUEST, format!("not a reactivation: {e}")).into_response(),
    }
}

/// Deletes a deactivated draft from the history.
pub(super) async fn discard_prompt(State(state): State<AppState>, body: Bytes) -> Response {
    match serde_json::from_slice::<OnShelf>(&body) {
        Ok(pick) => change_drafts(&state, |book, drafts, shelf| {
            prompts::discard(book, drafts, shelf, &pick)
        }),
        Err(e) => (StatusCode::BAD_REQUEST, format!("not a deletion: {e}")).into_response(),
    }
}

/// Read, change, write the drafts and the shelf, one page at a time;
/// answers the prompt as it now stands. The book is only read.
pub(super) fn change_drafts(
    state: &AppState,
    change: impl FnOnce(
        &harness_core::domain::prompt_book::PromptBook,
        &mut harness_core::domain::prompt_book::Drafts,
        &mut harness_core::domain::prompt_book::Shelf,
    ) -> Result<prompts::PromptView, Refused>,
) -> Response {
    // No await between read and write: the lock only keeps two pages apart.
    let Ok(_guard) = state.prompts_lock.lock() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "drafts lock poisoned").into_response();
    };
    let read = state
        .prompts
        .book()
        .and_then(|book| Ok((book, state.prompts.drafts()?, state.prompts.shelf()?)));
    let (book, mut drafts, mut shelf) = match read {
        Ok(all) => all,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let shown = match change(&book, &mut drafts, &mut shelf) {
        Ok(shown) => shown,
        Err(why @ Refused::Unknown(_)) => {
            return (StatusCode::NOT_FOUND, why.to_string()).into_response();
        }
        Err(why) => return (StatusCode::BAD_REQUEST, why.to_string()).into_response(),
    };
    match state
        .prompts
        .keep_drafts(&drafts)
        .and_then(|()| state.prompts.keep_shelf(&shelf))
    {
        Ok(()) => json(&shown),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// What the page sends to dry-run a prompt.
#[derive(Debug, Deserialize)]
pub(super) struct TrialRequest {
    key: String,
    version: String,
    text: String,
    workflow: String,
    run: String,
}

/// Starts a dry-run: one at a time, on a run of the line the prompt is sent on.
pub(super) async fn start_trial(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(trials) = state.trials.as_ref() else {
        return (StatusCode::NOT_FOUND, "this view cannot start a dry-run").into_response();
    };
    let ask = match serde_json::from_slice::<TrialRequest>(&body) {
        Ok(ask) => ask,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("not a dry-run: {e}")).into_response(),
    };
    let Some(prompt) = harness_workflows::prompts::find(&ask.key) else {
        return (
            StatusCode::NOT_FOUND,
            format!("no prompt named {}", ask.key),
        )
            .into_response();
    };
    let Some((_, station)) = prompt
        .stations
        .iter()
        .find(|(line, _)| *line == ask.workflow)
    else {
        return (
            StatusCode::BAD_REQUEST,
            format!("{} is not sent on the {} line", ask.key, ask.workflow),
        )
            .into_response();
    };
    if ask.text.trim().is_empty()
        || ask.text.chars().count() > harness_core::domain::prompt_book::MAX_CHARS
        || !is_run_id(&ask.run)
    {
        return (
            StatusCode::BAD_REQUEST,
            "an empty or oversized text, or not a run",
        )
            .into_response();
    }
    if trials
        .reports()
        .iter()
        .any(|(_, report)| report.state == harness_core::domain::prompt_book::TrialState::Running)
    {
        return (
            StatusCode::CONFLICT,
            "a dry-run is already running — one at a time",
        )
            .into_response();
    }
    let model = blueprint::lines()
        .into_iter()
        .find(|line| line.id == ask.workflow)
        .and_then(|line| line.stations.into_iter().find(|s| s.id == *station))
        .and_then(|s| s.model)
        .unwrap_or_else(|| "sonnet".to_string());
    let id = format!("{}-{}", unix_now(), ask.key.replace('/', "-"));
    let started = trials.start(&TrialAsk {
        id: id.clone(),
        key: ask.key,
        version: ask.version,
        text: ask.text,
        workflow: ask.workflow,
        run: ask.run,
        model,
        effort: "high".to_string(),
    });
    match started {
        Ok(()) => json(&serde_json::json!({ "id": id })),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// `?key=`: one prompt's dry-runs.
#[derive(Debug, Deserialize)]
pub(super) struct TrialsQuery {
    key: String,
}

/// One prompt's last dry-runs, newest first.
pub(super) async fn trials(
    State(state): State<AppState>,
    Query(query): Query<TrialsQuery>,
) -> Response {
    let mut list: Vec<serde_json::Value> = state
        .trials
        .as_ref()
        .map(|trials| trials.reports())
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, report)| report.key == query.key)
        .map(|(id, report)| {
            let mut value = serde_json::to_value(&report).unwrap_or_default();
            value["id"] = serde_json::Value::String(id);
            value
        })
        .collect();
    list.sort_by_key(|value| std::cmp::Reverse(value["started_at"].as_u64().unwrap_or(0)));
    list.truncate(10);
    json(&list)
}
