//! The factory-creation page: a new factory founded or a repository
//! connected, then the steps of the new factory's creation — the kind, the
//! brief, the grilling, the scope, the design system, the mock-ups, the
//! review, the initialization.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::creation_desk::{CreationDesk, Founding};
use crate::domain::creation::{BRIEF_PARTS, Creation, Edit, RuleChange, RulesState, Step};
use crate::domain::data_draft::DataEdit;
use crate::domain::mock_history::Facet;

use super::mock_history;
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
        "mockupStale": record.as_ref().is_some_and(Creation::mockup_stale),
        "rulesStale": record.as_ref().is_some_and(Creation::rules_stale),
        "marks": record.as_ref().map(|r| r.marks(facts)),
        "ruleGaps": record.as_ref().map(Creation::rule_gaps),
        "auditStale": record.as_ref().is_some_and(Creation::audit_stale),
        "dataStale": record.as_ref().is_some_and(Creation::data_stale),
        "dataGaps": record.as_ref().map(Creation::data_gaps),
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

pub(super) async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(saved): Json<Saved>,
) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let before = desk.rules().await;
    match desk.save(saved.edit, saved.step).await {
        Ok(record) => {
            remember_rules(&state, &headers, "a rule edited", before, &record);
            view(desk, Some(record)).await
        }
        Err(why) => refused(why),
    }
}

/// The rules' change a request made, kept in the mock-up's history.
fn remember_rules(
    state: &AppState,
    headers: &HeaderMap,
    what: &str,
    before: Option<RulesState>,
    record: &Creation,
) {
    if let Some(before) = before {
        mock_history::record(
            state,
            &mock_history::Gesture::of(headers, what),
            vec![Facet::Rules {
                before: Box::new(before),
                after: Box::new(record.rules_state()),
            }],
        );
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

/// One rule read again off the scope and the mock-up.
pub(super) async fn reread_rule(State(state): State<AppState>, Path(id): Path<u32>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.reread_rule(id).await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The rules kept in step with a change of the mock-up: proposals, for
/// the human to accept or refuse.
pub(super) async fn propose_rules(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(change): Json<RuleChange>,
) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let before = desk.rules().await;
    match desk.propose_rules(change).await {
        Ok(record) => {
            remember_rules(&state, &headers, "rules proposed", before, &record);
            view(desk, Some(record)).await
        }
        Err(why) => refused(why),
    }
}

/// One proposed change to the rules, accepted or refused.
pub(super) async fn settle_proposal(
    State(state): State<AppState>,
    Path((key, how)): Path<(u32, String)>,
    headers: HeaderMap,
) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let accept = match how.as_str() {
        "accept" => true,
        "refuse" => false,
        _ => return refused(format!("`{how}` is neither accept nor refuse")),
    };
    let before = desk.rules().await;
    match desk.settle_proposal(key, accept).await {
        Ok(record) => {
            let what = if accept {
                "a rule's change validated"
            } else {
                "a rule's change refused"
            };
            remember_rules(&state, &headers, what, before, &record);
            view(desk, Some(record)).await
        }
        Err(why) => refused(why),
    }
}

/// The architecture's guard asked about the cut.
pub(super) async fn audit(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.audit().await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// One finding of the guard: its fix applied, or dismissed.
pub(super) async fn finding(
    State(state): State<AppState>,
    Path((id, how)): Path<(u32, String)>,
) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let apply = match how.as_str() {
        "apply" => true,
        "dismiss" => false,
        _ => return refused(format!("`{how}`: apply or dismiss")),
    };
    match desk.resolve_finding(id, apply).await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The data model drafted.
pub(super) async fn data(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.draft_data().await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// What the human asks the data model's structure to become.
#[derive(Deserialize)]
pub(super) struct Change {
    request: String,
}

/// The data model's structure changed by the AI.
pub(super) async fn data_change(
    State(state): State<AppState>,
    Json(ask): Json<Change>,
) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.change_data(&ask.request).await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The human's retouches of the data model.
pub(super) async fn data_edit(
    State(state): State<AppState>,
    Json(edit): Json<DataEdit>,
) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.edit_data(edit).await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The data model validated.
pub(super) async fn data_validate(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.validate_data().await {
        Ok(record) => view(desk, Some(record)).await,
        Err(why) => refused(why),
    }
}

/// The drafted data model as the data pane draws it: its tables merged
/// with no database side.
pub(super) async fn data_model(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let record = match desk.record() {
        Ok(record) => record,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let model = match record.as_ref().and_then(|r| r.data.as_ref()) {
        Some(d) => match d.model_json() {
            Ok(text) => Some(harness_architecture::model::build(
                "creation",
                &Ok(None),
                &Ok(Some(text)),
            )),
            Err(why) => return refused(why),
        },
        None => None,
    };
    json(&serde_json::json!({
        "enabled": true,
        "model": model,
        "branch": "",
        "dir": "",
        "error": if model.is_none() { Some("no data model drafted yet") } else { None },
    }))
}

/// The rules validated: nothing is published.
pub(super) async fn validate_rules(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.validate_rules().await {
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

/// The proof of concept drawn ahead, once its scope is settled: answers
/// at once, the job follows.
pub(super) async fn ahead(State(state): State<AppState>) -> Response {
    let desk = match desk_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.draw_ahead().await {
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
