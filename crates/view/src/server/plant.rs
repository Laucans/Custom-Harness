//! The plant's own controls: the status button and its gestures, the
//! control room's period and rate limits, the restart after a change.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::domain::history::{self, Range};
use crate::domain::limits::{self, Read, Report};
use crate::domain::plant::{self, Action, Gesture};
use crate::domain::prompts::{self};

use super::{AppState, json, unix_now};

/// `?from=…&to=…`: two trace clocks, either left out.
#[derive(Debug, Deserialize)]
pub(super) struct PeriodQuery {
    from: Option<String>,
    to: Option<String>,
}

/// How much of `watch.log` a period reads: all of a long run's journal.
const HISTORY_BYTES: u64 = 32 * 1024 * 1024;

/// The control room over a chosen period, recomputed from the whole traces:
/// the ledger, the stops, and the watch journal.
pub(super) async fn period(
    State(state): State<AppState>,
    Query(query): Query<PeriodQuery>,
) -> Response {
    let range = match Range::parse(query.from.as_deref(), query.to.as_deref()) {
        Ok(range) => range,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    // The side and the milestone of a task, from the board as last read —
    // `None` for a task the board no longer lists, never guessed.
    let (sides, milestones) = state
        .board
        .borrow()
        .as_ref()
        .map(|board| (board.task_sides(), board.task_milestones()))
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
            |task| milestones.get(task).cloned(),
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
pub(super) struct LimitsQuery {
    force: Option<u8>,
}

/// Both rate limits, read now: GitHub every time (it is free), Claude when
/// the reading the page last paid for has aged (`domain::limits`). The two
/// reads run side by side, off the server's thread.
pub(super) async fn rate_limits(
    State(state): State<AppState>,
    Query(query): Query<LimitsQuery>,
) -> Response {
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
pub(super) async fn plant_status(State(state): State<AppState>) -> Response {
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
pub(super) struct Done {
    message: String,
}

/// `?lanes=N`: how many agents a start runs at once at most.
#[derive(Debug, Deserialize)]
pub(super) struct StartQuery {
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
pub(super) async fn plant_gesture(
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
pub(super) async fn pending(State(state): State<AppState>) -> Response {
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
pub(super) async fn restart(
    State(state): State<AppState>,
    Query(query): Query<StartQuery>,
) -> Response {
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
