//! The HTTP side: the page, its scripts, a few small routes, and the
//! steward's terminal over a WebSocket.
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
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};
use tokio_stream::wrappers::WatchStream;
use tokio_stream::{Stream, StreamExt as _};

use crate::desk::Desk;
use crate::domain::history::{self, Range};
use crate::domain::limits::{self, Read, Report};
use crate::domain::plant::{self, Action, Gesture};
use crate::domain::snapshot::Snapshot;
use crate::domain::steward::Status;
use crate::domain::traces::{is_run_id, run_of, stage_logs};
use crate::ports::{BoardReading, Limits, Plant, Traces};
use harness_core::domain::quota::Reading;

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
    /// The watch process the page starts and stops — `None` under `--demo`.
    pub plant: Option<Arc<dyn Plant>>,
    /// How long a started watch must live before the start counts.
    pub start_grace: Duration,
    /// The rate limits, read on demand — `None` when nothing can read them.
    pub limits: Option<Arc<dyn Limits>>,
    /// The last Claude reading the page paid for, kept so a click is not a
    /// session.
    pub claude_read: Arc<std::sync::Mutex<Option<Read<Reading>>>>,
}

const INDEX: &str = include_str!("../static/index.html");
const STYLE: &str = include_str!("../static/style.css");
const APP: &str = include_str!("../static/app.js");
const XTERM_JS: &str = include_str!("../static/vendor/xterm.js");
const XTERM_CSS: &str = include_str!("../static/vendor/xterm.css");
const XTERM_FIT: &str = include_str!("../static/vendor/addon-fit.js");

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
        .route("/render/{file}", get(render_asset))
        .route("/vendor/xterm.js", get(xterm_js))
        .route("/vendor/xterm.css", get(xterm_css))
        .route("/vendor/addon-fit.js", get(xterm_fit))
        .route("/api/snapshot", get(snapshot))
        .route("/api/events", get(events))
        .route("/api/runs/{workflow}/locate", get(locate_run))
        .route("/api/runs/{workflow}/{run}/stages", get(run_stages))
        .route("/api/runs/{workflow}/{run}/{file}", get(run_file))
        .route("/api/issues/{number}", get(issue))
        .route("/api/history", get(period))
        .route("/api/limits", get(rate_limits))
        .route("/api/plant", get(plant_status))
        .route("/api/plant/{gesture}", post(plant_gesture))
        .route("/api/steward", get(steward))
        .route("/api/steward/term", get(term))
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
/// stage — what an agent's pane lists, and what a click on one shows.
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
    json(&stage_logs(&run_log, &session_log, &ledger_stages, tail))
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

/// Whether there is a steward, and whether they are at the desk.
async fn steward(State(state): State<AppState>) -> Response {
    json(&state.desk.as_ref().map_or_else(
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
    ))
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
    let computed = tokio::task::spawn_blocking(move || {
        history::history(
            range,
            &traces.ledger(),
            &traces.errors(),
            &traces.watch_log(HISTORY_BYTES).unwrap_or_default(),
            |task| sides.get(task).copied(),
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
    })
}

/// What a gesture did, for the page to say.
#[derive(Debug, Serialize)]
struct Done {
    message: String,
}

/// `start`, `soft` or `hard`: decided by `domain::plant`, carried out by
/// the port. A gesture that makes no sense now is a `409` with the reason.
///
/// Only the first action can fail the gesture: a hard stop's later kills
/// reach processes that may already be gone with the watch. A start counts
/// once the watch is still there after [`AppState::start_grace`] — one that
/// died on its first line (no remote, a bad flag) is a failed start, with
/// what it printed.
async fn plant_gesture(State(state): State<AppState>, Path(gesture): Path<String>) -> Response {
    let Some(plant) = state.plant.as_ref() else {
        return (StatusCode::NOT_FOUND, "this view has no hand on the plant").into_response();
    };
    let Some(gesture) = Gesture::parse(&gesture) else {
        return (StatusCode::NOT_FOUND, "unknown gesture").into_response();
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
            Action::Start => plant
                .start()
                .map(|pid| format!("watch started (pid {pid})")),
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

async fn term(
    ws: WebSocketUpgrade,
    Query(query): Query<TermQuery>,
    State(state): State<AppState>,
) -> Response {
    let Some(desk) = state.desk else {
        return not_found("no steward: the view runs with --no-steward");
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
        let snap = assemble::snapshot(&Inputs {
            observed: &observed,
            board: None,
            project: &project,
            lines: &lines,
            now: 0,
            demo: false,
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
            plant: None,
            start_grace: Duration::ZERO,
            limits: None,
            claude_read: Arc::default(),
        }
    }

    async fn post_to(state: AppState, uri: &str) -> (StatusCode, String) {
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
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
            "[2026-10-08T15:21:29Z] [code] session opens\n",
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
