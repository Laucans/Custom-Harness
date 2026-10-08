//! What the front-end receives: one serializable picture of the plant.
//!
//! Everything the page draws is in here, so a client never asks twice and
//! the SSE stream can push the whole thing on every change. Issue bodies are
//! the exception — they are fetched per issue, because thirty SPECs of twenty
//! kilobytes do not belong in a picture refreshed every two seconds.

use harness_core::domain::quota::Reading;
use serde::Serialize;

use crate::domain::blueprint::{Kind, Room};
use crate::domain::traces::{Costs, InFlight};
use crate::ports::ErrorRow;

/// The project the plant stands for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Project {
    /// The name on the sign.
    pub name: String,
    /// `owner/name`, or empty when the view runs without a GitHub target.
    pub slug: String,
    /// The repository's web page.
    pub url: String,
    /// The branch the agents integrate into.
    pub integration_branch: String,
}

/// One chimney: one model.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Chimney {
    /// `opus`, `sonnet`, `haiku`.
    pub model: String,
    /// A session on this model is open right now.
    pub smoking: bool,
    /// How many employees are on it.
    pub runs: u32,
    /// Dollars spent on stages that open this model by default — an estimate,
    /// since the ledger carries no model column.
    pub usd: f64,
}

/// What is seen from outside the plant.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Factory {
    /// One per model.
    pub chimneys: Vec<Chimney>,
    /// The watch is believed to be polling.
    pub watching: bool,
    /// The last tick, dispatched or not.
    pub last_tick_at: Option<String>,
    /// A dispatch the watch has not reported back on.
    pub in_flight: Option<InFlight>,
    /// What the router saw on its last tick.
    pub saw: Option<String>,
    /// Nobody is working.
    pub idle: bool,
}

/// What a station is doing in the latest run of its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StationState {
    /// Nothing passed here in this round.
    Idle,
    /// The product is here.
    Active,
    /// Reported in this round.
    Done,
    /// Skipped in this round.
    Skipped,
}

/// One station with its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StationView {
    /// From the blueprint.
    pub id: String,
    /// From the blueprint.
    pub label: String,
    /// From the blueprint.
    pub kind: Kind,
    /// From the blueprint.
    pub stage: Option<String>,
    /// From the blueprint.
    pub model: Option<String>,
    /// From the blueprint: what it is for.
    pub purpose: String,
    /// From the latest run.
    pub state: StationState,
}

/// The latest run of a line, in a few words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastRun {
    /// The run id — also the folder name.
    pub run_id: String,
    /// The clock of its first line.
    pub started_at: Option<String>,
    /// The clock of its last line.
    pub last_at: Option<String>,
    /// Seconds since its newest file was written.
    pub age_secs: Option<u64>,
    /// `#62 Asset folder rule…`, when the run named a task.
    pub task: Option<String>,
    /// `1/3`, when the run is in rounds.
    pub round: Option<String>,
    /// The last line of `run.log`.
    pub last_line: String,
    /// The last warning, if any.
    pub warning: Option<String>,
}

/// One production line with its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LineView {
    /// The log folder name.
    pub id: String,
    /// The name on the line.
    pub title: String,
    /// What starts it.
    pub trigger: String,
    /// What the whole workflow is for.
    pub purpose: String,
    /// In belt order.
    pub stations: Vec<StationView>,
    /// How many runs this line has logged.
    pub runs: u32,
    /// Its latest run.
    pub last_run: Option<LastRun>,
    /// Someone is on it right now.
    pub active: bool,
    /// Its last runs, newest first — who worked here, and on what.
    pub recent_work: Vec<RecentRun>,
}

/// One past or present run of a line, as its "recent work" lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecentRun {
    /// `20261008-152126-64`.
    pub run_id: String,
    /// `#64 · Dev loop`, the same name its employee carries.
    pub name: String,
    /// The issue it worked, when known.
    pub issue: Option<u64>,
    /// The stages its ledger rows name, oldest first — the machines it went
    /// through and finished.
    pub stages: Vec<String>,
    /// What it consumed, its finished stages summed.
    pub tokens: Option<Tokens>,
    /// Still at work.
    pub active: bool,
}

/// A running process, drawn as a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Employee {
    /// `agent-loop/20261006-202608`.
    pub id: String,
    /// `#62 · Dev loop` — the issue first.
    pub name: String,
    /// The line they stand on.
    pub workflow: String,
    /// The run they carry.
    pub run_id: String,
    /// The station they stand beside.
    pub station: Option<String>,
    /// The stage in progress.
    pub stage: Option<String>,
    /// The model the stage opened.
    pub model: Option<String>,
    /// `#62 Asset folder rule…`.
    pub task: Option<String>,
    /// `#17 Dernier kilomètre…`.
    pub milestone: Option<String>,
    /// `1/3`.
    pub round: Option<String>,
    /// The clock of the run's first line.
    pub since: Option<String>,
    /// Seconds since the run last wrote.
    pub age_secs: Option<u64>,
    /// The last line their session wrote.
    pub last_line: String,
    /// Still believed to be working.
    pub active: bool,
    /// What the run has consumed so far — its finished stages, as the cost
    /// ledger records them. `None` until its first stage ends.
    pub tokens: Option<Tokens>,
}

/// The tokens a run consumed, summed over the ledger rows it wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tokens {
    /// Input tokens, cache excluded.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Tokens read from cache.
    pub cache_read: u64,
    /// Tokens written to cache.
    pub cache_write: u64,
    /// All four together.
    pub total: u64,
    /// How many finished stages the sum covers.
    pub stages: u32,
}

/// Where an issue stands, from its labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    /// Closed.
    Done,
    /// On the integration branch, waiting for the merge that closes it.
    Delivered,
    /// Waiting on a human.
    Human,
    /// Blocked by an open issue.
    Blocked,
    /// `harness:ready`: the next thing the plant will take.
    Ready,
    /// Open, not ready.
    Todo,
}

/// An issue, enough to draw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueView {
    /// Its number.
    pub number: u64,
    /// Its title.
    pub title: String,
    /// `open` or `closed`.
    pub state: String,
    /// Its labels.
    pub labels: Vec<String>,
    /// Its web page.
    pub url: String,
    /// `roadmap`, `milestone`, `agent`, `human`, or `issue`.
    pub kind: &'static str,
    /// Where it stands.
    pub status: IssueStatus,
}

/// A milestone with its tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MilestoneView {
    /// The milestone issue.
    pub issue: IssueView,
    /// `milestone/<n>-<slug>`.
    pub branch: String,
    /// The branch's web page.
    pub branch_url: String,
    /// Its tasks.
    pub tasks: Vec<IssueView>,
    /// Tasks closed or delivered.
    pub done: u32,
    /// All tasks.
    pub total: u32,
}

/// The sign on the plant: what is to do, what is done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardView {
    /// Open roadmap items.
    pub roadmap: Vec<IssueView>,
    /// Milestones with their tasks.
    pub milestones: Vec<MilestoneView>,
    /// Open issues waiting on a human.
    pub needs_human: Vec<IssueView>,
}

/// A branch, as the store shows a version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Version {
    /// The branch name.
    pub name: String,
    /// Its web page.
    pub url: String,
    /// What it is.
    pub label: String,
}

/// What the store sells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Versions {
    /// The repository's web page.
    pub repo_url: String,
    /// `main`.
    pub main: Version,
    /// The agents' integration branch.
    pub integration: Version,
    /// One per open milestone.
    pub milestones: Vec<Version>,
}

/// The whole picture.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot {
    /// When it was assembled, ISO-8601 UTC.
    pub at: String,
    /// The project.
    pub project: Project,
    /// Seen from outside.
    pub factory: Factory,
    /// The people at work.
    pub employees: Vec<Employee>,
    /// The lines, in wall order.
    pub lines: Vec<LineView>,
    /// The six rooms.
    pub rooms: Vec<Room>,
    /// The sign — `None` when GitHub was not read.
    pub board: Option<BoardView>,
    /// The store.
    pub versions: Versions,
    /// The control room's numbers.
    pub costs: Costs,
    /// The rate-limit windows, as last read.
    pub quota: Option<Reading>,
    /// The last stops, newest first.
    pub errors: Vec<ErrorRow>,
    /// The watch journal's last lines.
    pub recent: Vec<String>,
    /// The latest run is shown live even though it is over.
    pub demo: bool,
}
