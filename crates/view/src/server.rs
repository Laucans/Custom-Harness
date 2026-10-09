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

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};
use tokio_stream::wrappers::WatchStream;
use tokio_stream::{Stream, StreamExt as _};

use crate::data_desk::{DataDesk, Issue, Wanted};
use crate::desk::Desk;
use crate::domain::annotations;
use crate::domain::assemble;
use crate::domain::blueprint;
use crate::domain::builds;
use crate::domain::capabilities::{self, Catalogue};
use crate::domain::cleanup::Settings;
use crate::domain::data_usage::{self, UsedBy};
use crate::domain::data_versions::{self, Versions};
use crate::domain::divergence;
use crate::domain::doctor::{self, Diagnosis};
use crate::domain::fusion;
use crate::domain::gates;
use crate::domain::history::{self, Range};
use crate::domain::limits::{self, Read, Report};
use crate::domain::note_session::{self, Brief};
use crate::domain::observe::observe_run;
use crate::domain::plant::{self, Action, Gesture};
use crate::domain::prompts::{self, Deactivate, Edit, OnShelf, Refused, Restore};
use crate::domain::snapshot::Snapshot;
use crate::domain::snapshot::StationView;
use crate::domain::steward::Status;
use crate::domain::traces::{RunStages, clock_span, is_run_id, line_of_run, run_of, stage_logs};
use crate::janitor::Janitor;
use crate::note_desks::NoteDesks;
use crate::ports::{
    BoardReading, Builds, Limits, NoteStore, Plant, Prompts, Traces, TrialAsk, Trials,
};
use crate::units_desk::UnitsDesk;
use harness_architecture::model::DataModel;
use harness_core::domain::quota::Reading;
use harness_core::ports::store::events::EventLog;

/// The diagnoses asked of the doctor, by `workflow/run`, shared with the
/// listeners that hear them end.
pub type Diagnoses = Arc<std::sync::Mutex<HashMap<String, Diagnosis>>>;

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
    /// The pins' AI sessions — `None` when the view runs no terminal.
    pub note_desks: Option<NoteDesks>,
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
}

const INDEX: &str = include_str!("../static/index.html");
const STYLE: &str = include_str!("../static/style.css");
const APP: &str = include_str!("../static/app.js");
const XTERM_JS: &str = include_str!("../static/vendor/xterm.js");
const XTERM_CSS: &str = include_str!("../static/vendor/xterm.css");
const XTERM_FIT: &str = include_str!("../static/vendor/addon-fit.js");
const CLOCK_JS: &str = include_str!("../static/clock.js");
const BOARD_JS: &str = include_str!("../static/board.js");
const ACTIONS_JS: &str = include_str!("../static/actions.js");
const DATA_MODEL_JS: &str = include_str!("../static/data-model.js");

/// The run files a browser may tail.
const TAILABLE: [&str; 4] = ["run.log", "session.log", "prompts.md", "stream.jsonl"];

/// What a tail returns when the query does not say.
const TAIL_DEFAULT: u64 = 16 * 1024;

/// The most a tail returns however much the query asks.
const TAIL_MAX: u64 = 512 * 1024;

/// The terminal size when the page did not say.
const TERM_DEFAULT: (u16, u16) = (120, 32);

/// The routes, with their state.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/style.css", get(style))
        .route("/app.js", get(app))
        .route("/clock.js", get(clock_js))
        .route("/board.js", get(board_js))
        .route("/actions.js", get(actions_js))
        .route("/data-model.js", get(data_model_js))
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
        .route("/api/data-model/divergence", get(data_divergence))
        .route("/api/data-model/build", post(data_build))
        .route("/api/data-model/builds", get(data_builds))
        .route("/api/data-model/builds/{id}/log", get(data_build_log))
        .route("/api/data-model/issue", post(data_issue))
        .route("/api/capabilities", get(capability_list))
        .route("/api/capabilities/fusion", post(capability_fusion))
        .route("/api/notes/{board}", get(notes))
        .route("/api/notes/{board}/undo", post(undo_note))
        .route("/api/notes/{board}/{id}", put(put_note).delete(delete_note))
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

/// `?bytes=N`: how much of the file's end to return.
#[derive(Debug, Deserialize)]
struct TailQuery {
    bytes: Option<u64>,
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

/// `?at=…&issue=…`: a trigger, as the journal stamped it.
#[derive(Debug, Deserialize)]
struct LocateQuery {
    at: String,
    issue: Option<u64>,
}

/// The run a trigger started, so its row leads to its logs.
#[derive(Debug, Serialize)]
struct Located {
    run: Option<String>,
}

async fn locate_run(
    State(state): State<AppState>,
    Path(workflow): Path<String>,
    Query(query): Query<LocateQuery>,
) -> Response {
    if !is_workflow(&workflow) {
        return not_found("no such workflow");
    }
    json(&Located {
        run: run_of(&state.traces.runs(&workflow), query.issue, &query.at),
    })
}

/// `?stage=…`: the stage a status was of, to tell two lines apart.
#[derive(Debug, Deserialize)]
struct LineOfQuery {
    stage: Option<String>,
}

/// The line a run id belongs to.
#[derive(Debug, Serialize)]
struct LineOf {
    workflow: Option<String>,
}

/// The line whose logs hold `run`, so a status known only by its run id —
/// a paid session, a stop — leads to those logs.
async fn line_of(
    State(state): State<AppState>,
    Path(run): Path<String>,
    Query(query): Query<LineOfQuery>,
) -> Response {
    if !is_run_id(&run) {
        return not_found("no such run");
    }
    json(&LineOf {
        workflow: line_of_run(
            &run,
            query.stage.as_deref(),
            &blueprint::lines(),
            |workflow| state.traces.runs(workflow),
        ),
    })
}

async fn run_file(
    State(state): State<AppState>,
    Path((workflow, run, file)): Path<(String, String, String)>,
    Query(query): Query<TailQuery>,
) -> Response {
    if !is_workflow(&workflow) || !is_run_id(&run) || !TAILABLE.contains(&file.as_str()) {
        return not_found("no such trace");
    }
    let bytes = query.bytes.unwrap_or(TAIL_DEFAULT).min(TAIL_MAX);
    state
        .traces
        .tail(&workflow, &run, &file, bytes)
        .map_or_else(
            || not_found("no such trace"),
            |text| {
                (
                    [
                        (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                        (header::CACHE_CONTROL, "no-store"),
                    ],
                    text,
                )
                    .into_response()
            },
        )
}

/// A run's sessions, one per machine it went through, each named after its
/// stage and timed — what an agent's pane lists, and what a click on one
/// shows — with the run's own first and last clocks.
async fn run_stages(
    State(state): State<AppState>,
    Path((workflow, run)): Path<(String, String)>,
) -> Response {
    if !is_workflow(&workflow) || !is_run_id(&run) {
        return not_found("no such run");
    }
    let Some(run_log) = state.traces.read(&workflow, &run, "run.log") else {
        return not_found("no such run");
    };
    let session_log = state
        .traces
        .read(&workflow, &run, "session.log")
        .unwrap_or_default();
    let mut rows: Vec<_> = state
        .traces
        .ledger()
        .into_iter()
        .filter(|row| row.run == run)
        .collect();
    rows.sort_by(|a, b| a.when.cmp(&b.when));
    let ledger_stages: Vec<String> = rows.into_iter().map(|row| row.stage).collect();
    let tail = usize::try_from(TAIL_DEFAULT).unwrap_or(usize::MAX);
    let (started_at, last_at) = clock_span(&run_log);
    json(&RunStages {
        started_at,
        last_at,
        stages: stage_logs(&run_log, &session_log, &ledger_stages, tail),
    })
}

/// A run's process graph: every station of its line, as the run left it.
#[derive(Debug, Serialize)]
struct RunGraph {
    /// In belt order.
    stations: Vec<StationView>,
}

/// How many of the newest events a graph reads for its gate verdicts.
const GRAPH_EVENTS: usize = 5_000;

async fn run_graph(
    State(state): State<AppState>,
    Path((workflow, run)): Path<(String, String)>,
) -> Response {
    if !is_workflow(&workflow) || !is_run_id(&run) {
        return not_found("no such run");
    }
    let lines = blueprint::lines();
    let Some(line) = lines.iter().find(|line| line.id == workflow) else {
        return not_found("no such line");
    };
    if state.traces.read(&workflow, &run, "run.log").is_none() {
        return not_found("no such run");
    }
    let observed = observe_run(state.traces.as_ref(), line, &run);
    let at_work = observed.alive.unwrap_or(false);
    let told = state
        .events
        .as_ref()
        .and_then(|store| store.between(None, None, GRAPH_EVENTS).ok())
        .map(|events| gates::told(&events))
        .unwrap_or_default();
    json(&RunGraph {
        stations: assemble::run_stations(line, &observed, at_work, &told),
    })
}

/// An issue with its body, as the office shows it.
#[derive(Debug, Serialize)]
struct IssueBody {
    number: u64,
    title: String,
    state: String,
    labels: Vec<String>,
    body: String,
    url: String,
    blocked_by: Vec<Blocker>,
}

#[derive(Debug, Serialize)]
struct Blocker {
    number: u64,
    title: String,
    state: String,
}

async fn issue(State(state): State<AppState>, Path(number): Path<u64>) -> Response {
    let board = state.board.borrow().clone();
    let Some(board) = board else {
        return not_found("the board has not been read");
    };
    let found = board
        .roadmap
        .iter()
        .chain(
            board
                .milestones
                .iter()
                .flat_map(|m| std::iter::once(&m.issue).chain(m.tasks.iter())),
        )
        .find(|issue| issue.number == number);
    found.map_or_else(
        || not_found("no such issue on the board"),
        |issue| {
            json(&IssueBody {
                number: issue.number,
                title: issue.title.clone(),
                state: issue.state.clone(),
                labels: issue.labels.clone(),
                body: issue.body.clone(),
                url: format!("https://github.com/{}/issues/{}", board.slug, issue.number),
                blocked_by: issue
                    .blocked_by
                    .iter()
                    .map(|b| Blocker {
                        number: b.number,
                        title: b.title.clone(),
                        state: b.state.clone(),
                    })
                    .collect(),
            })
        },
    )
}

/// Whether there is someone for this desk, and whether they sit at it.
fn status_of(desk: Option<&Desk>) -> Status {
    desk.map_or_else(
        || Status {
            available: false,
            live: false,
            command: String::new(),
        },
        |desk| Status {
            available: true,
            live: desk.is_live(),
            command: desk.command(),
        },
    )
}

/// Whether there is a steward, and whether they are at the desk.
async fn steward(State(state): State<AppState>) -> Response {
    json(&status_of(state.desk.as_ref()))
}

/// Whether there is a doctor, and whether they are in.
async fn doctor(State(state): State<AppState>) -> Response {
    json(&status_of(state.doctor.as_ref()))
}

/// `?from=…&to=…`: two trace clocks, either left out.
#[derive(Debug, Deserialize)]
struct PeriodQuery {
    from: Option<String>,
    to: Option<String>,
}

/// How much of `watch.log` a period reads: all of a long run's journal.
const HISTORY_BYTES: u64 = 32 * 1024 * 1024;

/// The control room over a chosen period, recomputed from the whole traces:
/// the ledger, the stops, and the watch journal.
async fn period(State(state): State<AppState>, Query(query): Query<PeriodQuery>) -> Response {
    let range = match Range::parse(query.from.as_deref(), query.to.as_deref()) {
        Ok(range) => range,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    // The side of a task, from the board as last read — `None` for a task
    // the board no longer lists, never guessed.
    let sides: std::collections::HashMap<String, bool> = state
        .board
        .borrow()
        .as_ref()
        .map(|board| {
            board
                .milestones
                .iter()
                .flat_map(|m| m.tasks.iter())
                .map(|task| {
                    (
                        task.number.to_string(),
                        task.labels
                            .iter()
                            .any(|l| l == harness_workflows::common::labels::DATA_LAYER),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let traces = Arc::clone(&state.traces);
    let zone = state.zone.clone();
    let computed = tokio::task::spawn_blocking(move || {
        history::history(
            range,
            &traces.ledger(),
            &traces.errors(),
            &traces.watch_log(HISTORY_BYTES).unwrap_or_default(),
            |task| sides.get(task).copied(),
            &zone,
        )
    })
    .await;
    match computed {
        Ok(history) => json(&history),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// `?force=1`: the human asked to read again.
#[derive(Debug, Deserialize)]
struct LimitsQuery {
    force: Option<u8>,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Both rate limits, read now: GitHub every time (it is free), Claude when
/// the reading the page last paid for has aged (`domain::limits`). The two
/// reads run side by side, off the server's thread.
async fn rate_limits(State(state): State<AppState>, Query(query): Query<LimitsQuery>) -> Response {
    let Some(source) = state.limits.clone() else {
        return (StatusCode::NOT_FOUND, "this view reads no rate limit").into_response();
    };
    let now = unix_now();
    let kept = state.claude_read.lock().ok().and_then(|kept| kept.clone());
    let paid_at = kept
        .as_ref()
        .filter(|read| read.value.is_some())
        .map(|read| read.at);
    let pay_again = limits::claude_stale(paid_at, now, query.force == Some(1));
    let github = {
        let source = Arc::clone(&source);
        tokio::task::spawn_blocking(move || Read::of(source.github(), now))
    };
    let claude = match kept {
        Some(read) if !pay_again => read,
        _ => {
            let probed = tokio::task::spawn_blocking(move || Read::of(source.claude(), now))
                .await
                .unwrap_or_else(|e| Read::of(Err(format!("the probe broke: {e}")), now));
            if let Ok(mut kept) = state.claude_read.lock() {
                *kept = Some(probed.clone());
            }
            probed
        }
    };
    let github = github
        .await
        .unwrap_or_else(|e| Read::of(Err(format!("the read broke: {e}")), now));
    json(&Report { claude, github })
}

/// Whether the page has a hand on the plant, and the watch that runs.
async fn plant_status(State(state): State<AppState>) -> Response {
    json(&plant::Status {
        available: state.plant.is_some(),
        running: state
            .plant
            .as_ref()
            .and_then(|plant| plant::running(plant.as_ref())),
        max_lanes: plant::MAX_LANES,
    })
}

/// What a gesture did, for the page to say.
#[derive(Debug, Serialize)]
struct Done {
    message: String,
}

/// `?lanes=N`: how many agents a start runs at once at most.
#[derive(Debug, Deserialize)]
struct StartQuery {
    lanes: Option<usize>,
}

/// `start`, `soft` or `hard`: decided by `domain::plant`, carried out by
/// the port. A gesture that makes no sense now is a `409` with the reason;
/// a start asking for lanes out of bounds is a `400`.
///
/// Only the first action can fail the gesture: a hard stop's later kills
/// reach processes that may already be gone with the watch. A start counts
/// once the watch is still there after [`AppState::start_grace`] — one that
/// died on its first line (no remote, a bad flag) is a failed start, with
/// what it printed.
async fn plant_gesture(
    State(state): State<AppState>,
    Path(gesture): Path<String>,
    Query(query): Query<StartQuery>,
) -> Response {
    let Some(plant) = state.plant.as_ref() else {
        return (StatusCode::NOT_FOUND, "this view has no hand on the plant").into_response();
    };
    let Some(gesture) = Gesture::parse(&gesture) else {
        return (StatusCode::NOT_FOUND, "unknown gesture").into_response();
    };
    let lanes = match plant::lanes(query.lanes) {
        Ok(lanes) => lanes,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    let processes = plant.processes();
    let running = plant::running(plant.as_ref());
    let actions = match plant::decide(gesture, running, &processes) {
        Ok(actions) => actions,
        Err(why) => return (StatusCode::CONFLICT, why).into_response(),
    };
    let mut message = String::new();
    for (at, action) in actions.into_iter().enumerate() {
        let done = match action {
            Action::Start => plant.start(lanes).map(|pid| {
                let most = lanes.map_or_else(String::new, |n| format!(", at most {n} agents"));
                format!("watch started (pid {pid}{most})")
            }),
            Action::Send(signal, target) => plant.send(signal, target).map(|()| match gesture {
                Gesture::Soft => "soft stop asked — running tasks finish, none starts".to_string(),
                _ => "hard stop — the watch and its lanes are killed".to_string(),
            }),
        };
        if at > 0 {
            continue;
        }
        match done {
            Ok(said) => message = said,
            Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
        }
    }
    if gesture == Gesture::Start {
        tokio::time::sleep(state.start_grace).await;
        if plant::running(plant.as_ref()).is_none() {
            let printed = plant.output_tail();
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "the watch exited as soon as it started{}",
                    if printed.is_empty() {
                        String::new()
                    } else {
                        format!(":\n{printed}")
                    }
                ),
            )
                .into_response();
        }
    }
    json(&Done { message })
}

/// What a page reads of the janitor when the view runs without him.
const NO_JANITOR: &str = "no janitor: the view runs with --no-janitor";

/// The janitor's last weighing, last sweep, settings and what they are doing.
async fn janitor(State(state): State<AppState>) -> Response {
    state.janitor.as_ref().map_or_else(
        || json(&serde_json::json!({ "available": false })),
        |janitor| json(&janitor.status()),
    )
}

/// Weighs the yard now; answers once the walk is done.
async fn janitor_diagnose(State(state): State<AppState>) -> Response {
    let Some(janitor) = state.janitor.as_ref() else {
        return (StatusCode::NOT_FOUND, NO_JANITOR).into_response();
    };
    janitor.diagnose().await;
    json(&janitor.status())
}

/// Sweeps the yard now — a fresh weighing first, so nothing goes on an old
/// one — and answers with what went.
async fn janitor_sweep(State(state): State<AppState>) -> Response {
    let Some(janitor) = state.janitor.as_ref() else {
        return (StatusCode::NOT_FOUND, NO_JANITOR).into_response();
    };
    janitor.sweep(false).await;
    json(&janitor.status())
}

/// New settings, as JSON: the limit, the threshold, whether to sweep on their
/// own and whether strays go. Out of bounds is a `400` with the reason.
async fn janitor_settings(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(janitor) = state.janitor.as_ref() else {
        return (StatusCode::NOT_FOUND, NO_JANITOR).into_response();
    };
    let settings = match Settings::parse(&String::from_utf8_lossy(&body)) {
        Ok(settings) => settings,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    match janitor.set(settings) {
        Ok(status) => json(&status),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// What the data pane reads: one version merged, and which one it is.
#[derive(Serialize)]
struct DataModelReply<'a> {
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
struct DataModelQuery {
    branch: Option<String>,
    dir: Option<String>,
    /// `1` after a build: the archive and the live files read again.
    fresh: Option<String>,
}

async fn data_model(
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
    match desk.model(wanted).await {
        Ok(shown) => {
            reply.branch = shown.branch;
            reply.dir = shown.dir;
            reply.model = Some(&shown.model);
            json(&reply)
        }
        Err(why) => {
            reply.error = Some(why);
            json(&reply)
        }
    }
}

/// Who uses each table and field of the version shown.
#[derive(Serialize)]
struct UsedByReply {
    /// The branch the manifests were read on.
    branch: String,
    /// By `table:<name>` and `field:<table>.<name>`.
    used_by: UsedBy,
    /// Why there is nothing: no manifests read here.
    error: Option<String>,
}

/// The units that use the tables and fields of the version asked, read
/// from the manifests of its branch.
async fn data_used_by(
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
struct VersionsReply<'a> {
    enabled: bool,
    #[serde(flatten)]
    versions: &'a Versions,
}

async fn data_versions(State(state): State<AppState>) -> Response {
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
struct DivergenceQuery {
    branch: String,
    dir: Option<String>,
}

/// The diagnosis the watch recorded for a branch's diverging version.
fn divergence_of(
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

async fn data_divergence(
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
struct BuildAsk {
    what: String,
    #[serde(default)]
    branch: String,
}

/// Starts the harness command that builds a branch's missing file: a
/// snapshot for its schema, a draft (a pull request) for its model.
async fn data_build(State(state): State<AppState>, body: Bytes) -> Response {
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
async fn data_builds(State(state): State<AppState>) -> Response {
    json(
        &state
            .builds
            .as_ref()
            .map(|b| b.builds())
            .unwrap_or_default(),
    )
}

/// A build's whole output, as it stands.
async fn data_build_log(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    state
        .builds
        .as_ref()
        .and_then(|b| b.log(&id))
        .map_or_else(|| not_found("no such build"), IntoResponse::into_response)
}

/// What the human sends to open an issue from a divergence.
#[derive(Debug, Deserialize)]
struct IssueAsk {
    branch: String,
    dir: String,
    resolution: String,
}

/// Opens the issue a human decided on: their resolution first, the state of
/// affairs after. Its labels start the pipeline.
async fn data_issue(State(state): State<AppState>, body: Bytes) -> Response {
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

/// What `/api/capabilities` is asked.
#[derive(Deserialize)]
struct CatalogueAsk {
    /// Read the product again, rather than the kept reading.
    #[serde(default)]
    fresh: bool,
}

/// The capabilities list, from the kept reading and the latest board.
async fn catalogue_of(state: &AppState, fresh: bool) -> Result<Catalogue, (StatusCode, String)> {
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
    ))
}

/// Every Capability and `DataCapability` of the product.
async fn capability_list(
    State(state): State<AppState>,
    Query(ask): Query<CatalogueAsk>,
) -> Response {
    match catalogue_of(&state, ask.fresh).await {
        Ok(catalogue) => json(&catalogue),
        Err(refused) => refused.into_response(),
    }
}

/// The two units a fusion test compares.
#[derive(Deserialize)]
struct FusionAsk {
    a: String,
    b: String,
    /// Ask the model to word the answer; without it, the tooling's alone.
    #[serde(default)]
    ai: bool,
}

/// Could two units be one? The tooling compares; the model, if asked,
/// words the answer from the comparison alone.
async fn capability_fusion(State(state): State<AppState>, body: Bytes) -> Response {
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
    let catalogue = match catalogue_of(&state, false).await {
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

/// A board's pins, the deleted ones left out.
async fn notes(State(state): State<AppState>, Path(board): Path<String>) -> Response {
    if !annotations::is_board(&board) {
        return not_found("no such board");
    }
    match state.pins.notes(&board) {
        Ok(kept) => json(
            &kept
                .into_iter()
                .filter(|k| k.deleted_at.is_none())
                .map(|k| k.note)
                .collect::<Vec<_>>(),
        ),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// The clock, as the traces write it.
fn stamp() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Keeps one pin — new, moved, rewritten, its kind changed.
async fn put_note(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    if !annotations::is_board(&board) || !annotations::is_id(&id) {
        return not_found("no such pin");
    }
    let note = match serde_json::from_slice::<annotations::Note>(&body) {
        Ok(note) if note.id == id => note,
        Ok(_) => {
            return (StatusCode::BAD_REQUEST, "the pin's id is not the path's").into_response();
        }
        Err(e) => return (StatusCode::BAD_REQUEST, format!("not a pin: {e}")).into_response(),
    };
    let note = match annotations::checked(note) {
        Ok(note) => note,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    match state.pins.put(&board, &note) {
        Ok(()) => json(&note),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// Deletes a pin — only marked, so ctrl+z brings it back; the deleted pins
/// past the undo depth are destroyed, conversation and all.
async fn delete_note(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
) -> Response {
    if !annotations::is_board(&board) || !annotations::is_id(&id) {
        return not_found("no such pin");
    }
    // An open session's turns are kept with the pin before it goes.
    if let Some(desks) = &state.note_desks {
        let _ = desks.close(&board, &id, false).await;
    }
    let done = state
        .pins
        .mark_deleted(&board, &id, Some(&stamp()))
        .and_then(|()| state.pins.notes(&board))
        .and_then(|kept| {
            let deleted: Vec<(String, String)> = kept
                .into_iter()
                .filter_map(|k| Some((k.note.id, k.deleted_at?)))
                .collect();
            annotations::beyond_undo(&deleted)
                .iter()
                .try_for_each(|gone| state.pins.destroy(&board, gone))
        });
    match done {
        Ok(()) => json(&serde_json::json!({ "deleted": id })),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// Brings back the pin deleted last.
async fn undo_note(State(state): State<AppState>, Path(board): Path<String>) -> Response {
    if !annotations::is_board(&board) {
        return not_found("no such board");
    }
    let last = match state.pins.notes(&board) {
        Ok(kept) => kept
            .into_iter()
            .filter(|k| k.deleted_at.is_some())
            .max_by(|a, b| a.deleted_at.cmp(&b.deleted_at)),
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    };
    let Some(last) = last else {
        return not_found("nothing to bring back");
    };
    match state.pins.mark_deleted(&board, &last.note.id, None) {
        Ok(()) => json(&last.note),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// What the page sends to open a pin's session: what it shows of the pin's
/// target, the branch the pin belongs to and the version shown.
#[derive(Debug, Default, Deserialize)]
struct SessionAsk {
    #[serde(default)]
    context: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    dir: String,
}

/// The most of a schema a session is handed: a few hundred tables.
const SCHEMA_CHARS: usize = 60_000;

/// Why a pin has no session here.
const NO_SESSIONS: &str = "no AI session here: the view runs with --no-steward";

/// Opens a pin's session — or finds it open — and says what runs it.
async fn open_session(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let Some(desks) = state.note_desks.clone() else {
        return not_found(NO_SESSIONS);
    };
    let ask: SessionAsk = serde_json::from_slice(&body).unwrap_or_default();
    // The page's context is a description, never a megabyte.
    let context: String = ask.context.chars().take(20_000).collect();
    let schema = match &state.data {
        Some(data) if !ask.branch.is_empty() => data
            .schema(Wanted {
                branch: Some(ask.branch.clone()),
                dir: (!ask.dir.is_empty()).then(|| ask.dir.clone()),
                fresh: false,
            })
            .await
            .map(|text| text.chars().take(SCHEMA_CHARS).collect())
            .unwrap_or_default(),
        _ => String::new(),
    };
    // A feature branch: the milestone it belongs to, as the board has it.
    let milestone = data_versions::milestone_number(&ask.branch)
        .and_then(|number| {
            let board = state.board.borrow().clone()?;
            board
                .milestones
                .iter()
                .find(|m| m.issue.number == number)
                .map(note_session::milestone_words)
        })
        .unwrap_or_default();
    let brief = Brief {
        context,
        branch: ask.branch,
        schema,
        milestone,
    };
    match desks.open(&board, &id, brief).await {
        Ok(command) => json(&serde_json::json!({ "available": true, "command": command })),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

/// A pin's session's terminal.
async fn session_term(
    ws: WebSocketUpgrade,
    Path((board, id)): Path<(String, String)>,
    Query(query): Query<TermQuery>,
    State(state): State<AppState>,
) -> Response {
    let desk = state
        .note_desks
        .as_ref()
        .and_then(|desks| desks.desk(&board, &id));
    term(ws, &query, desk, "this pin's session is not open")
}

/// Closes a pin's session; its turns join the pin's conversation.
async fn close_session(
    State(state): State<AppState>,
    Path((board, id)): Path<(String, String)>,
) -> Response {
    let Some(desks) = state.note_desks.clone() else {
        return not_found(NO_SESSIONS);
    };
    match desks.close(&board, &id, true).await {
        Ok(turns) => json(&serde_json::json!({ "turns": turns })),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

/// Turns a brainstorm into a note (its conclusion) or an asking (its
/// request), the AI writing either.
async fn turn_note(
    State(state): State<AppState>,
    Path((board, id, kind)): Path<(String, String, String)>,
) -> Response {
    let kind = match kind.as_str() {
        "note" => annotations::Kind::Note,
        "asking" => annotations::Kind::Asking,
        _ => return not_found("a brainstorm turns into a note or an asking"),
    };
    let Some(desks) = state.note_desks.clone() else {
        return not_found(NO_SESSIONS);
    };
    match desks.turn_into(&board, &id, kind).await {
        Ok(note) => json(&note),
        Err(why) => (StatusCode::BAD_REQUEST, why).into_response(),
    }
}

/// How long the doctor gets to answer before a diagnosis is given up on.
const DIAGNOSIS_PATIENCE: Duration = Duration::from_mins(20);

/// How long a freshly started doctor gets to show its prompt before the
/// question is typed; typed sooner, the keystrokes land in a program that is
/// not yet reading them as text.
const DOCTOR_WARM_UP: Duration = Duration::from_secs(5);

/// A pause between the pasted question and the Enter that sends it.
const PASTE_SETTLE: Duration = Duration::from_millis(400);

/// How much of the doctor's recent output is kept to hear a mark in: a mark
/// is one line, repainted a few times at most.
const EARSHOT: usize = 32 * 1024;

/// The diagnoses asked so far, by `workflow/run`.
async fn diagnoses(State(state): State<AppState>) -> Response {
    let map = state
        .diagnoses
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default();
    json(&map)
}

/// Asks the doctor what happened to one run: its logs' tails are pasted into
/// the doctor's terminal as a question, and a listener marks the diagnosis
/// done when a reply ends with the run's mark.
async fn diagnose(
    State(state): State<AppState>,
    Path((workflow, run)): Path<(String, String)>,
) -> Response {
    let Some(desk) = state.doctor.clone() else {
        return not_found("no doctor: the view runs with --no-doctor");
    };
    if !is_workflow(&workflow) || !is_run_id(&run) {
        return not_found("no such run");
    }
    let Some(run_log) = state
        .traces
        .tail(&workflow, &run, "run.log", doctor::RUN_LOG_BYTES)
    else {
        return not_found("no such trace");
    };
    let session_log = state
        .traces
        .tail(&workflow, &run, "session.log", doctor::SESSION_LOG_BYTES)
        .unwrap_or_default();
    let key = format!("{workflow}/{run}");
    let since = jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    // Already asked and not answered: the same question is not typed twice.
    let pending = state
        .diagnoses
        .lock()
        .ok()
        .and_then(|map| match map.get(&key) {
            Some(running @ Diagnosis::Running { .. }) => Some(running.clone()),
            _ => None,
        });
    if let Some(running) = pending {
        return json(&running);
    }
    // One question at a time: held until Enter is sent.
    let _turn = state.asking.lock().await;
    // Listening before the question is typed, so the answer's first bytes are
    // not missed; and before the start, so a program that dies at once is heard.
    let ears = desk.watch();
    let question = doctor::diagnosis(&workflow, &run, &run_log, &session_log);
    if let Err(why) = type_to_doctor(&desk, &question).await {
        return (StatusCode::SERVICE_UNAVAILABLE, why).into_response();
    }
    let asked = Diagnosis::Running {
        since: since.clone(),
    };
    if let Ok(mut map) = state.diagnoses.lock() {
        map.insert(key.clone(), asked.clone());
    }
    tokio::spawn(listen(ears, run, key, since, Arc::clone(&state.diagnoses)));
    json(&asked)
}

/// Types a question into the doctor's terminal — summoning the doctor first
/// when nobody sits there — as a bracketed paste, then Enter. The caller holds
/// `AppState::asking`: two questions at once would land in one input line.
async fn type_to_doctor(desk: &Desk, question: &str) -> Result<(), String> {
    let was_live = desk.is_live();
    desk.summon(TERM_DEFAULT.0, TERM_DEFAULT.1)?;
    if !was_live {
        tokio::time::sleep(DOCTOR_WARM_UP).await;
    }
    // A bracketed paste: the newlines inside stay text, Enter comes after.
    let paste = format!("\x1b[200~{question}\x1b[201~");
    desk.write(paste.as_bytes())?;
    tokio::time::sleep(PASTE_SETTLE).await;
    desk.write(b"\r")
}

/// What the page sends when the human asks the doctor about a failed action.
#[derive(Debug, Deserialize)]
struct ActionAsk {
    action: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    log: String,
}

/// Asks the doctor about an action that failed on the page: what it was, its
/// error, its log. The page then opens the doctor's pane.
async fn ask_doctor(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(desk) = state.doctor.clone() else {
        return (
            StatusCode::NOT_FOUND,
            "no doctor: the view runs with --no-doctor",
        )
            .into_response();
    };
    let ask = match serde_json::from_slice::<ActionAsk>(&body) {
        Ok(ask) => ask,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("not a question: {e}")).into_response(),
    };
    let action: String = ask.action.chars().take(300).collect();
    let error: String = ask.error.chars().take(2_000).collect();
    let question = doctor::about_action(&action, &error, &ask.log);
    let _turn = state.asking.lock().await;
    match type_to_doctor(&desk, &question).await {
        Ok(()) => json(&serde_json::json!({ "asked": true })),
        Err(why) => (StatusCode::SERVICE_UNAVAILABLE, why).into_response(),
    }
}

/// Hears the doctor's output until a reply ends with the run's mark, the
/// program ends, or patience runs out; then writes the diagnosis down.
async fn listen(
    mut ears: broadcast::Receiver<Vec<u8>>,
    run: String,
    key: String,
    since: String,
    diagnoses: Diagnoses,
) {
    let mut heard: Vec<u8> = Vec::new();
    let deadline = tokio::time::Instant::now() + DIAGNOSIS_PATIENCE;
    let clock = || {
        jiff::Timestamp::now()
            .strftime("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    };
    let outcome = loop {
        match tokio::time::timeout_at(deadline, ears.recv()).await {
            Ok(Ok(bytes)) => {
                heard.extend(bytes);
                let overflow = heard.len().saturating_sub(EARSHOT);
                heard.drain(..overflow);
                if doctor::diagnosed(&heard, &run) {
                    break Diagnosis::Done { since, at: clock() };
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => {
                break Diagnosis::Lost { since, at: clock() };
            }
        }
    };
    if let Ok(mut map) = diagnoses.lock()
        && matches!(map.get(&key), Some(Diagnosis::Running { .. }))
    {
        map.insert(key, outcome);
    }
}

/// `?cols=N&rows=N`: the size of the pane's terminal.
#[derive(Debug, Deserialize)]
struct TermQuery {
    cols: Option<u16>,
    rows: Option<u16>,
}

/// What the page sends as text, beside the raw keystrokes it sends as bytes.
#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Control {
    /// The pane changed size.
    Size { cols: u16, rows: u16 },
    /// Kill the program and start a fresh one.
    Restart,
}

async fn steward_term(
    ws: WebSocketUpgrade,
    Query(query): Query<TermQuery>,
    State(state): State<AppState>,
) -> Response {
    term(
        ws,
        &query,
        state.desk,
        "no steward: the view runs with --no-steward",
    )
}

async fn doctor_term(
    ws: WebSocketUpgrade,
    Query(query): Query<TermQuery>,
    State(state): State<AppState>,
) -> Response {
    term(
        ws,
        &query,
        state.doctor,
        "no doctor: the view runs with --no-doctor",
    )
}

/// Sits the visitor at this desk, or says why there is none.
fn term(ws: WebSocketUpgrade, query: &TermQuery, desk: Option<Desk>, missing: &str) -> Response {
    let Some(desk) = desk else {
        return not_found(missing);
    };
    let cols = query.cols.unwrap_or(TERM_DEFAULT.0).max(20);
    let rows = query.rows.unwrap_or(TERM_DEFAULT.1).max(5);
    ws.on_upgrade(move |socket| attend(socket, desk, cols, rows))
}

/// A note from the desk, shown dim in the terminal.
fn aside(text: &str) -> Message {
    Message::Text(format!("\r\n\x1b[2m[{text}]\x1b[0m\r\n").into())
}

/// One visit: the screen so far, then keystrokes one way and output the other
/// until either side leaves. The program outlives the visit.
async fn attend(mut socket: WebSocket, desk: Desk, mut cols: u16, mut rows: u16) {
    let (mut output, replay) = match desk.attach(cols, rows) {
        Ok(attached) => attached,
        Err(why) => {
            let _ = socket.send(aside(&why)).await;
            return;
        }
    };
    if !replay.is_empty()
        && socket
            .send(Message::Binary(Bytes::from(replay)))
            .await
            .is_err()
    {
        return;
    }
    // A fresh size makes the program redraw its screen for the visitor.
    let _ = desk.resize(cols, rows);
    loop {
        tokio::select! {
            chunk = output.recv() => match chunk {
                Ok(bytes) => {
                    if socket.send(Message::Binary(Bytes::from(bytes))).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Binary(bytes))) => {
                    if desk.write(&bytes).is_err() {
                        // The program ended; a keystroke calls it back.
                        match desk.summon(cols, rows) {
                            Ok(()) => { let _ = desk.write(&bytes); }
                            Err(why) => { let _ = socket.send(aside(&why)).await; }
                        }
                    }
                }
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<Control>(&text) {
                    Ok(Control::Size { cols: c, rows: r }) => {
                        cols = c.max(20);
                        rows = r.max(5);
                        let _ = desk.resize(cols, rows);
                    }
                    Ok(Control::Restart) => {
                        if let Err(why) = desk.restart(cols, rows) {
                            let _ = socket.send(aside(&why)).await;
                        }
                    }
                    Err(_) => {}
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
            },
        }
    }
}

/// The prompts a station sends: what runs, what waits, what was kept.
async fn station_prompts(
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
async fn edit_prompt(State(state): State<AppState>, body: Bytes) -> Response {
    match serde_json::from_slice::<Edit>(&body) {
        Ok(edit) => change_drafts(&state, |book, drafts, shelf| {
            prompts::propose(book, drafts, shelf, &edit, unix_now())
        }),
        Err(e) => (StatusCode::BAD_REQUEST, format!("not an edit: {e}")).into_response(),
    }
}

/// Proposes, for the next start, a text kept earlier.
async fn restore_prompt(State(state): State<AppState>, body: Bytes) -> Response {
    match serde_json::from_slice::<Restore>(&body) {
        Ok(restore) => change_drafts(&state, |book, drafts, shelf| {
            prompts::restore(book, drafts, shelf, &restore, unix_now())
        }),
        Err(e) => (StatusCode::BAD_REQUEST, format!("not a restore: {e}")).into_response(),
    }
}

/// Deactivates the draft waiting: it stays in the history.
async fn deactivate_prompt(State(state): State<AppState>, body: Bytes) -> Response {
    match serde_json::from_slice::<Deactivate>(&body) {
        Ok(ask) => change_drafts(&state, |book, drafts, shelf| {
            prompts::deactivate(book, drafts, shelf, &ask)
        }),
        Err(e) => (StatusCode::BAD_REQUEST, format!("not a deactivation: {e}")).into_response(),
    }
}

/// Makes a deactivated draft the one that waits again.
async fn reactivate_prompt(State(state): State<AppState>, body: Bytes) -> Response {
    match serde_json::from_slice::<OnShelf>(&body) {
        Ok(pick) => change_drafts(&state, |book, drafts, shelf| {
            prompts::reactivate(book, drafts, shelf, &pick, unix_now())
        }),
        Err(e) => (StatusCode::BAD_REQUEST, format!("not a reactivation: {e}")).into_response(),
    }
}

/// Deletes a deactivated draft from the history.
async fn discard_prompt(State(state): State<AppState>, body: Bytes) -> Response {
    match serde_json::from_slice::<OnShelf>(&body) {
        Ok(pick) => change_drafts(&state, |book, drafts, shelf| {
            prompts::discard(book, drafts, shelf, &pick)
        }),
        Err(e) => (StatusCode::BAD_REQUEST, format!("not a deletion: {e}")).into_response(),
    }
}

/// Read, change, write the drafts and the shelf, one page at a time;
/// answers the prompt as it now stands. The book is only read.
fn change_drafts(
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
struct TrialRequest {
    key: String,
    version: String,
    text: String,
    workflow: String,
    run: String,
}

/// Starts a dry-run: one at a time, on a run of the line the prompt is sent on.
async fn start_trial(State(state): State<AppState>, body: Bytes) -> Response {
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
struct TrialsQuery {
    key: String,
}

/// One prompt's last dry-runs, newest first.
async fn trials(State(state): State<AppState>, Query(query): Query<TrialsQuery>) -> Response {
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

/// Where a restart the human asked for stands.
#[derive(Debug, Clone, Serialize)]
pub struct Restart {
    /// When it was asked, Unix seconds.
    pub since: u64,
    /// `draining`, `started` or `failed`.
    pub state: &'static str,
    /// What happened, in a sentence.
    pub message: String,
}

/// What waits for the next start, and the restart under way.
async fn pending(State(state): State<AppState>) -> Response {
    let changes = state
        .prompts
        .drafts()
        .map(|drafts| prompts::pending(&drafts))
        .unwrap_or_default();
    let restart = state.restart.lock().ok().and_then(|r| r.clone());
    json(&serde_json::json!({ "changes": changes, "restart": restart }))
}

/// Restarts the factory so the changes waiting apply: a soft stop, then a
/// start as soon as the watch has drained — at once when none runs.
async fn restart(State(state): State<AppState>, Query(query): Query<StartQuery>) -> Response {
    let Some(plant) = state.plant.clone() else {
        return (StatusCode::NOT_FOUND, "this view has no hand on the plant").into_response();
    };
    let lanes = match plant::lanes(query.lanes) {
        Ok(lanes) => lanes,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    let Ok(mut under_way) = state.restart.lock() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "restart lock poisoned").into_response();
    };
    if under_way.as_ref().is_some_and(|r| r.state == "draining") {
        return (
            StatusCode::CONFLICT,
            "a restart is already waiting for the watch to drain",
        )
            .into_response();
    }
    let since = unix_now();
    let running = plant::running(plant.as_ref());
    if running.is_some() {
        let processes = plant.processes();
        if let Ok(actions) = plant::decide(Gesture::Soft, running, &processes) {
            for action in actions {
                if let Action::Send(signal, target) = action
                    && let Err(why) = plant.send(signal, target)
                {
                    return (StatusCode::INTERNAL_SERVER_ERROR, why).into_response();
                }
            }
        }
        *under_way = Some(Restart {
            since,
            state: "draining",
            message: "soft stop sent — the factory starts again once the running tasks finish"
                .to_string(),
        });
        drop(under_way);
        let shared = Arc::clone(&state.restart);
        let poll = state.restart_poll;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(poll).await;
                if plant::running(plant.as_ref()).is_none() {
                    break;
                }
            }
            let started = plant.start(lanes);
            if let Ok(mut slot) = shared.lock() {
                *slot = Some(match started {
                    Ok(pid) => Restart {
                        since,
                        state: "started",
                        message: format!("restarted (pid {pid}) — the changes apply now"),
                    },
                    Err(why) => Restart {
                        since,
                        state: "failed",
                        message: why,
                    },
                });
            }
        });
        return json(&Done {
            message: "soft stop sent — the factory restarts once the running tasks finish"
                .to_string(),
        });
    }
    let started = plant.start(lanes);
    let (said, outcome) = match started {
        Ok(pid) => (
            Restart {
                since,
                state: "started",
                message: format!("started (pid {pid}) — the changes apply now"),
            },
            Ok(()),
        ),
        Err(why) => (
            Restart {
                since,
                state: "failed",
                message: why.clone(),
            },
            Err(why),
        ),
    };
    let message = said.message.clone();
    *under_way = Some(said);
    match outcome {
        Ok(()) => json(&Done { message }),
        Err(why) => (StatusCode::INTERNAL_SERVER_ERROR, why).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::fake::Echoing;
    use crate::domain::assemble::{self, Inputs};
    use crate::domain::blueprint;
    use crate::domain::observe::Observed;
    use crate::domain::observe::fake::Shelf;
    use crate::domain::plant::fake::Switch;
    use crate::domain::snapshot::Project;
    use crate::ports::Process;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt as _;

    fn state(shelf: Shelf, desk: Option<Desk>) -> AppState {
        let lines = blueprint::lines();
        let project = Project {
            name: "p".to_string(),
            slug: String::new(),
            url: String::new(),
            integration_branch: "main_agent".to_string(),
        };
        let observed = Observed::default();
        let told = crate::domain::gates::Told::default();
        let snap = assemble::snapshot(&Inputs {
            observed: &observed,
            board: None,
            project: &project,
            lines: &lines,
            now: 0,
            demo: false,
            told: &told,
            zone: &jiff::tz::TimeZone::UTC,
        });
        let (_, snapshot) = watch::channel(Arc::new(snap));
        let (_, board) = watch::channel(None);
        AppState {
            snapshot,
            board,
            traces: Arc::new(shelf),
            static_dir: None,
            render_dir: PathBuf::from("/nonexistent/render"),
            desk,
            doctor: None,
            diagnoses: Arc::default(),
            asking: Arc::default(),
            plant: None,
            start_grace: Duration::ZERO,
            limits: None,
            claude_read: Arc::default(),
            events: None,
            janitor: None,
            data: None,
            pins: Arc::new(Pins::default()),
            note_desks: None,
            builds: None,
            integration_branch: "main_agent".to_string(),
            prompts: Arc::new(Book::default()),
            prompts_lock: Arc::default(),
            trials: Some(Arc::new(Bench::default())),
            restart: Arc::default(),
            restart_poll: Duration::from_millis(5),
            units: None,
            zone: jiff::tz::TimeZone::UTC,
        }
    }

    /// A prompt book and its drafts, in memory.
    #[derive(Default)]
    struct Book(
        std::sync::Mutex<(
            harness_core::domain::prompt_book::PromptBook,
            harness_core::domain::prompt_book::Drafts,
            harness_core::domain::prompt_book::Shelf,
        )>,
    );

    impl Book {
        /// The factory starts: the drafts become what runs.
        fn start(&self) {
            let mut both = self.0.lock().expect("book");
            let drafts = std::mem::take(&mut both.1);
            both.0
                .apply(&drafts, harness_workflows::prompts::default_of, 9);
        }
    }

    impl Prompts for Book {
        fn book(&self) -> Result<harness_core::domain::prompt_book::PromptBook, String> {
            Ok(self.0.lock().expect("book").0.clone())
        }

        fn drafts(&self) -> Result<harness_core::domain::prompt_book::Drafts, String> {
            Ok(self.0.lock().expect("book").1.clone())
        }

        fn keep_drafts(
            &self,
            drafts: &harness_core::domain::prompt_book::Drafts,
        ) -> Result<(), String> {
            self.0.lock().expect("book").1 = drafts.clone();
            Ok(())
        }

        fn shelf(&self) -> Result<harness_core::domain::prompt_book::Shelf, String> {
            Ok(self.0.lock().expect("book").2.clone())
        }

        fn keep_shelf(
            &self,
            deactivated: &harness_core::domain::prompt_book::Shelf,
        ) -> Result<(), String> {
            self.0.lock().expect("book").2 = deactivated.clone();
            Ok(())
        }
    }

    /// Trials that never run: what was asked is kept, and reported running.
    #[derive(Default)]
    struct Bench(std::sync::Mutex<Vec<TrialAsk>>);

    impl Trials for Bench {
        fn start(&self, ask: &TrialAsk) -> Result<(), String> {
            self.0.lock().expect("bench").push(ask.clone());
            Ok(())
        }

        fn reports(&self) -> Vec<(String, harness_core::domain::prompt_book::TrialReport)> {
            self.0
                .lock()
                .expect("bench")
                .iter()
                .map(|ask| {
                    (
                        ask.id.clone(),
                        harness_core::domain::prompt_book::TrialReport {
                            key: ask.key.clone(),
                            ..Default::default()
                        },
                    )
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn an_edit_waits_for_the_start_and_a_revert_is_kept_once_it_ran() {
        let book = Arc::new(Book::default());
        let mut state = state(Shelf::default(), None);
        state.prompts = Arc::clone(&book) as Arc<dyn Prompts>;
        let (status, shown) = send(state.clone(), "GET", "/api/prompts/split/slice", "").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            shown.contains("\"edited\":false") && shown.contains("\"draft\":null"),
            "{shown}"
        );

        let edit = r#"{"key":"split/slice","text":"Slice #{num}"}"#;
        let (status, saved) = send(state.clone(), "POST", "/api/prompts/edit", edit).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert!(
            saved.contains("\"edited\":false"),
            "nothing runs it before a start: {saved}"
        );
        let (_, waiting) = send(state.clone(), "GET", "/api/pending", "").await;
        assert!(
            waiting.contains("\"key\":\"split/slice\"") && waiting.contains("\"kind\":\"edit\""),
            "{waiting}"
        );

        book.start();
        let (_, shown) = send(state.clone(), "GET", "/api/prompts/split/slice", "").await;
        assert!(
            shown.contains("\"running\":\"Slice #{num}\"") && shown.contains("\"edited\":true"),
            "{shown}"
        );
        let (_, waiting) = send(state.clone(), "GET", "/api/pending", "").await;
        assert!(waiting.contains("\"changes\":[]"), "{waiting}");

        let back = r#"{"key":"split/slice","from":"default"}"#;
        let (status, _) = send(state.clone(), "POST", "/api/prompts/restore", back).await;
        assert_eq!(status, StatusCode::OK);
        book.start();
        let (_, shown) = send(state.clone(), "GET", "/api/prompts/split/slice", "").await;
        assert!(
            shown.contains("\"reverted\":[{\"text\":\"Slice #{num}\""),
            "{shown}"
        );

        let far = r#"{"key":"split/slice","from":"history","index":4}"#;
        let (status, _) = send(state.clone(), "POST", "/api/prompts/restore", far).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = send(
            state.clone(),
            "POST",
            "/api/prompts/edit",
            r#"{"key":"nope","text":"x"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, none) = send(state, "GET", "/api/prompts/split/publish", "").await;
        assert_eq!(none, "[]");
    }

    #[tokio::test]
    async fn a_dry_run_starts_once_on_its_own_line_with_the_station_s_model() {
        let state = state(Shelf::default(), None);
        let ask = |run: &str, line: &str| {
            format!(
                r#"{{"key":"split/slice","version":"draft","text":"Slice #{{num}}","workflow":"{line}","run":"{run}"}}"#
            )
        };
        let (status, why) = send(
            state.clone(),
            "POST",
            "/api/prompts/trial",
            &ask("20261009-120000-12", "planner"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}");
        let (status, started) = send(
            state.clone(),
            "POST",
            "/api/prompts/trial",
            &ask("20261009-120000-12", "split"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{started}");
        let (status, _) = send(
            state.clone(),
            "POST",
            "/api/prompts/trial",
            &ask("20261009-120000-12", "split"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "one at a time");
        let (_, listed) = send(state, "GET", "/api/prompts/trials?key=split/slice", "").await;
        assert!(
            listed.contains("\"key\":\"split/slice\"") && listed.contains("\"state\":\"running\""),
            "{listed}"
        );
    }

    #[tokio::test]
    async fn a_restart_drains_then_starts_and_says_so() {
        let mut state = state(Shelf::default(), None);
        let switch = Arc::new(Switch::default());
        switch.start(None).expect("running");
        state.plant = Some(Arc::clone(&switch) as Arc<dyn Plant>);
        let (status, said) = send(state.clone(), "POST", "/api/restart", "").await;
        assert_eq!(status, StatusCode::OK, "{said}");
        assert!(said.contains("soft stop"), "{said}");
        let (status, _) = send(state.clone(), "POST", "/api/restart", "").await;
        assert_eq!(status, StatusCode::CONFLICT);
        *switch.started.lock().expect("table") = None;
        for _ in 0..100 {
            let (_, waiting) = send(state.clone(), "GET", "/api/pending", "").await;
            if waiting.contains("\"state\":\"started\"") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the factory never started again");
    }

    /// A pins' store in memory.
    #[derive(Default)]
    struct Pins(std::sync::Mutex<Vec<(String, crate::ports::KeptNote)>>);

    // A test fake: each method holds its lock for the whole of its few lines.
    #[allow(clippy::significant_drop_tightening)]
    impl NoteStore for Pins {
        fn notes(&self, board: &str) -> Result<Vec<crate::ports::KeptNote>, String> {
            let kept = self.0.lock().map_err(|e| e.to_string())?;
            Ok(kept
                .iter()
                .filter(|(b, _)| b == board)
                .map(|(_, k)| k.clone())
                .collect())
        }
        fn put(&self, board: &str, note: &annotations::Note) -> Result<(), String> {
            let mut kept = self.0.lock().map_err(|e| e.to_string())?;
            match kept
                .iter_mut()
                .find(|(b, k)| b == board && k.note.id == note.id)
            {
                Some((_, k)) => k.note = note.clone(),
                None => kept.push((
                    board.to_string(),
                    crate::ports::KeptNote {
                        note: note.clone(),
                        deleted_at: None,
                    },
                )),
            }
            Ok(())
        }
        fn mark_deleted(&self, board: &str, id: &str, at: Option<&str>) -> Result<(), String> {
            let mut kept = self.0.lock().map_err(|e| e.to_string())?;
            if let Some((_, k)) = kept.iter_mut().find(|(b, k)| b == board && k.note.id == id) {
                k.deleted_at = at.map(str::to_string);
            }
            Ok(())
        }
        fn destroy(&self, board: &str, id: &str) -> Result<(), String> {
            let mut kept = self.0.lock().map_err(|e| e.to_string())?;
            kept.retain(|(b, k)| !(b == board && k.note.id == id));
            Ok(())
        }
        fn conversation(&self, _board: &str, _id: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn save_conversation(
            &self,
            _board: &str,
            _id: &str,
            _json: &str,
            _turns: usize,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    async fn send(state: AppState, method: &str, uri: &str, body: &str) -> (StatusCode, String) {
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn a_pin_is_kept_deleted_brought_back_and_destroyed_past_ten() {
        let state = state(Shelf::default(), None);
        let pin = |id: &str, kind: &str| {
            format!(
                r#"{{"id":"{id}","target":"table:campaign","dx":3,"dy":4,"x":0,"y":0,"text":"why uuid?","at":"t","kind":"{kind}"}}"#
            )
        };
        let (status, _) = send(
            state.clone(),
            "PUT",
            "/api/notes/data-model/p1",
            &pin("p1", "brainstorm"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(
            state.clone(),
            "PUT",
            "/api/notes/data-model/p2",
            &pin("p1", "note"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "the path names the pin");
        let (_, back) = send(state.clone(), "GET", "/api/notes/data-model", "").await;
        assert!(back.contains(r#""kind":"brainstorm""#), "{back}");

        let (status, _) = send(state.clone(), "DELETE", "/api/notes/data-model/p1", "").await;
        assert_eq!(status, StatusCode::OK);
        let (_, back) = send(state.clone(), "GET", "/api/notes/data-model", "").await;
        assert_eq!(back, "[]");
        let (status, restored) =
            send(state.clone(), "POST", "/api/notes/data-model/undo", "").await;
        assert_eq!(status, StatusCode::OK);
        assert!(restored.contains("why uuid?"));
        let (status, _) = send(state.clone(), "POST", "/api/notes/data-model/undo", "").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "nothing left to bring back");

        for k in 0..12 {
            let id = format!("q{k:02}");
            send(
                state.clone(),
                "PUT",
                &format!("/api/notes/data-model/{id}"),
                &pin(&id, "note"),
            )
            .await;
            send(
                state.clone(),
                "DELETE",
                &format!("/api/notes/data-model/{id}"),
                "",
            )
            .await;
        }
        let kept = state.pins.notes("data-model").expect("read");
        assert_eq!(
            kept.iter().filter(|k| k.deleted_at.is_some()).count(),
            annotations::UNDO_DEPTH,
            "only the last ten deleted pins can come back"
        );
        let (status, _) = send(
            state.clone(),
            "POST",
            "/api/notes/data-model/p1/session",
            "{}",
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "no sessions without terminals"
        );
        let (status, _) = send(state, "GET", "/api/notes/..%2Fetc", "").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_data_model_says_when_nothing_reads_it() {
        let (_, body) = send(state(Shelf::default(), None), "GET", "/api/data-model", "").await;
        assert_eq!(
            body,
            r#"{"enabled":false,"model":null,"branch":"","dir":"","error":null}"#
        );
        let (_, body) = send(
            state(Shelf::default(), None),
            "GET",
            "/api/data-model/versions",
            "",
        )
        .await;
        assert_eq!(body, r#"{"enabled":false,"archive":false,"branches":[]}"#);
    }

    async fn post_to(state: AppState, uri: &str) -> (StatusCode, String) {
        post_body(state, uri, "").await
    }

    async fn post_body(state: AppState, uri: &str, body: &str) -> (StatusCode, String) {
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn watch(pid: u32) -> Vec<Process> {
        vec![Process {
            pid,
            parent: 1,
            group: pid,
            command: "./target/debug/harness watch".to_string(),
        }]
    }

    fn with_plant(processes: Vec<Process>) -> (AppState, Arc<Switch>) {
        let switch = Arc::new(Switch {
            processes,
            ..Switch::default()
        });
        let mut state = state(Shelf::default(), None);
        state.plant = Some(Arc::clone(&switch) as Arc<dyn Plant>);
        (state, switch)
    }

    fn sent(switch: &Switch) -> Vec<String> {
        switch.sent.lock().expect("sent").clone()
    }

    #[tokio::test]
    async fn the_switch_starts_an_empty_plant_and_stops_a_running_one() {
        let (state, switch) = with_plant(Vec::new());
        let (status, _) = post_to(state.clone(), "/api/plant/soft").await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, body) = post_to(state.clone(), "/api/plant/start").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, _) = post_to(state, "/api/plant/soft").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(sent(&switch), ["start", "Term Process(4242)"]);

        let (state, switch) = with_plant(watch(7));
        let (status, _) = post_to(state.clone(), "/api/plant/start").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            post_to(state.clone(), "/api/plant/soft").await.0,
            StatusCode::OK
        );
        assert_eq!(
            post_to(state.clone(), "/api/plant/hard").await.0,
            StatusCode::OK
        );
        assert_eq!(
            post_to(state, "/api/plant/explode").await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            sent(&switch),
            ["Term Process(7)", "Kill Process(7)", "Kill Group(7)"]
        );
    }

    #[tokio::test]
    async fn a_start_carries_the_agents_asked_within_bounds() {
        let (state, switch) = with_plant(Vec::new());
        let (status, _) = post_to(state.clone(), "/api/plant/start?lanes=0").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, body) = post_to(state, "/api/plant/start?lanes=4").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("at most 4 agents"), "{body}");
        assert_eq!(sent(&switch), ["start 4"]);
    }

    #[tokio::test]
    async fn a_watch_that_dies_at_start_is_a_failed_start_with_its_words() {
        let switch = Arc::new(Switch {
            dies_at_start: true,
            ..Switch::default()
        });
        let mut state = state(Shelf::default(), None);
        state.plant = Some(Arc::clone(&switch) as Arc<dyn Plant>);
        let (status, body) = post_to(state, "/api/plant/start").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body.contains("exited as soon as it started"), "{body}");
        assert!(body.contains("no git repository"), "{body}");
    }

    /// Counts what it was asked, and answers the same every time.
    #[derive(Default)]
    struct Gauges {
        probes: std::sync::atomic::AtomicU32,
    }

    impl Limits for Gauges {
        fn claude(&self) -> Result<Reading, String> {
            self.probes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Reading {
                windows: Vec::new(),
                at: 1,
            })
        }

        fn github(&self) -> Result<Vec<crate::ports::GithubWindow>, String> {
            Err("gh: not logged in".to_string())
        }
    }

    #[tokio::test]
    async fn an_opened_panel_reads_both_limits_and_pays_claude_once() {
        let gauges = Arc::new(Gauges::default());
        let mut state = state(Shelf::default(), None);
        state.limits = Some(Arc::clone(&gauges) as Arc<dyn Limits>);
        let (status, body) = get(state.clone(), "/api/limits").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"claude\":{\"value\":{"), "{body}");
        assert!(body.contains("gh: not logged in"), "{body}");
        let _ = get(state.clone(), "/api/limits").await;
        let _ = get(state, "/api/limits?force=1").await;
        assert_eq!(
            gauges.probes.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a fresh reading is reused, even when asked again at once"
        );
    }

    #[tokio::test]
    async fn a_period_is_read_from_the_traces_and_a_bad_one_refused() {
        let state = state(Shelf::default(), None);
        let (status, body) = get(
            state.clone(),
            "/api/history?from=2026-10-08T20:00:00Z&to=2026-10-08T21:00:00Z",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("\"from\":\"2026-10-08T20:00:00Z\""), "{body}");
        let (status, _) = get(state.clone(), "/api/history?from=yesterday").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = get(
            state,
            "/api/history?from=2026-10-08T21:00:00Z&to=2026-10-08T20:00:00Z",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_page_reads_whether_a_watch_runs() {
        let (state, _) = with_plant(watch(7));
        let (status, body) = get(state, "/api/plant").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"available\":true"), "{body}");
        assert!(body.contains("\"pid\":7"), "{body}");
        assert!(body.contains("\"max_lanes\":16"), "{body}");
        let (_, none) = get(state_without_plant(), "/api/plant").await;
        assert!(none.contains("\"available\":false"), "{none}");
    }

    fn state_without_plant() -> AppState {
        state(Shelf::default(), None)
    }

    async fn get(state: AppState, uri: &str) -> (StatusCode, String) {
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn the_page_and_the_picture_are_served() {
        let (status, body) = get(state(Shelf::default(), None), "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("<canvas"), "{body}");
        assert!(body.contains("/vendor/xterm.js"), "{body}");
        assert!(!body.contains("iso.js"), "the canvas engine is gone");
        let (status, body) = get(state(Shelf::default(), None), "/render/render.js").await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "no bundle in the test checkout"
        );
        assert!(body.contains("build-render"), "{body}");
        let (status, _) = get(state(Shelf::default(), None), "/render/..%2Fapp.js").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "only the two bundle files");
        let (status, body) = get(state(Shelf::default(), None), "/api/snapshot").await;
        assert_eq!(status, StatusCode::OK);
        let snap: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(snap["rooms"].as_array().map(Vec::len), Some(6));
        assert_eq!(snap["lines"][0]["id"], "agent-loop");
        let (status, body) = get(state(Shelf::default(), None), "/vendor/xterm.js").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.len() > 100_000, "xterm.js is embedded whole");
    }

    #[tokio::test]
    async fn a_run_s_sessions_come_back_one_per_stage() {
        let mut shelf = Shelf::default();
        shelf.put(
            "agent-loop",
            "20261008-152126-64",
            "run.log",
            "[2026-10-08T15:21:28Z] run 1\n[2026-10-08T15:21:29Z] [code] session opens\n",
        );
        shelf.put(
            "agent-loop",
            "20261008-152126-64",
            "session.log",
            "── turn 1 /code ──\nbuilt it\n",
        );
        let (status, body) = get(
            state(shelf, None),
            "/api/runs/agent-loop/20261008-152126-64/stages",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"stage\":\"code\""), "{body}");
        assert!(body.contains("built it"), "{body}");
        assert!(
            body.contains("\"started_at\":\"2026-10-08T15:21:28Z\""),
            "{body}"
        );
        assert!(
            body.contains("\"opened_at\":\"2026-10-08T15:21:29Z\""),
            "{body}"
        );
        assert!(body.contains("\"closed_at\":null"), "{body}");
        let (status, _) = get(
            state(Shelf::default(), None),
            "/api/runs/agent-loop/..%2Fx/stages",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_tail_is_bounded_to_the_run_files_and_nothing_above_them() {
        let mut shelf = Shelf::default();
        shelf.put(
            "agent-loop",
            "20261006-202608",
            "session.log",
            "line one\nline two\n",
        );
        let (status, body) = get(
            state(shelf, None),
            "/api/runs/agent-loop/20261006-202608/session.log?bytes=100",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "line one\nline two\n");
        for bad in [
            "/api/runs/agent-loop/20261006-202608/costs.tsv",
            "/api/runs/agent-loop/..%2F..%2Fcosts.tsv/run.log",
            "/api/runs/Agent/20261006-202608/run.log",
            "/api/runs/agent-loop/20261006-999999/run.log",
        ] {
            let (status, _) = get(state(Shelf::default(), None), bad).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{bad}");
        }
    }

    #[tokio::test]
    async fn an_issue_before_the_board_was_read_is_not_found_rather_than_empty() {
        let (status, body) = get(state(Shelf::default(), None), "/api/issues/62").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("board"));
    }

    #[tokio::test]
    async fn the_steward_status_says_whether_there_is_one_and_whether_they_sit() {
        let (status, body) = get(state(Shelf::default(), None), "/api/steward").await;
        assert_eq!(status, StatusCode::OK);
        let none: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(none["available"], false);

        let desk = Desk::new(Arc::new(Echoing::default()));
        let (status, body) = get(state(Shelf::default(), Some(desk.clone())), "/api/steward").await;
        assert_eq!(status, StatusCode::OK);
        let some: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(some["available"], true);
        assert_eq!(some["live"], false, "nobody sits before the first visit");
        assert_eq!(some["command"], "echo");

        // A plain GET is not a WebSocket handshake: the route exists and
        // refuses it before any desk is consulted.
        let (status, _) = get(state(Shelf::default(), None), "/api/steward/term").await;
        assert!(status.is_client_error(), "{status}");
    }

    #[tokio::test]
    async fn the_janitor_weighs_sweeps_and_takes_a_new_limit_from_the_page() {
        use crate::domain::cleanup::fake::Lot;
        use crate::ports::{Heap, Survey, Yard};
        let (status, body) = get(state_without_plant(), "/api/janitor").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"available\":false"), "{body}");
        let (status, _) = post_to(state_without_plant(), "/api/janitor/sweep").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let lot = Arc::new(Lot {
            survey: Survey {
                total: 300,
                folders: vec![Heap {
                    rel: "init".to_string(),
                    bytes: 300,
                    idle_secs: 3 * 86_400,
                    dir: true,
                }],
                ..Survey::default()
            },
            ..Lot::default()
        });
        let mut state = state_without_plant();
        state.janitor = Some(Janitor::new(Arc::clone(&lot) as Arc<dyn Yard>));
        let (status, body) = post_to(state.clone(), "/api/janitor/diagnose").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"freeable\":300"), "{body}");
        assert!(
            lot.removed.lock().expect("lock").is_empty(),
            "a diagnosis removes nothing"
        );
        let (status, body) = post_to(state.clone(), "/api/janitor/sweep").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"freed\":300"), "{body}");
        assert_eq!(*lot.removed.lock().expect("lock"), ["init"]);

        let (status, why) = post_body(
            state.clone(),
            "/api/janitor/settings",
            r#"{"limit_bytes": 1000}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}");
        let twenty = 20_u64 * 1024 * 1024 * 1024;
        let (status, body) = post_body(
            state,
            "/api/janitor/settings",
            &format!(r#"{{"limit_bytes": {twenty}, "auto_sweep": true, "threshold_percent": 90}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains(&format!("\"limit_bytes\":{twenty}")),
            "{body}"
        );
        assert!(
            lot.read_settings()
                .is_some_and(|s| s.contains("\"auto_sweep\": true"))
        );
    }

    #[tokio::test]
    async fn the_doctor_has_a_desk_of_their_own() {
        let (status, body) = get(state(Shelf::default(), None), "/api/doctor").await;
        assert_eq!(status, StatusCode::OK);
        let none: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(none["available"], false);

        let mut with = state(Shelf::default(), None);
        with.doctor = Some(Desk::new(Arc::new(Echoing::default())));
        let (_, body) = get(with.clone(), "/api/doctor").await;
        let some: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(some["available"], true, "the doctor is in");
        let (_, body) = get(with, "/api/steward").await;
        let steward: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(
            steward["available"], false,
            "but the steward's desk is another"
        );

        let (status, _) = get(state(Shelf::default(), None), "/api/doctor/term").await;
        assert!(status.is_client_error(), "{status}");
    }

    #[tokio::test]
    async fn a_diagnosis_is_typed_to_the_doctor_and_heard_when_the_mark_comes_back() {
        let mut shelf = Shelf::default();
        shelf.put(
            "agent-loop",
            "20261007-142103",
            "run.log",
            "[12:00:01] FAILED: cargo test broke\n",
        );
        let mut with = state(shelf, None);
        let desk = Desk::new(Arc::new(Echoing::default()));
        with.doctor = Some(desk.clone());
        // No doctor, no diagnosis.
        let (status, _) = post_to(
            state(Shelf::default(), None),
            "/api/doctor/diagnose/agent-loop/20261007-142103",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // An unknown run is refused before anything is typed.
        let (status, _) = post_to(
            with.clone(),
            "/api/doctor/diagnose/agent-loop/20261007-000000",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!desk.is_live(), "nothing summoned the doctor");

        // The desk is sat at first, so the handler does not wait for a warm-up.
        desk.summon(80, 24).expect("summoned");
        let (status, body) = post_to(
            with.clone(),
            "/api/doctor/diagnose/agent-loop/20261007-142103",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let asked: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(asked["state"], "running");
        let (_, body) = get(with.clone(), "/api/doctor/diagnoses").await;
        assert!(
            body.contains(r#""agent-loop/20261007-142103":{"state":"running""#),
            "{body}"
        );

        // The echoing terminal repeats the question: that is not the answer.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (_, body) = get(with.clone(), "/api/doctor/diagnoses").await;
        assert!(body.contains(r#""state":"running""#), "{body}");

        // A reply that ends with the mark is.
        desk.write(b"...\r\nDIAGNOSIS: 20261007-142103 \xc2\xb7 amber\r\n")
            .expect("written");
        let mut done = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let (_, body) = get(with.clone(), "/api/doctor/diagnoses").await;
            if body.contains(r#""state":"done""#) {
                done = true;
                break;
            }
        }
        assert!(done, "the mark was heard");
        desk.shutdown();
    }

    #[test]
    fn a_workflow_segment_is_a_log_folder_name() {
        assert!(is_workflow("agent-loop"));
        assert!(is_workflow("pr-fix"));
        assert!(!is_workflow(""));
        assert!(!is_workflow("../logs"));
        assert!(!is_workflow("Agent"));
    }

    #[test]
    fn the_page_controls_are_size_and_restart() {
        let size: Control =
            serde_json::from_str(r#"{"t":"size","cols":100,"rows":40}"#).expect("size");
        assert!(matches!(
            size,
            Control::Size {
                cols: 100,
                rows: 40
            }
        ));
        let restart: Control = serde_json::from_str(r#"{"t":"restart"}"#).expect("restart");
        assert!(matches!(restart, Control::Restart));
        assert!(serde_json::from_str::<Control>(r#"{"t":"nuke"}"#).is_err());
    }
}
