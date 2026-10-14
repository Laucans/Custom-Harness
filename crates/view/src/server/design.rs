//! The design system's easel: the kit, its tokens and assets, and what
//! the kit's AI is asked.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::data_desk::Issue;
use crate::domain::design;
use crate::domain::divergence;
use crate::domain::drafts::{self};
use crate::domain::mockup;
use crate::kit_desk::{Asset, KitDesk, Save};
use harness_architecture::kit;

use super::mockup::NO_MOCKUPS;
use super::{AppState, json, not_found};

/// One kit component alone, for the canvas: a page the view makes around
/// its `data-kit` element, the tokens linked. With `?draft=1`, its draft.
pub(super) async fn kit_preview(
    State(state): State<AppState>,
    Path((slug, name)): Path<(String, String)>,
    Query(query): Query<KitPreviewQuery>,
) -> Response {
    if state.mockups.is_none() {
        return not_found(NO_MOCKUPS);
    }
    if mockup::branch_of(&slug).is_none() || !kit::is_name(&name) {
        return not_found("no such component");
    }
    let shown = if query.draft.is_some_and(|d| d == "1") {
        format!("{name}.draft")
    } else {
        name
    };
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        kit::preview_page(&shown),
    )
        .into_response()
}

/// The design system's showcase: the simulated page the harness codes, made
/// of the kit's components as the branch has them.
pub(super) async fn kit_showcase(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Response {
    if state.mockups.is_none() {
        return not_found(NO_MOCKUPS);
    }
    if mockup::branch_of(&slug).is_none() {
        return not_found("no such branch");
    }
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        kit::SHOWCASE,
    )
        .into_response()
}

pub(super) const NO_KIT: &str =
    "nothing reads the product's design system: the view runs with --no-board and no --data-dir";

pub(super) fn kit_of(state: &AppState) -> Result<KitDesk, Box<Response>> {
    state.kit.clone().ok_or_else(|| Box::new(not_found(NO_KIT)))
}

pub(super) async fn design_reply(State(state): State<AppState>) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match desk.reply().await {
        Ok(reply) => json(&reply),
        Err(why) => (StatusCode::SERVICE_UNAVAILABLE, why).into_response(),
    }
}

/// A tokens file injected, or the tokens edited by hand (`edit`).
#[derive(Debug, Default, Deserialize)]
pub(super) struct TokensAsk {
    #[serde(default)]
    json: String,
    #[serde(default)]
    edit: bool,
}

pub(super) async fn design_tokens(State(state): State<AppState>, body: Bytes) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let ask: TokensAsk = match serde_json::from_slice(&body) {
        Ok(ask) => ask,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("not a tokens file: {e}")).into_response();
        }
    };
    if ask.json.chars().count() > TOKENS_CHARS {
        return (StatusCode::BAD_REQUEST, "too big a tokens file").into_response();
    }
    let view = match desk.inject(ask.json, ask.edit).await {
        Ok(view) => view,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    queue_kit(&desk).await;
    json(&view)
}

/// Queues the kit's own generation once tokens exist to style it from —
/// candidate or validated, either already has a `tokens.css` of its own
/// (`KitDesk::inject`, `::generate`): the simulated page means something
/// before the human commits to a candidate, not only after. Best-effort:
/// no Claude at hand, or nothing ticked, is not this call's business.
async fn queue_kit(desk: &KitDesk) {
    let _ = desk.start_generation(Vec::new()).await;
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct BriefAsk {
    #[serde(default)]
    brief: String,
}

pub(super) async fn design_generate(State(state): State<AppState>, body: Bytes) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let ask: BriefAsk = serde_json::from_slice(&body).unwrap_or_default();
    let view = match desk.generate(ask.brief).await {
        Ok(view) => view,
        Err(why) => return (StatusCode::BAD_GATEWAY, why).into_response(),
    };
    queue_kit(&desk).await;
    json(&view)
}

/// Validates the design system, and opens the roadmap task that installs it
/// in the product — once, when a board is there to open it on.
pub(super) async fn design_validate(State(state): State<AppState>) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let view = match desk.validate().await {
        Ok(view) => view,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    queue_kit(&desk).await;
    if view.design.issue.is_empty()
        && let Some(data) = state.data.clone()
    {
        let (title, body) = design::install_issue(&view.design, &view.tokens);
        let issue = Issue {
            title,
            body,
            labels: divergence::ISSUE_LABELS
                .iter()
                .map(|l| (*l).to_string())
                .collect(),
        };
        if let Ok(url) = data.open_issue(issue).await {
            return outcome(desk.set_issue(url).await);
        }
    }
    json(&view)
}

pub(super) async fn design_drop_candidate(State(state): State<AppState>) -> Response {
    match kit_of(&state) {
        Ok(desk) => outcome(desk.drop_candidate().await),
        Err(r) => *r,
    }
}

/// `?kind=moodboard|logo|example|reference&name=<file>&component=<Name>`: the
/// image's bytes are the body.
#[derive(Debug, Default, Deserialize)]
pub(super) struct AssetQuery {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    component: String,
}

pub(super) fn asset_of(query: &AssetQuery) -> Result<Asset, Box<Response>> {
    match query.kind.as_str() {
        "moodboard" => Ok(Asset::Moodboard),
        "logo" => Ok(Asset::Logo),
        "example" => Ok(Asset::Example),
        "reference" if kit::is_name(&query.component) => Ok(Asset::Reference {
            component: query.component.clone(),
        }),
        _ => Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                "an image is a moodboard's, the logo, an interface example, or a component's reference",
            )
                .into_response(),
        )),
    }
}

pub(super) async fn design_asset(
    State(state): State<AppState>,
    Query(query): Query<AssetQuery>,
    body: Bytes,
) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let asset = match asset_of(&query) {
        Ok(asset) => asset,
        Err(r) => return *r,
    };
    match desk.keep_asset(asset, &query.name, body.to_vec()).await {
        Ok(path) => json(&serde_json::json!({ "path": path })),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

pub(super) async fn design_asset_remove(State(state): State<AppState>, body: Bytes) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let query: AssetQuery = serde_json::from_slice(&body).unwrap_or_default();
    let asset = match asset_of(&query) {
        Ok(asset) => asset,
        Err(r) => return *r,
    };
    outcome(desk.remove_asset(asset, &query.name).await)
}

/// What an interface example is of and what to take from it.
#[derive(Debug, Deserialize)]
pub(super) struct ExampleAsk {
    file: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    scope: kit::ExampleScope,
}

pub(super) async fn design_example(State(state): State<AppState>, body: Bytes) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    match serde_json::from_slice::<ExampleAsk>(&body) {
        Ok(ask) => outcome(desk.describe_example(ask.file, ask.note, ask.scope).await),
        Err(why) => (StatusCode::BAD_REQUEST, why.to_string()).into_response(),
    }
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct MessageAsk {
    #[serde(default)]
    message: String,
}

pub(super) async fn design_publish(State(state): State<AppState>, body: Bytes) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let ask: MessageAsk = serde_json::from_slice(&body).unwrap_or_default();
    match desk.publish(ask.message).await {
        Ok(published) => json(&published),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

pub(super) async fn design_discard(State(state): State<AppState>) -> Response {
    match kit_of(&state) {
        Ok(desk) => match desk.discard().await {
            Ok(()) => json(&serde_json::json!({ "discarded": true })),
            Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
        },
        Err(r) => *r,
    }
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct CheckedAsk {
    #[serde(default)]
    checked: Vec<String>,
}

pub(super) async fn kit_checked(State(state): State<AppState>, body: Bytes) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let ask: CheckedAsk = serde_json::from_slice(&body).unwrap_or_default();
    outcome(desk.set_checked(ask.checked).await)
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct CustomAsk {
    #[serde(default)]
    name: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    prompt: String,
}

pub(super) async fn kit_custom(State(state): State<AppState>, body: Bytes) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let ask: CustomAsk = serde_json::from_slice(&body).unwrap_or_default();
    outcome(desk.custom(ask.name, ask.title, ask.prompt).await)
}

pub(super) async fn kit_reprompt(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    if !kit::is_name(&name) {
        return not_found("no such component");
    }
    let ask: CustomAsk = serde_json::from_slice(&body).unwrap_or_default();
    outcome(desk.reprompt(name, ask.title, ask.prompt).await)
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct NamesAsk {
    #[serde(default)]
    names: Vec<String>,
}

pub(super) async fn kit_generate(State(state): State<AppState>, body: Bytes) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    let ask: NamesAsk = serde_json::from_slice(&body).unwrap_or_default();
    match desk.start_generation(ask.names).await {
        Ok(queued) => json(&serde_json::json!({ "queued": queued, "jobs": desk.jobs() })),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

pub(super) async fn kit_usages(State(state): State<AppState>) -> Response {
    match kit_of(&state) {
        Ok(desk) => match desk.usages().await {
            Ok(usages) => json(&usages),
            Err(why) => (StatusCode::SERVICE_UNAVAILABLE, why).into_response(),
        },
        Err(r) => *r,
    }
}

pub(super) async fn kit_delete(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    if !kit::is_name(&name) {
        return not_found("no such component");
    }
    outcome(desk.delete(name).await)
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct KitApplyAsk {
    #[serde(default)]
    requests: Vec<drafts::Asked>,
}

pub(super) async fn kit_apply(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    if !kit::is_name(&name) {
        return not_found("no such component");
    }
    let ask: KitApplyAsk = match serde_json::from_slice(&body) {
        Ok(ask) => ask,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("not an application: {e}")).into_response();
        }
    };
    match desk.apply(name, ask.requests).await {
        Ok(skipped) => json(&serde_json::json!({ "skipped": skipped })),
        Err(why) => (StatusCode::BAD_GATEWAY, why).into_response(),
    }
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct SaveAsk {
    #[serde(default)]
    how: String,
    #[serde(default)]
    name: String,
}

pub(super) async fn kit_save(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    if !kit::is_name(&name) {
        return not_found("no such component");
    }
    let ask: SaveAsk = serde_json::from_slice(&body).unwrap_or_default();
    let how = if ask.how == "fork" {
        Save::Fork { name: ask.name }
    } else {
        Save::Replace
    };
    outcome(desk.save(name, how).await)
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct DraftAsk {
    #[serde(default)]
    html: String,
}

/// A component's draft written as the human retouched it on the easel.
pub(super) async fn kit_write_draft(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    if !kit::is_name(&name) {
        return not_found("no such component");
    }
    let ask: DraftAsk = serde_json::from_slice(&body).unwrap_or_default();
    match desk.write_draft(name, ask.html).await {
        Ok(()) => json(&serde_json::json!({ "written": true })),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

pub(super) async fn kit_drop_draft(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    let desk = match kit_of(&state) {
        Ok(desk) => desk,
        Err(r) => return *r,
    };
    if !kit::is_name(&name) {
        return not_found("no such component");
    }
    match desk.drop_draft(name).await {
        Ok(()) => json(&serde_json::json!({ "dropped": true })),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

/// `?draft=1`: the component's draft rather than the component.
#[derive(Debug, Default, Deserialize)]
pub(super) struct KitPreviewQuery {
    #[serde(default)]
    draft: Option<String>,
}

pub(super) fn outcome<T: Serialize>(result: Result<T, String>) -> Response {
    match result {
        Ok(value) => json(&value),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

/// The most a tokens file may weigh.
pub(super) const TOKENS_CHARS: usize = 200_000;
