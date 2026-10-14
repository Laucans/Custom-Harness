//! The mock-up on its easel: the pages of a branch, their preview, the
//! workspace a session edits and what saves or discards it.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::creation_desk::Settled;
use crate::domain::annotations;
use crate::domain::drafts::{self};
use crate::domain::mf_gen;
use crate::domain::mock_history::Facet;
use crate::domain::mockup;
use crate::domain::steps::{self, Step};
use crate::ports::MockState;
use harness_architecture::kit;

use super::capabilities::catalogue_of;
use super::drafts::draft_board;
use super::journal;
use super::mock_history;
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
    /// The request pin they come from: its step keeps the draft as it was,
    /// for its Undo. Empty: none.
    #[serde(default)]
    pin: String,
    /// The board the pin is on.
    #[serde(default)]
    board: String,
    /// The page the request was pinned on (`pages/…`): its rules go with it.
    #[serde(default)]
    page: String,
    /// Where and why, in words: the request as the human pinned it, the
    /// page, its objective, the spot — told with each request.
    #[serde(default)]
    context: String,
}

/// Keeps, in each pin's step, the files as they were before a write — the
/// write's place in the branch's order with them.
pub(super) fn keep_steps(
    state: &AppState,
    branch: &str,
    pins: &[String],
    board: &str,
    files: &[(String, Option<String>)],
) -> Result<(), String> {
    let kept = state.steps.steps(branch)?;
    let seq = steps::next_seq(&kept);
    for pin in pins.iter().filter(|p| !p.is_empty()) {
        let mut step = kept
            .iter()
            .find(|s| s.pin == *pin)
            .cloned()
            .unwrap_or_else(|| Step {
                pin: pin.clone(),
                board: board.to_string(),
                ..Step::default()
            });
        for (path, text) in files {
            step.keep(path, text.clone(), seq);
        }
        state.steps.put(branch, &step)?;
    }
    Ok(())
}

/// Applies requests to a micro-frontend's draft, the units it reads and
/// writes briefed as for its generation.
pub(super) async fn mf_apply(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
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
    let rules = rules_for(&state, &ask.page, std::slice::from_ref(&name)).await;
    let requests = told_with(ask.requests, &ask.context, &rules);
    let paths = vec![mf_gen::draft_path(&name)];
    let before = match state.mockups.clone() {
        Some(mockups) => mock_history::files(mockups, branch.clone(), paths.clone())
            .await
            .ok(),
        None => None,
    };
    let steps_before = mock_history::steps_now(&state, &branch);
    match desk.apply(&branch, ask.spec, units, requests).await {
        Ok(skipped) => {
            if let Some(files) = &before {
                if !ask.pin.is_empty()
                    && let Err(why) = keep_steps(&state, &branch, &[ask.pin], &ask.board, files)
                {
                    return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
                }
                remember_files(
                    &state,
                    &mock_history::Gesture::of(&headers, "a request proposed"),
                    &branch,
                    files,
                    paths,
                    steps_before,
                    Vec::new(),
                )
                .await;
            }
            json(&serde_json::json!({ "skipped": skipped }))
        }
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

/// What a change of the mock-up's files did, kept in its history: the
/// files as they were and became, the request pins' steps, and `more`.
async fn remember_files(
    state: &AppState,
    gesture: &mock_history::Gesture,
    branch: &str,
    before: &[(String, Option<String>)],
    paths: Vec<String>,
    steps_before: Vec<Step>,
    more: Vec<Facet>,
) {
    let Some(mockups) = state.mockups.clone() else {
        return;
    };
    let Ok(after) = mock_history::files(mockups, branch.to_string(), paths).await else {
        return;
    };
    let mut facets = mock_history::file_facets(before, &after);
    facets.push(Facet::Steps {
        before: steps_before,
        after: mock_history::steps_now(state, branch),
    });
    facets.extend(more);
    mock_history::record(state, gesture, facets);
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
    /// The page a fork is made for, relative to `mock/`: it injects the
    /// fork in place of the origin. Empty: no page changes.
    #[serde(default)]
    page: String,
    /// The request pins whose change it validates: their steps keep what
    /// the save writes, their proposals to the rules are accepted.
    #[serde(default)]
    pins: Vec<String>,
}

pub(super) async fn mf_save(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
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
    let forking = ask.how == "fork";
    let page = ask.page.clone();
    let mut paths = vec![
        mf_gen::fragment_path(&name),
        mf_gen::draft_path(&name),
        "pages.json".to_string(),
    ];
    if forking && !page.is_empty() {
        paths.push(page.clone());
    }
    let Some(mockups) = state.mockups.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let mut before = match mock_history::files(mockups, branch.clone(), paths.clone()).await {
        Ok(files) => files,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let steps_before = mock_history::steps_now(&state, &branch);
    let rules_before = mock_history::rules_now(&state).await;
    let fork = forking.then_some(ask.name);
    let saved = match desk.save(&branch, name.clone(), fork, Some(ask.page)).await {
        Ok(saved) => saved,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    if forking {
        before.push((mf_gen::fragment_path(&saved), None));
        paths.push(mf_gen::fragment_path(&saved));
    }
    let mut said = Vec::new();
    if !ask.pins.is_empty() {
        if let Err(why) = keep_steps(&state, &branch, &ask.pins, "", &before) {
            said.push(why);
        }
        if let Some(creation) = &state.creation {
            let fork = (forking && !page.is_empty()).then_some((
                page.as_str(),
                name.as_str(),
                saved.as_str(),
            ));
            match creation
                .settle_pins(&ask.pins, Settled::Kept { fork })
                .await
            {
                Ok(more) => said.extend(more),
                Err(why) => said.push(why),
            }
        }
    }
    let rules = match (rules_before, mock_history::rules_now(&state).await) {
        (Some(before), Some(after)) => vec![Facet::Rules {
            before: Box::new(before),
            after: Box::new(after),
        }],
        _ => Vec::new(),
    };
    let validated = if forking {
        format!("validated: {name} forked as {saved} for this page")
    } else {
        format!("validated: {name} changed on every page that shows it")
    };
    for pin in &ask.pins {
        journal::note(&state, pin, "validated", &validated, paths.clone());
    }
    remember_files(
        &state,
        &mock_history::Gesture::of(&headers, "a proposed change validated"),
        &branch,
        &before,
        paths,
        steps_before,
        rules,
    )
    .await;
    json(&serde_json::json!({ "saved": saved, "said": said }))
}

/// A request pin turned back to not proposed, on its board.
fn pending_again(state: &AppState, board: &str, pin: &str) -> Result<(), String> {
    let kept = state.pins.notes(board)?;
    if let Some(mut note) = kept
        .into_iter()
        .find(|k| k.note.id == pin && k.deleted_at.is_none())
        .map(|k| k.note)
    {
        note.state = annotations::RequestState::Pending;
        state.pins.put(board, &note)?;
    }
    Ok(())
}

/// Requests told where and why they were made, and the product's rules
/// they must keep: what a micro-frontend's rewrite reads with each.
fn told_with(requests: Vec<drafts::Asked>, context: &str, rules: &str) -> Vec<drafts::Asked> {
    requests
        .into_iter()
        .map(|mut a| {
            if !context.trim().is_empty() {
                a.text = format!("{}\n\nWhere and why: {}", a.text.trim(), context.trim());
            }
            if !rules.trim().is_empty() {
                a.text = format!(
                    "{}\n\nThe product's rules it carries — keep them:\n{}",
                    a.text,
                    rules.trim()
                );
            }
            a
        })
        .collect()
}

/// The product's rules a page or its micro-frontends carry, one a line, as
/// the creation keeps them; none without a creation.
pub(super) async fn rules_for(state: &AppState, page: &str, names: &[String]) -> String {
    let Some(rules) = mock_history::rules_now(state).await else {
        return String::new();
    };
    rules
        .requirements
        .iter()
        .filter(|r| {
            (!page.is_empty() && r.pages.iter().any(|p| p == page))
                || r.components.iter().any(|c| names.contains(c))
        })
        .map(|r| {
            format!(
                "- R{} — {}: {} (given {}, when {}, then {})",
                r.id,
                r.title.trim(),
                r.body.trim().replace('\n', " "),
                r.criterion.given.trim(),
                r.criterion.when.trim(),
                r.criterion.then.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A request pin's Undo, as the page asks it.
#[derive(Debug, Default, Deserialize)]
pub(super) struct UndoAsk {
    #[serde(default)]
    branch: String,
    /// The pin taken back.
    #[serde(default)]
    pin: String,
}

/// A request pin's Undo: the files its step wrote as they were before, the
/// steps built on it taken back with it, the rules their proposals changed
/// as they were. Answers the pins taken back, and what was kept.
pub(super) async fn undo_step(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(mockups) = state.mockups.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let ask: UndoAsk = serde_json::from_slice(&body).unwrap_or_default();
    if ask.pin.is_empty() {
        return (StatusCode::BAD_REQUEST, "no pin to take back").into_response();
    }
    let branch = branch_or_integration(&state, Some(ask.branch));
    let kept = match state.steps.steps(&branch) {
        Ok(kept) => kept,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let undone = steps::undone_with(&kept, &ask.pin);
    let files = steps::restored(&undone);
    let paths: Vec<String> = files.iter().map(|b| b.path.clone()).collect();
    let before =
        match mock_history::files(Arc::clone(&mockups), branch.clone(), paths.clone()).await {
            Ok(before) => before,
            Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
        };
    let rules_before = mock_history::rules_now(&state).await;
    let mut pins: Vec<String> = undone.iter().map(|s| s.pin.clone()).collect();
    if pins.is_empty() {
        pins.push(ask.pin.clone());
    }
    let boards: Vec<(String, String)> = undone
        .iter()
        .filter(|s| !s.board.is_empty())
        .map(|s| (s.board.clone(), s.pin.clone()))
        .collect();
    let at = branch.clone();
    let written = tokio::task::spawn_blocking(move || -> Result<(), String> {
        for b in files {
            match b.text {
                Some(text) => mockups.write(&at, &b.path, text.as_bytes())?,
                None => mockups.remove(&at, &b.path)?,
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("the undo was dropped: {e}"))
    .and_then(|done| done);
    if let Err(why) = written {
        return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
    }
    for pin in &pins {
        if let Err(why) = state.steps.remove(&branch, pin) {
            return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
        }
    }
    // The pins taken back, wherever they are: not proposed any more.
    let mut more = Vec::new();
    for (board, pin) in boards {
        let was = mock_history::pin_now(&state, &board, &pin);
        if let Err(why) = pending_again(&state, &board, &pin) {
            return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
        }
        let now = mock_history::pin_now(&state, &board, &pin);
        more.push(mock_history::pin_facet(&board, &pin, was, now));
    }
    for pin in &pins {
        let with = if *pin == ask.pin {
            if pins.len() > 1 {
                format!("taken back, with {} request(s) built on it", pins.len() - 1)
            } else {
                "taken back".to_string()
            }
        } else {
            format!("taken back with the request {} it built on", ask.pin)
        };
        journal::note(&state, pin, "taken back", &with, paths.clone());
    }
    let said = match &state.creation {
        Some(creation) => creation
            .settle_pins(&pins, Settled::Undone)
            .await
            .unwrap_or_else(|why| vec![why]),
        None => Vec::new(),
    };
    if let (Some(before), Some(after)) = (rules_before, mock_history::rules_now(&state).await) {
        more.push(Facet::Rules {
            before: Box::new(before),
            after: Box::new(after),
        });
    }
    remember_files(
        &state,
        &mock_history::Gesture::of(&headers, "a request taken back"),
        &branch,
        &before,
        paths,
        kept,
        more,
    )
    .await;
    json(&serde_json::json!({ "pins": pins, "said": said }))
}

pub(super) async fn mf_drop_draft(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
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
    let paths = vec![mf_gen::draft_path(&name)];
    let before = match state.mockups.clone() {
        Some(mockups) => mock_history::files(mockups, branch.clone(), paths.clone())
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let steps_before = mock_history::steps_now(&state, &branch);
    match desk.drop_draft(&branch, name).await {
        Ok(()) => {
            remember_files(
                &state,
                &mock_history::Gesture::of(&headers, "a proposed change discarded"),
                &branch,
                &before,
                paths,
                steps_before,
                Vec::new(),
            )
            .await;
            json(&serde_json::json!({ "dropped": true }))
        }
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

/// The mock-up's files, oldest written first, each with its kind: what
/// the creation's mock-ups step lists while a session draws.
pub(super) async fn mockup_files(
    State(state): State<AppState>,
    Query(query): Query<MockupQuery>,
) -> Response {
    let Some(mockups) = state.mockups.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let branch = branch_or_integration(&state, query.branch);
    match tokio::task::spawn_blocking(move || mockups.files(&branch)).await {
        Ok(Ok(files)) => {
            let files: Vec<serde_json::Value> = files
                .into_iter()
                .map(|f| serde_json::json!({ "path": f.path, "kind": mockup::kind_of(&f.path), "at": f.at }))
                .collect();
            json(&serde_json::json!({ "files": files }))
        }
        Ok(Err(why)) => (StatusCode::BAD_REQUEST, why).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "the listing was dropped").into_response(),
    }
}

/// The last discarded workspace brought back.
pub(super) async fn mockup_restore(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(mockups) = state.mockups.clone() else {
        return not_found(NO_MOCKUPS);
    };
    let ask: MockupAsk = serde_json::from_slice(&body).unwrap_or_default();
    let branch = branch_or_integration(&state, Some(ask.branch));
    let done = tokio::task::spawn_blocking(move || mockups.restore(&branch)).await;
    match done {
        Ok(Ok(())) => json(&serde_json::json!({ "restored": true })),
        Ok(Err(why)) => (StatusCode::BAD_REQUEST, why).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "the restore was dropped").into_response(),
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
    let names = mockup::injected(&html);
    let rules = rules_for(&state, &ask.page, &names).await;
    let prompt = drafts::apply_prompt(
        &drafts::Page {
            path: &ask.page,
            html: &html,
            components: &ask.components,
            kit: &kit,
            objective: &ask.objective,
            rules: &rules,
        },
        &ask.requests,
    );
    let answer = match desks.ask(prompt).await {
        Ok(answer) => answer,
        Err(why) => return (StatusCode::BAD_GATEWAY, why).into_response(),
    };
    match drafts::answered(&answer, &ask.requests) {
        Ok((ops, mfs)) => json(&serde_json::json!({ "ops": ops, "mfs": mfs })),
        Err(why) => (StatusCode::BAD_GATEWAY, format!("{why}\n\n{answer}")).into_response(),
    }
}

/// The most a written file may weigh.
pub(super) const WRITE_CHARS: usize = 2_000_000;

pub(super) async fn mockup_write(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
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
    let paths = vec![ask.path.clone()];
    let before = mock_history::files(Arc::clone(&mockups), branch.clone(), paths.clone())
        .await
        .unwrap_or_default();
    let steps_before = mock_history::steps_now(&state, &branch);
    let at = branch.clone();
    let written =
        tokio::task::spawn_blocking(move || mockups.write(&at, &ask.path, ask.text.as_bytes()))
            .await;
    match written {
        Ok(Ok(())) => {
            remember_files(
                &state,
                &mock_history::Gesture::of(&headers, "the page written"),
                &branch,
                &before,
                paths,
                steps_before,
                Vec::new(),
            )
            .await;
            json(&serde_json::json!({ "written": true }))
        }
        Ok(Err(why)) => (StatusCode::BAD_REQUEST, why).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "the write was dropped").into_response(),
    }
}

/// What the page sends to apply requests to its draft: the page as the
/// draft shows it, the micro-frontends in short, and the requests.
#[derive(Debug, Default, Deserialize)]
pub(super) struct ApplyAsk {
    /// What the page is for, as the manifest says.
    #[serde(default)]
    objective: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_handed_to_a_micro_frontend_says_where_why_and_the_rules_to_keep() {
        let asked = vec![drafts::Asked {
            id: "p1".into(),
            text: "a folded mode".into(),
            ..drafts::Asked::default()
        }];
        let told = told_with(
            asked.clone(),
            "pinned on Fiche d'entité: a rail on the left",
            "- R3 — Rail: …",
        );
        assert_eq!(
            told[0].text,
            "a folded mode\n\nWhere and why: pinned on Fiche d'entité: a rail on the left\n\nThe product's rules it carries — keep them:\n- R3 — Rail: …"
        );
        assert_eq!(
            told_with(asked, " ", "")[0].text,
            "a folded mode",
            "nothing to add"
        );
    }
}
