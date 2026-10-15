//! The data model pane: the diagram, its versions, what uses a table, a
//! comparison of two versions, a divergence, a build, the issue it opens.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::data_desk::{Issue, Wanted};
use crate::domain::builds;
use crate::domain::data_usage::{self, UsedBy};
use crate::domain::data_versions::Versions;
use crate::domain::divergence;
use harness_architecture::compare::{self, Comparison};
use harness_architecture::model::DataModel;

use super::{AppState, json, not_found};

/// What the data pane reads: one version merged, and which one it is.
#[derive(Serialize)]
pub(super) struct DataModelReply<'a> {
    enabled: bool,
    model: Option<&'a DataModel>,
    /// The branch shown; empty for the live files.
    branch: String,
    /// The version's folder; empty for the live files.
    dir: String,
    /// Why there is no model.
    error: Option<String>,
}

/// `?branch=…&dir=…`: a version of the data model; neither, the integration
/// branch's newest.
#[derive(Debug, Default, Deserialize)]
pub(super) struct DataModelQuery {
    branch: Option<String>,
    dir: Option<String>,
    /// `1` after a build: the archive and the live files read again.
    fresh: Option<String>,
}

pub(super) async fn data_model(
    State(state): State<AppState>,
    Query(query): Query<DataModelQuery>,
) -> Response {
    let mut reply = DataModelReply {
        enabled: state.data.is_some(),
        model: None,
        branch: String::new(),
        dir: String::new(),
        error: None,
    };
    let Some(desk) = state.data else {
        return json(&reply);
    };
    let wanted = Wanted {
        branch: query.branch.filter(|b| !b.is_empty()),
        dir: query.dir.filter(|d| !d.is_empty()),
        fresh: query.fresh.as_deref() == Some("1"),
    };
    let shown = desk.model(wanted).await;
    match &shown {
        Ok(shown) => {
            reply.branch.clone_from(&shown.branch);
            reply.dir.clone_from(&shown.dir);
            reply.model = Some(&shown.model);
        }
        Err(why) => reply.error = Some(why.clone()),
    }
    json(&reply)
}

/// What `/api/data-model/compare` is asked: the version shown, and the one
/// to hold it against.
#[derive(Deserialize)]
pub(super) struct CompareQuery {
    branch: Option<String>,
    dir: Option<String>,
    with_branch: Option<String>,
    with_dir: Option<String>,
}

/// One side of a comparison, as shown.
#[derive(Serialize)]
pub(super) struct Side {
    branch: String,
    dir: String,
    origin: String,
}

/// What the version shown changes relative to the other.
#[derive(Serialize)]
pub(super) struct CompareReply {
    shown: Option<Side>,
    other: Option<Side>,
    comparison: Comparison,
    error: Option<String>,
}

pub(super) async fn data_compare(
    State(state): State<AppState>,
    Query(query): Query<CompareQuery>,
) -> Response {
    let mut reply = CompareReply {
        shown: None,
        other: None,
        comparison: Comparison::default(),
        error: None,
    };
    let Some(desk) = state.data else {
        reply.error = Some("nothing reads the product's data model here".to_string());
        return json(&reply);
    };
    let wanted = |branch: Option<String>, dir: Option<String>| Wanted {
        branch: branch.filter(|b| !b.is_empty()),
        dir: dir.filter(|d| !d.is_empty()),
        fresh: false,
    };
    let side = |shown: &crate::data_desk::Shown| Side {
        branch: shown.branch.clone(),
        dir: shown.dir.clone(),
        origin: shown.model.origin.clone(),
    };
    let (shown, other) = tokio::join!(
        desk.model(wanted(query.branch, query.dir)),
        desk.model(wanted(query.with_branch, query.with_dir)),
    );
    match (shown, other) {
        (Ok(shown), Ok(other)) => {
            reply.comparison = compare::compare(&other.model, &shown.model);
            reply.shown = Some(side(&shown));
            reply.other = Some(side(&other));
        }
        (Err(why), _) | (_, Err(why)) => reply.error = Some(why),
    }
    json(&reply)
}

/// Who uses each table and field of the version shown.
#[derive(Serialize)]
pub(super) struct UsedByReply {
    /// The branch the manifests were read on.
    branch: String,
    /// By `table:<name>` and `field:<table>.<name>`.
    used_by: UsedBy,
    /// Why there is nothing: no manifests read here.
    error: Option<String>,
}

/// The units that use the tables and fields of the version asked, read
/// from the manifests of its branch.
pub(super) async fn data_used_by(
    State(state): State<AppState>,
    Query(query): Query<DataModelQuery>,
) -> Response {
    let mut reply = UsedByReply {
        branch: String::new(),
        used_by: UsedBy::new(),
        error: None,
    };
    let (Some(desk), Some(units)) = (state.data.clone(), state.units.clone()) else {
        reply.error = Some("nothing reads the product's manifests here".to_string());
        return json(&reply);
    };
    let wanted = Wanted {
        branch: query.branch.filter(|b| !b.is_empty()),
        dir: query.dir.filter(|d| !d.is_empty()),
        fresh: false,
    };
    let shown = match desk.model(wanted).await {
        Ok(shown) => shown,
        Err(why) => {
            reply.error = Some(why);
            return json(&reply);
        }
    };
    reply.branch = if shown.branch.is_empty() {
        units.integration.clone()
    } else {
        shown.branch
    };
    match units.reading(false).await {
        Ok(reading) => {
            let files = reading
                .refs
                .iter()
                .find(|r| r.git_ref == reply.branch)
                .map_or(&[][..], |r| r.files.as_slice());
            reply.used_by = data_usage::used_by(&shown.model, files);
        }
        Err(why) => reply.error = Some(why),
    }
    json(&reply)
}

/// Every version the archive keeps, by branch.
#[derive(Serialize)]
pub(super) struct VersionsReply<'a> {
    enabled: bool,
    #[serde(flatten)]
    versions: &'a Versions,
}

pub(super) async fn data_versions(State(state): State<AppState>) -> Response {
    let none = Versions::default();
    let Some(desk) = state.data else {
        return json(&VersionsReply {
            enabled: false,
            versions: &none,
        });
    };
    match desk.versions().await {
        Ok(versions) => json(&VersionsReply {
            enabled: true,
            versions: &versions,
        }),
        Err(why) => (StatusCode::SERVICE_UNAVAILABLE, why).into_response(),
    }
}

/// `?branch=…&dir=…`: which diagnosed divergence.
#[derive(Debug, Default, Deserialize)]
pub(super) struct DivergenceQuery {
    branch: String,
    dir: Option<String>,
}

/// The diagnosis the watch recorded for a branch's diverging version.
pub(super) fn divergence_of(
    state: &AppState,
    branch: &str,
    dir: Option<&str>,
) -> Option<divergence::Divergence> {
    let events = state
        .events
        .as_ref()?
        .between(None, None, crate::EVENTS_READ)
        .ok()?;
    divergence::find(&events, branch, dir.filter(|d| !d.is_empty()))
}

pub(super) async fn data_divergence(
    State(state): State<AppState>,
    Query(query): Query<DivergenceQuery>,
) -> Response {
    divergence_of(&state, &query.branch, query.dir.as_deref()).map_or_else(
        || not_found("no diagnosed divergence for this branch"),
        |d| json(&d),
    )
}

/// What the page asks to build: `schema` or `model`, for a branch.
#[derive(Debug, Deserialize)]
pub(super) struct BuildAsk {
    what: String,
    #[serde(default)]
    branch: String,
}

/// Starts the harness command that builds a branch's missing file: a
/// snapshot for its schema, a draft (a pull request) for its model.
pub(super) async fn data_build(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(builds) = state.builds.clone() else {
        return (
            StatusCode::NOT_FOUND,
            "nothing is built here: the view runs with --demo",
        )
            .into_response();
    };
    let ask = match serde_json::from_slice::<BuildAsk>(&body) {
        Ok(ask) => ask,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("not a build request: {e}")).into_response();
        }
    };
    let Some(what) = builds::What::parse(&ask.what) else {
        return (StatusCode::BAD_REQUEST, "build `schema` or `model`").into_response();
    };
    let branch = if ask.branch.is_empty() {
        state.integration_branch.clone()
    } else {
        ask.branch
    };
    if !builds::is_branch(&branch) {
        return (StatusCode::BAD_REQUEST, "not a branch name").into_response();
    }
    match builds.start(what, &branch) {
        Ok(id) => json(&serde_json::json!({ "started": true, "branch": branch, "id": id })),
        Err(why) => (StatusCode::CONFLICT, why).into_response(),
    }
}

/// Every build the pane started, newest first.
pub(super) async fn data_builds(State(state): State<AppState>) -> Response {
    json(
        &state
            .builds
            .as_ref()
            .map(|b| b.builds())
            .unwrap_or_default(),
    )
}

/// A build's whole output, as it stands.
pub(super) async fn data_build_log(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    state
        .builds
        .as_ref()
        .and_then(|b| b.log(&id))
        .map_or_else(|| not_found("no such build"), IntoResponse::into_response)
}

/// What the human sends to open an issue from a divergence.
#[derive(Debug, Deserialize)]
pub(super) struct IssueAsk {
    branch: String,
    dir: String,
    resolution: String,
}

/// Opens the issue a human decided on: their resolution first, the state of
/// affairs after. Its labels start the pipeline.
pub(super) async fn data_issue(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(desk) = state.data.clone() else {
        return (StatusCode::NOT_FOUND, "nothing reads the product here").into_response();
    };
    let ask = match serde_json::from_slice::<IssueAsk>(&body) {
        Ok(ask) => ask,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("not an issue request: {e}"),
            )
                .into_response();
        }
    };
    let Some(found) = divergence_of(&state, &ask.branch, Some(&ask.dir)) else {
        return not_found("no diagnosed divergence for this version");
    };
    let (title, body) = match divergence::issue(&found, &ask.resolution) {
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
