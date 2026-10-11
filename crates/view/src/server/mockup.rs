//! The mock-up on its easel: the pages of a branch, their preview, the
//! workspace a session edits and what saves or discards it.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::domain::annotations;
use crate::domain::drafts::{self};
use crate::domain::mf_gen;
use crate::domain::mockup;
use crate::ports::MockState;
use harness_architecture::kit;

use super::capabilities::catalogue_of;
use super::drafts::draft_board;
use super::notes::NO_SESSIONS;
use super::{AppState, json, not_found};

/// Why the easel is empty.
pub(super) const NO_MOCKUPS: &str =
    "nothing reads the product's mock-up: the view runs with --no-board and no --data-dir";

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
    let read = tokio::task::spawn_blocking(move || {
        // The kit's look, as the design system's branch has its components now — published or not.
        if wanted == kit::KIT_CSS {
            return Ok(Some(kit_stylesheet(mockups.as_ref())?.into_bytes()));
        }
        let found = mockups.file(&branch, &wanted)?;
        // The kit and the tokens live on the design system's branch: a page
        // of any branch finds them beside itself. A loader older than the
        // kit — no `data-kit` in it — is replaced by the harness's own.
        let found = match found {
            Some(bytes)
                if wanted == "mock.js" && !String::from_utf8_lossy(&bytes).contains("data-kit") =>
            {
                harness_architecture::assets::content_of("mock/mock.js")
                    .map(|text| text.as_bytes().to_vec())
                    .or(Some(bytes))
            }
            None if (wanted.starts_with("kit/") || wanted.starts_with("design/"))
                && branch != kit::DESIGN_BRANCH =>
            {
                mockups.file(kit::DESIGN_BRANCH, &wanted)?
            }
            None if wanted == "mock.js" => harness_architecture::assets::content_of("mock/mock.js")
                .map(|text| text.as_bytes().to_vec()),
            other => other,
        };
        Ok::<_, String>(found)
    })
    .await;
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
    Query(query): Query<PreviewQuery>,
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
        // A micro-frontend's draft: the same page around `<Name>.draft`, its file.
        mockup::preview_page(&if query.draft.as_deref() == Some("1") {
            format!("{name}.draft")
        } else {
            name
        }),
    )
        .into_response()
}

/// `?draft=1`: the micro-frontend's draft rather than its fragment.
#[derive(Debug, Default, Deserialize)]
pub(super) struct PreviewQuery {
    draft: Option<String>,
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

/// `kit.css` derived from the design system's branch: every generated
/// component's style.
fn kit_stylesheet(mockups: &dyn crate::ports::Mockups) -> Result<String, String> {
    let text = |path: &str| -> Result<Option<String>, String> {
        Ok(mockups
            .file(kit::DESIGN_BRANCH, path)?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
    };
    let catalogue = kit::Kit::parse(text("kit/kit.json")?.as_deref())?;
    let mut parts = Vec::new();
    for c in catalogue
        .components
        .iter()
        .filter(|c| c.state == kit::ComponentState::Generated)
    {
        if let Some(fragment) = text(&kit::Kit::fragment_path(&c.name))? {
            parts.push((c.name.clone(), fragment));
        }
    }
    Ok(kit::stylesheet(&parts))
}

/// The micro-frontends to generate on a branch, as the page asks for them.
#[derive(Debug, Default, Deserialize)]
pub(super) struct GenerateAsk {
    #[serde(default)]
    branch: String,
    #[serde(default)]
    items: Vec<mf_gen::Spec>,
}

/// Queues micro-frontends' generations on a branch, each with the brief of
/// the units it reads and writes — their contracts and rules, on every branch.
pub(super) async fn mf_generate(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(desk) = state.mf.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let ask: GenerateAsk = serde_json::from_slice(&body).unwrap_or_default();
    let branch = branch_or_integration(&state, Some(ask.branch));
    let entries = catalogue_of(&state, false, None)
        .await
        .map(|c| c.entries)
        .unwrap_or_default();
    let items = ask
        .items
        .into_iter()
        .map(|spec| {
            let brief = mf_gen::units_brief(&spec, &entries);
            (spec, brief)
        })
        .collect();
    match desk.start(&branch, items) {
        Ok(queued) => json(&serde_json::json!({ "queued": queued })),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

/// The micro-frontends' generations asked on a branch.
pub(super) async fn mf_generations(
    State(state): State<AppState>,
    Query(query): Query<MockupQuery>,
) -> Response {
    let Some(desk) = state.mf.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let branch = branch_or_integration(&state, query.branch);
    let names = match state.mockups.clone() {
        Some(mockups) => {
            let at = branch.clone();
            tokio::task::spawn_blocking(move || mockups.state(&at))
                .await
                .ok()
                .and_then(Result::ok)
                .map(|s| s.components.into_iter().map(|c| c.name).collect())
                .unwrap_or_default()
        }
        None => Vec::new(),
    };
    let drafts = desk.drafts(&branch, names).await.unwrap_or_default();
    json(&serde_json::json!({ "jobs": desk.jobs(&branch), "drafts": drafts }))
}

/// Requests applied to a micro-frontend, as the page sends them.
#[derive(Debug, Default, Deserialize)]
pub(super) struct MfApplyAsk {
    #[serde(default)]
    branch: String,
    #[serde(default)]
    spec: mf_gen::Spec,
    #[serde(default)]
    requests: Vec<drafts::Asked>,
}

/// Applies requests to a micro-frontend's draft, the units it reads and
/// writes briefed as for its generation.
pub(super) async fn mf_apply(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    let Some(desk) = state.mf.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let ask: MfApplyAsk = serde_json::from_slice(&body).unwrap_or_default();
    if !kit::is_name(&name) || ask.spec.name != name {
        return not_found("no such micro-frontend");
    }
    let branch = branch_or_integration(&state, Some(ask.branch));
    let entries = catalogue_of(&state, false, None)
        .await
        .map(|c| c.entries)
        .unwrap_or_default();
    let units = mf_gen::units_brief(&ask.spec, &entries);
    match desk.apply(&branch, ask.spec, units, ask.requests).await {
        Ok(skipped) => json(&serde_json::json!({ "skipped": skipped })),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

/// A micro-frontend's draft saved, in its place or as a fork.
#[derive(Debug, Default, Deserialize)]
pub(super) struct MfSaveAsk {
    #[serde(default)]
    branch: String,
    /// `replace` or `fork`.
    #[serde(default)]
    how: String,
    /// The fork's name; empty, one is made.
    #[serde(default)]
    name: String,
}

pub(super) async fn mf_save(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    let Some(desk) = state.mf.clone() else {
        return not_found(NO_MOCKUPS);
    };
    if !kit::is_name(&name) {
        return not_found("no such micro-frontend");
    }
    let ask: MfSaveAsk = serde_json::from_slice(&body).unwrap_or_default();
    let branch = branch_or_integration(&state, Some(ask.branch));
    let fork = (ask.how == "fork").then_some(ask.name);
    match desk.save(&branch, name, fork).await {
        Ok(saved) => json(&serde_json::json!({ "saved": saved })),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

pub(super) async fn mf_drop_draft(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    let Some(desk) = state.mf.clone() else {
        return not_found(NO_MOCKUPS);
    };
    if !kit::is_name(&name) {
        return not_found("no such micro-frontend");
    }
    let ask: MockupAsk = serde_json::from_slice(&body).unwrap_or_default();
    let branch = branch_or_integration(&state, Some(ask.branch));
    match desk.drop_draft(&branch, name).await {
        Ok(()) => json(&serde_json::json!({ "dropped": true })),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
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

/// The most of a page an application is handed.
pub(super) const APPLY_HTML_CHARS: usize = 120_000;

/// Applies requests to the page's draft in one go: one question to Claude,
/// answered with the operations, each signed with its request.
pub(super) async fn apply_requests(
    State(state): State<AppState>,
    Path(board): Path<String>,
    body: Bytes,
) -> Response {
    if let Err(r) = draft_board(&board) {
        return *r;
    }
    let Some(desks) = state.note_desks.clone() else {
        return not_found(NO_SESSIONS);
    };
    let ask: ApplyAsk = match serde_json::from_slice(&body) {
        Ok(ask) => ask,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("not an application: {e}")).into_response();
        }
    };
    if ask.requests.is_empty() {
        return (StatusCode::BAD_REQUEST, "no request to apply").into_response();
    }
    let html: String = ask.html.chars().take(APPLY_HTML_CHARS).collect();
    let kit = match state.kit.clone() {
        Some(desk) => desk.summary().await.unwrap_or_default(),
        None => String::new(),
    };
    let prompt = drafts::apply_prompt(&ask.page, &html, &ask.components, &kit, &ask.requests);
    let answer = match desks.ask(prompt).await {
        Ok(answer) => answer,
        Err(why) => return (StatusCode::BAD_GATEWAY, why).into_response(),
    };
    match drafts::ops_from_answer(&answer, &ask.requests) {
        Ok(ops) => json(&serde_json::json!({ "ops": ops })),
        Err(why) => (StatusCode::BAD_GATEWAY, format!("{why}\n\n{answer}")).into_response(),
    }
}

/// The most a written file may weigh.
pub(super) const WRITE_CHARS: usize = 2_000_000;

pub(super) async fn mockup_write(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(mockups) = state.mockups.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let ask: WriteAsk = match serde_json::from_slice(&body) {
        Ok(ask) => ask,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("not a file: {e}")).into_response(),
    };
    if ask.text.chars().count() > WRITE_CHARS {
        return (StatusCode::BAD_REQUEST, "too big a file").into_response();
    }
    let branch = branch_or_integration(&state, Some(ask.branch));
    let written =
        tokio::task::spawn_blocking(move || mockups.write(&branch, &ask.path, ask.text.as_bytes()))
            .await;
    match written {
        Ok(Ok(())) => json(&serde_json::json!({ "written": true })),
        Ok(Err(why)) => (StatusCode::BAD_REQUEST, why).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "the write was dropped").into_response(),
    }
}

/// What the page sends to apply requests to its draft: the page as the
/// draft shows it, the micro-frontends in short, and the requests.
#[derive(Debug, Default, Deserialize)]
pub(super) struct ApplyAsk {
    #[serde(default)]
    page: String,
    #[serde(default)]
    html: String,
    #[serde(default)]
    components: String,
    #[serde(default)]
    requests: Vec<drafts::Asked>,
}

/// What the page sends to write a file of the mock-up in the branch's
/// workspace: the page as the draft shows it, a forked fragment, the
/// draft's own file.
#[derive(Debug, Default, Deserialize)]
pub(super) struct WriteAsk {
    #[serde(default)]
    branch: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    text: String,
}
