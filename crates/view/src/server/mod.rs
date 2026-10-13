//! The HTTP side: the page, its scripts, a few small routes, and the
//! steward's and the doctor's terminals over a WebSocket.
//!
//! The handlers never read the disk for the picture — they read the latest
//! `Snapshot` the poller published, so a slow `gh` never slows a page. Two
//! routes do touch the traces: the live tail of a run file, and an issue
//! body, both on demand and both bounded. One route is a conversation: the
//! WebSocket that carries keystrokes to the steward's desk and its screen back.
//!
//! The front-end is embedded at compile time; `--static-dir` serves it from
//! disk instead, so a page can be redone without a rebuild. The renderer —
//! a Bevy scene compiled to WebAssembly, several megabytes — is never
//! embedded: it is served from `crates/view/static/render/`, where
//! `scripts/build-render.sh` puts it, and the page says so when it is absent.
//!
//! One file per pane: the handlers of a route group live together, and
//! this file keeps the state, the route table, the page's assets and the
//! few helpers every group shares.

mod capabilities;
mod creation;
mod data_model;
mod design;
mod desks;
mod drafts;
mod factories;
mod janitor;
mod mockup;
mod notes;
mod plant;
mod prompts;
mod runs;
#[cfg(test)]
mod tests;
mod usage;

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use serde::Serialize;
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tokio_stream::{Stream, StreamExt as _};

use crate::creation_desk::CreationDesk;
use crate::data_desk::DataDesk;
use crate::desk::Desk;
use crate::domain::design::MAX_ASSET_BYTES;
use crate::domain::doctor::Diagnosis;
use crate::domain::limits::Read;
use crate::domain::snapshot::Snapshot;
use crate::janitor::Janitor;
use crate::kit_desk::KitDesk;
use crate::mf_desk::MfDesk;
use crate::note_desks::NoteDesks;
use crate::ports::{
    BoardReading, Builds, DraftStore, Limits, Mockups, NoteStore, Plant, Prompts, Traces, Trials,
    Usage,
};
use crate::removal::Removal;
use crate::units_desk::UnitsDesk;
use harness_core::domain::quota::Reading;
use harness_core::ports::store::events::EventLog;
use harness_core::ports::store::factories::{Factories, Factory};

/// The diagnoses asked of the doctor, by `workflow/run`, shared with the
/// listeners that hear them end.
pub type Diagnoses = Arc<std::sync::Mutex<HashMap<String, Diagnosis>>>;

/// The factory the human asked to move to, once they did: `main` hears it,
/// stops serving and starts the view again there.
pub type Switching = Arc<watch::Sender<Option<Factory>>>;

use capabilities::{capability_fusion, capability_list};
use data_model::{
    data_build, data_build_log, data_builds, data_compare, data_divergence, data_issue, data_model,
    data_used_by, data_versions,
};
use design::{
    design_asset, design_asset_remove, design_discard, design_drop_candidate, design_generate,
    design_publish, design_reply, design_tokens, design_validate, kit_apply, kit_checked,
    kit_custom, kit_delete, kit_drop_draft, kit_generate, kit_preview, kit_reprompt, kit_save,
    kit_showcase, kit_usages, kit_write_draft,
};
use desks::{ask_doctor, diagnose, diagnoses, doctor, doctor_term, steward, steward_term};
use drafts::{
    all_drafts, draft_history, keep_draft, open_draft, put_draft, reset_draft, restore_draft,
    unvalidate_draft, validate_draft,
};
use factories::{
    delink as factory_delink, destroy as factory_destroy, list as factory_list,
    open as factory_open,
};
use janitor::{janitor, janitor_diagnose, janitor_settings, janitor_sweep};
use mockup::{
    apply_requests, mf_apply, mf_drop_draft, mf_generate, mf_generations, mf_save, mock_file,
    mock_preview, mockup_discard, mockup_save, mockup_state, mockup_write,
};
use notes::{
    all_notes, backlog_note, close_session, delete_note, notes, open_session, put_note,
    session_term, turn_note, undo_note,
};
use plant::{Restart, pending, period, plant_gesture, plant_status, rate_limits, restart};
use prompts::{
    deactivate_prompt, discard_prompt, edit_prompt, reactivate_prompt, restore_prompt, start_trial,
    station_prompts, trials,
};
use runs::{issue, line_of, locate_run, run_file, run_graph, run_stages};

/// What every handler can reach.
#[derive(Clone)]
pub struct AppState {
    /// The latest picture.
    pub snapshot: watch::Receiver<Arc<Snapshot>>,
    /// The latest board, bodies included — `None` until GitHub answered.
    pub board: watch::Receiver<Option<Arc<BoardReading>>>,
    /// The traces, for the live tails.
    pub traces: Arc<dyn Traces>,
    /// Serve the front-end from here instead of the embedded copy.
    pub static_dir: Option<PathBuf>,
    /// Where the renderer bundle lies (`render.js`, `render_bg.wasm`).
    pub render_dir: PathBuf,
    /// The steward's desk — `None` under `--no-steward`.
    pub desk: Option<Desk>,
    /// The doctor's desk — `None` under `--no-doctor`.
    pub doctor: Option<Desk>,
    /// The diagnoses asked of the doctor, by `workflow/run`.
    pub diagnoses: Diagnoses,
    /// Held while a question is typed to the doctor: two questions at once
    /// would land in one input line.
    pub asking: Arc<tokio::sync::Mutex<()>>,
    /// The watch process the page starts and stops — `None` under `--demo`.
    pub plant: Option<Arc<dyn Plant>>,
    /// How long a started watch must live before the start counts.
    pub start_grace: Duration,
    /// The rate limits, read on demand — `None` when nothing can read them.
    pub limits: Option<Arc<dyn Limits>>,
    /// The last Claude reading the page paid for, kept so a click is not a
    /// session.
    pub claude_read: Arc<std::sync::Mutex<Option<Read<Reading>>>>,
    /// The harness's events, for a run's gate verdicts — `None` when the
    /// store could not be opened.
    pub events: Option<Arc<dyn EventLog + Send + Sync>>,
    /// The janitor, who weighs and sweeps the yard — `None` under `--no-janitor`.
    pub janitor: Option<Janitor>,
    /// The product's data model, its versions — `None` when nothing can
    /// read it (`--no-board` without `--data-dir`).
    pub data: Option<DataDesk>,
    /// The harness's commands the data pane starts — `None` under `--demo`.
    pub builds: Option<Arc<dyn Builds>>,
    /// The branch the agents integrate into: what a build names when the
    /// page names none.
    pub integration_branch: String,
    /// Where the human's pins and their conversations are kept.
    pub pins: Arc<dyn NoteStore>,
    /// Where the pages' draft versions are kept.
    pub drafts: Arc<dyn DraftStore>,
    /// The pins' AI sessions — `None` when the view runs no terminal.
    pub note_desks: Option<NoteDesks>,
    /// The product's mock-up on every branch — `None` when nothing can read
    /// the product.
    pub mockups: Option<Arc<dyn Mockups>>,
    /// The design system's easel — `None` when nothing can read the product.
    pub kit: Option<KitDesk>,
    /// The micro-frontends' generations — `None` when nothing can read the product.
    pub mf: Option<MfDesk>,
    /// The prompt book, read only, and the drafts the human leaves for the
    /// next start.
    pub prompts: Arc<dyn Prompts>,
    /// Held across the drafts' read-modify-write.
    pub prompts_lock: Arc<std::sync::Mutex<()>>,
    /// The prompts' dry-runs — `None` when nothing can start one.
    pub trials: Option<Arc<dyn Trials>>,
    /// The restart the human asked for, while it is under way.
    pub restart: Arc<std::sync::Mutex<Option<Restart>>>,
    /// How often a restart looks whether the watch has drained.
    pub restart_poll: Duration,
    /// The product's Capabilities and `DataCapabilities` — `None` when
    /// nothing can read the product.
    pub units: Option<UnitsDesk>,
    /// The harness machine's time zone, asked of the system (`TZ`, else
    /// `/etc/localtime`): the control room counts its days in it.
    pub zone: jiff::tz::TimeZone,
    /// The factories this machine knows — `None` under `--demo`.
    pub factories: Option<Arc<dyn Factories + Send + Sync>>,
    /// A factory's creation: founding a new one from here, and the steps of
    /// this one's when it is being created — `None` without a register.
    pub creation: Option<CreationDesk>,
    /// What the view's own AI calls cost — `None` when nothing is kept.
    pub usage: Option<Arc<dyn Usage>>,
    /// The id of the factory this view is on; empty without a register.
    pub factory: String,
    /// Takes a factory down — delinked, or destroyed — `None` without a
    /// register.
    pub removal: Option<Arc<Removal>>,
    /// Where a switch to another factory is put for `main` to act on.
    pub switching: Switching,
}

const INDEX: &str = include_str!("../../static/index.html");
const STYLE: &str = include_str!("../../static/style.css");
const APP: &str = include_str!("../../static/app.js");
const XTERM_JS: &str = include_str!("../../static/vendor/xterm.js");
const XTERM_CSS: &str = include_str!("../../static/vendor/xterm.css");
const XTERM_FIT: &str = include_str!("../../static/vendor/addon-fit.js");
const CLOCK_JS: &str = include_str!("../../static/clock.js");
const BOARD_JS: &str = include_str!("../../static/board.js");
const ACTIONS_JS: &str = include_str!("../../static/actions.js");
const DATA_MODEL_JS: &str = include_str!("../../static/data-model.js");
const MOCKUP_JS: &str = include_str!("../../static/mockup.js");
const EDIT_JS: &str = include_str!("../../static/edit.js");
const DESIGN_JS: &str = include_str!("../../static/design.js");
const KIT_JS: &str = include_str!("../../static/kit.js");
const GALLERY_JS: &str = include_str!("../../static/gallery.js");
const FACTORIES_JS: &str = include_str!("../../static/factories.js");
const CREATION_JS: &str = include_str!("../../static/creation.js");

/// The routes, with their state.
// One route per line: the table is the function, and splitting it would
// hide where a path is served.
#[allow(clippy::too_many_lines)]
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/style.css", get(style))
        .route("/app.js", get(app))
        .route("/clock.js", get(clock_js))
        .route("/board.js", get(board_js))
        .route("/actions.js", get(actions_js))
        .route("/data-model.js", get(data_model_js))
        .route("/mockup.js", get(mockup_js))
        .route("/edit.js", get(edit_js))
        .route("/design.js", get(design_js))
        .route("/kit.js", get(kit_js))
        .route("/gallery.js", get(gallery_js))
        .route("/factories.js", get(factories_js))
        .route("/creation.js", get(creation_js))
        .route("/mock/{slug}/__preview/{name}", get(mock_preview))
        .route("/mock/{slug}/__kit/{name}", get(kit_preview))
        .route("/mock/{slug}/__showcase/contact", get(kit_showcase))
        .route("/mock/{slug}/{*path}", get(mock_file))
        .route("/render/{file}", get(render_asset))
        .route("/vendor/xterm.js", get(xterm_js))
        .route("/vendor/xterm.css", get(xterm_css))
        .route("/vendor/addon-fit.js", get(xterm_fit))
        .route("/api/snapshot", get(snapshot))
        .route("/api/events", get(events))
        .route("/api/runs/{workflow}/locate", get(locate_run))
        .route("/api/run-of/{run}", get(line_of))
        .route("/api/runs/{workflow}/{run}/stages", get(run_stages))
        .route("/api/runs/{workflow}/{run}/graph", get(run_graph))
        .route("/api/runs/{workflow}/{run}/{file}", get(run_file))
        .route("/api/issues/{number}", get(issue))
        .route("/api/history", get(period))
        .route("/api/limits", get(rate_limits))
        .route("/api/plant", get(plant_status))
        .route("/api/plant/{gesture}", post(plant_gesture))
        .route("/api/steward", get(steward))
        .route("/api/steward/term", get(steward_term))
        .route("/api/doctor", get(doctor))
        .route("/api/doctor/term", get(doctor_term))
        .route("/api/doctor/diagnoses", get(diagnoses))
        .route("/api/doctor/ask", post(ask_doctor))
        .route("/api/doctor/diagnose/{workflow}/{run}", post(diagnose))
        .route("/api/janitor", get(janitor))
        .route("/api/janitor/diagnose", post(janitor_diagnose))
        .route("/api/janitor/sweep", post(janitor_sweep))
        .route("/api/janitor/settings", post(janitor_settings))
        .route("/api/data-model", get(data_model))
        .route("/api/data-model/versions", get(data_versions))
        .route("/api/data-model/used-by", get(data_used_by))
        .route("/api/data-model/compare", get(data_compare))
        .route("/api/data-model/divergence", get(data_divergence))
        .route("/api/data-model/build", post(data_build))
        .route("/api/data-model/builds", get(data_builds))
        .route("/api/data-model/builds/{id}/log", get(data_build_log))
        .route("/api/data-model/issue", post(data_issue))
        .route("/api/mockup", get(mockup_state))
        .route("/api/mockup/save", post(mockup_save))
        .route("/api/mockup/generate", post(mf_generate))
        .route("/api/mockup/generations", get(mf_generations))
        .route("/api/mockup/mf/{name}/apply", post(mf_apply))
        .route("/api/mockup/mf/{name}/save", post(mf_save))
        .route("/api/mockup/mf/{name}/draft/drop", post(mf_drop_draft))
        .route("/api/mockup/discard", post(mockup_discard))
        .route("/api/capabilities", get(capability_list))
        .route("/api/capabilities/fusion", post(capability_fusion))
        .route("/api/drafts", get(all_drafts))
        .route("/api/drafts/{board}", get(draft_history).put(put_draft))
        .route("/api/drafts/{board}/open", post(open_draft))
        .route("/api/drafts/{board}/keep", post(keep_draft))
        .route("/api/drafts/{board}/validate", post(validate_draft))
        .route("/api/drafts/{board}/unvalidate", post(unvalidate_draft))
        .route("/api/drafts/{board}/reset", post(reset_draft))
        .route("/api/drafts/{board}/restore/{version}", post(restore_draft))
        .route("/api/notes/{board}/apply", post(apply_requests))
        .route("/api/mockup/write", post(mockup_write))
        .route("/api/design", get(design_reply))
        .route("/api/design/tokens", post(design_tokens))
        .route("/api/design/generate", post(design_generate))
        .route("/api/design/validate", post(design_validate))
        .route("/api/design/candidate/drop", post(design_drop_candidate))
        // An image is read whole: axum's two megabytes would cut the upload
        // of any photo before the desk could say how heavy one may be.
        .route(
            "/api/design/asset",
            post(design_asset).layer(DefaultBodyLimit::max(
                MAX_ASSET_BYTES.saturating_add(64 * 1024),
            )),
        )
        .route("/api/design/asset/remove", post(design_asset_remove))
        .route("/api/design/publish", post(design_publish))
        .route("/api/design/discard", post(design_discard))
        .route("/api/kit/checked", post(kit_checked))
        .route("/api/kit/custom", post(kit_custom))
        .route("/api/kit/generate", post(kit_generate))
        .route("/api/kit/usages", get(kit_usages))
        .route("/api/kit/{name}", delete(kit_delete))
        .route("/api/kit/{name}/prompt", post(kit_reprompt))
        .route("/api/kit/{name}/apply", post(kit_apply))
        .route("/api/kit/{name}/save", post(kit_save))
        .route("/api/kit/{name}/draft", post(kit_write_draft))
        .route("/api/kit/{name}/draft/drop", post(kit_drop_draft))
        .route("/api/notes", get(all_notes))
        .route("/api/notes/{board}", get(notes))
        .route("/api/notes/{board}/undo", post(undo_note))
        .route("/api/notes/{board}/{id}", put(put_note).delete(delete_note))
        .route("/api/notes/{board}/{id}/backlog", post(backlog_note))
        .route("/api/notes/{board}/{id}/session", post(open_session))
        .route("/api/notes/{board}/{id}/session/term", get(session_term))
        .route("/api/notes/{board}/{id}/session/close", post(close_session))
        .route("/api/notes/{board}/{id}/into/{kind}", post(turn_note))
        .route("/api/prompts/edit", post(edit_prompt))
        .route("/api/prompts/restore", post(restore_prompt))
        .route("/api/prompts/deactivate", post(deactivate_prompt))
        .route("/api/prompts/reactivate", post(reactivate_prompt))
        .route("/api/prompts/discard", post(discard_prompt))
        .route("/api/prompts/trial", post(start_trial))
        .route("/api/prompts/trials", get(trials))
        .route("/api/prompts/{line}/{station}", get(station_prompts))
        .route("/api/pending", get(pending))
        .route("/api/restart", post(restart))
        .route("/api/factories", get(factory_list))
        .route("/api/factories/{id}/open", post(factory_open))
        .route("/api/factories/{id}/delink", post(factory_delink))
        .route("/api/factories/{id}/destroy", post(factory_destroy))
        .route("/api/creation", get(creation::creation).put(creation::save))
        .route("/api/creation/owners", get(creation::owners))
        .route("/api/creation/found", post(creation::found))
        .route("/api/creation/connect", post(creation::connect))
        .route("/api/creation/grill", post(creation::grill))
        .route("/api/creation/scope", post(creation::scope))
        .route("/api/creation/mockups", post(creation::mockups))
        .route("/api/creation/regenerate", post(creation::regenerate))
        .route("/api/creation/accept", post(creation::accept))
        .route("/api/creation/finish", post(creation::finish))
        .route("/api/usage", get(usage::usage))
        .with_state(state)
}

/// A JSON body, without `axum::Json` — which would need `serde`'s `rc`
/// feature to see through the `Arc` the channel hands over.
fn json<T: Serialize>(value: &T) -> Response {
    match serde_json::to_string(value) {
        Ok(body) => (
            [
                (header::CONTENT_TYPE, "application/json; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            body,
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn asset(state: &AppState, name: &str, embedded: &str, mime: &'static str) -> Response {
    let body = state
        .static_dir
        .as_ref()
        .and_then(|dir| std::fs::read_to_string(dir.join(name)).ok())
        .unwrap_or_else(|| embedded.to_string());
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

const HTML: &str = "text/html; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const JS: &str = "application/javascript; charset=utf-8";

async fn index(State(state): State<AppState>) -> Response {
    asset(&state, "index.html", INDEX, HTML)
}

async fn style(State(state): State<AppState>) -> Response {
    asset(&state, "style.css", STYLE, CSS)
}

async fn app(State(state): State<AppState>) -> Response {
    asset(&state, "app.js", APP, JS)
}

async fn clock_js(State(state): State<AppState>) -> Response {
    asset(&state, "clock.js", CLOCK_JS, JS)
}

async fn actions_js(State(state): State<AppState>) -> Response {
    asset(&state, "actions.js", ACTIONS_JS, JS)
}

async fn board_js(State(state): State<AppState>) -> Response {
    asset(&state, "board.js", BOARD_JS, JS)
}

async fn data_model_js(State(state): State<AppState>) -> Response {
    asset(&state, "data-model.js", DATA_MODEL_JS, JS)
}

async fn mockup_js(State(state): State<AppState>) -> Response {
    asset(&state, "mockup.js", MOCKUP_JS, JS)
}

/// The renderer bundle, from disk: the two files wasm-bindgen writes, and
/// nothing else under that folder.
async fn render_asset(State(state): State<AppState>, Path(file): Path<String>) -> Response {
    let mime = match file.as_str() {
        "render.js" => JS,
        "render_bg.wasm" => "application/wasm",
        _ => return not_found("no such renderer file"),
    };
    let path = state
        .static_dir
        .as_ref()
        .map(|dir| dir.join("render").join(&file))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| state.render_dir.join(&file));
    tokio::fs::read(&path).await.map_or_else(
        |_| {
            not_found(
                "the renderer is not built: run scripts/build-render.sh (wasm-pack and the wasm32 target), then reload",
            )
        },
        |bytes| {
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                bytes,
            )
                .into_response()
        },
    )
}

async fn xterm_js(State(state): State<AppState>) -> Response {
    asset(&state, "vendor/xterm.js", XTERM_JS, JS)
}

async fn xterm_css(State(state): State<AppState>) -> Response {
    asset(&state, "vendor/xterm.css", XTERM_CSS, CSS)
}

async fn xterm_fit(State(state): State<AppState>) -> Response {
    asset(&state, "vendor/addon-fit.js", XTERM_FIT, JS)
}

async fn snapshot(State(state): State<AppState>) -> Response {
    let current = state.snapshot.borrow().clone();
    json(&*current)
}

/// One `snapshot` event per change, the current picture first.
async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = WatchStream::new(state.snapshot).map(|snap| {
        let body = serde_json::to_string(&*snap).unwrap_or_default();
        Ok(Event::default().event("snapshot").data(body))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// A log folder name: lowercase words joined by dashes, nothing a path
/// could be smuggled in.
fn is_workflow(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn not_found(what: &str) -> Response {
    (StatusCode::NOT_FOUND, what.to_string()).into_response()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

async fn edit_js(State(state): State<AppState>) -> Response {
    asset(&state, "edit.js", EDIT_JS, JS)
}

async fn kit_js(State(state): State<AppState>) -> Response {
    asset(
        &state,
        "kit.js",
        KIT_JS,
        "application/javascript; charset=utf-8",
    )
}

async fn factories_js(State(state): State<AppState>) -> Response {
    asset(&state, "factories.js", FACTORIES_JS, JS)
}

async fn creation_js(State(state): State<AppState>) -> Response {
    asset(&state, "creation.js", CREATION_JS, JS)
}

async fn gallery_js(State(state): State<AppState>) -> Response {
    asset(&state, "gallery.js", GALLERY_JS, JS)
}

async fn design_js(State(state): State<AppState>) -> Response {
    asset(&state, "design.js", DESIGN_JS, JS)
}
