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

use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};
use tokio_stream::wrappers::WatchStream;
use tokio_stream::{Stream, StreamExt as _};

use crate::desk::Desk;
use crate::domain::snapshot::Snapshot;
use crate::domain::steward::Status;
use crate::domain::traces::is_run_id;
use crate::ports::{BoardReading, Traces};

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
        .route("/api/runs/{workflow}/{run}/{file}", get(run_file))
        .route("/api/issues/{number}", get(issue))
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
    use crate::domain::snapshot::Project;
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
        }
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
