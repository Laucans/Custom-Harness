//! A run's traces on demand: locate it, tail one of its files, its stages
//! and graph — and an issue's body with its blockers.

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::domain::assemble;
use crate::domain::blueprint;
use crate::domain::gates;
use crate::domain::observe::observe_run;
use crate::domain::snapshot::StationView;
use crate::domain::traces::{RunStages, clock_span, is_run_id, line_of_run, run_of, stage_logs};

use super::{AppState, is_workflow, json, not_found};

/// The run files a browser may tail.
const TAILABLE: [&str; 4] = ["run.log", "session.log", "prompts.md", "stream.jsonl"];

/// What a tail returns when the query does not say.
const TAIL_DEFAULT: u64 = 16 * 1024;

/// The most a tail returns however much the query asks.
const TAIL_MAX: u64 = 512 * 1024;

/// `?bytes=N`: how much of the file's end to return.
#[derive(Debug, Deserialize)]
pub(super) struct TailQuery {
    bytes: Option<u64>,
}

/// `?at=…&issue=…`: a trigger, as the journal stamped it.
#[derive(Debug, Deserialize)]
pub(super) struct LocateQuery {
    at: String,
    issue: Option<u64>,
}

/// The run a trigger started, so its row leads to its logs.
#[derive(Debug, Serialize)]
pub(super) struct Located {
    run: Option<String>,
}

pub(super) async fn locate_run(
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
pub(super) struct LineOfQuery {
    stage: Option<String>,
}

/// The line a run id belongs to.
#[derive(Debug, Serialize)]
pub(super) struct LineOf {
    workflow: Option<String>,
}

/// The line whose logs hold `run`, so a status known only by its run id —
/// a paid session, a stop — leads to those logs.
pub(super) async fn line_of(
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

pub(super) async fn run_file(
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
pub(super) async fn run_stages(
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
pub(super) struct RunGraph {
    /// In belt order.
    stations: Vec<StationView>,
}

/// How many of the newest events a graph reads for its gate verdicts.
const GRAPH_EVENTS: usize = 5_000;

pub(super) async fn run_graph(
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
pub(super) struct IssueBody {
    number: u64,
    title: String,
    state: String,
    labels: Vec<String>,
    body: String,
    url: String,
    blocked_by: Vec<Blocker>,
}

#[derive(Debug, Serialize)]
pub(super) struct Blocker {
    number: u64,
    title: String,
    state: String,
}

pub(super) async fn issue(State(state): State<AppState>, Path(number): Path<u64>) -> Response {
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
