//! What the view needs from the outside, as traits — and the values those
//! traits exchange: the traces the harness leaves on disk, the board it keeps
//! on GitHub, the steward's terminal, the watch process the page starts
//! and stops, and the rate limits it reads on demand.
//!
//! A port decides nothing. `FsTraces` knows where `.llocal/logs` is and what
//! a TSV looks like; what a row *means* is `domain`'s business.

use std::io::{self, Read};

use async_trait::async_trait;
use harness_core::domain::quota::Reading;
use harness_core::domain::{Issue, Outcome};
use serde::Serialize;

/// One row of the cost ledger, typed. `None` reads as "not observed", as the
/// ledger itself writes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LedgerRow {
    /// ISO-8601 UTC, to the second.
    pub when: String,
    /// The run id.
    pub run: String,
    /// The round, as written (`01`).
    pub round: String,
    /// The billed task; empty on a round that worked no task.
    pub task: String,
    /// The stage.
    pub stage: String,
    /// The carrier's estimate, in dollars.
    pub cost_usd: Option<f64>,
    /// Round-trips with the model.
    pub turns: Option<u32>,
    /// End to end.
    pub duration_ms: Option<u64>,
    /// Input tokens, cache excluded.
    pub input: Option<u64>,
    /// Output tokens.
    pub output: Option<u64>,
    /// Tokens read from cache.
    pub cache_read: Option<u64>,
    /// Tokens written to cache.
    pub cache_write: Option<u64>,
    /// `ok`, `STOP`, `FAILED`, `QUOTA`, or empty.
    pub outcome: String,
}

/// One row of the error ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorRow {
    /// ISO-8601 UTC, to the second.
    pub when: String,
    /// The run id.
    pub run: String,
    /// Which workflow stopped.
    pub workflow: String,
    /// `STOP`, `FAILED`, `QUOTA`, or a repair's conclusion.
    pub kind: String,
    /// What it said.
    pub reason: String,
}

/// What the harness leaves on disk, read without interpreting it.
///
/// Synchronous, like core's own store ports: these are small local files,
/// and claiming `async` would give a false idea of their cost. `Send + Sync`
/// because the HTTP handlers, which must be, read log tails through it.
pub trait Traces: Send + Sync {
    /// The run ids of this workflow, oldest first.
    fn runs(&self, workflow: &str) -> Vec<String>;

    /// A whole file of a run, or `None` if it is not there.
    fn read(&self, workflow: &str, run: &str, file: &str) -> Option<String>;

    /// The last `max_bytes` of a run file, cut at a line boundary.
    fn tail(&self, workflow: &str, run: &str, file: &str, max_bytes: u64) -> Option<String>;

    /// The first `max_bytes` of a run file.
    fn head(&self, workflow: &str, run: &str, file: &str, max_bytes: u64) -> Option<String>;

    /// Seconds since the newest file of this run was written.
    fn age_secs(&self, workflow: &str, run: &str) -> Option<u64>;

    /// Whether this run's process is still alive, from the OS lock it holds
    /// on its `alive.lock`. `None` when the run has no such file — one
    /// older than it — and only the age of its files can say.
    fn alive(&self, workflow: &str, run: &str) -> Option<bool>;

    /// The last `max_bytes` of the watch journal.
    fn watch_log(&self, max_bytes: u64) -> Option<String>;

    /// Every row of the cost ledger. Empty when there is none, or when it
    /// cannot be read — the view shows, it never pays, so the two are the same
    /// to it.
    fn ledger(&self) -> Vec<LedgerRow>;

    /// Every row of the error ledger, oldest first.
    fn errors(&self) -> Vec<ErrorRow>;

    /// The last rate-limit reading a run kept.
    fn quota(&self) -> Option<Reading>;
}

/// A milestone and the tasks under it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Milestone {
    /// The milestone issue.
    pub issue: Issue,
    /// Its sub-issues, blockers included where they were read.
    pub tasks: Vec<Issue>,
}

/// The board, as GitHub has it right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoardReading {
    /// `owner/name`.
    pub slug: String,
    /// Open roadmap items.
    pub roadmap: Vec<Issue>,
    /// Milestones, open ones first, each with its tasks.
    pub milestones: Vec<Milestone>,
}

/// Where the board is read from.
///
/// `?Send`, like every port in `harness-core` (migration decision #3): the
/// adapter behind it wraps core's `GitHub`, whose futures are not `Send`.
#[async_trait(?Send)]
pub trait Board {
    /// One full read of the board.
    ///
    /// # Errors
    ///
    /// Whatever the GitHub port reports — an unreadable board is shown as
    /// "no board", never as an empty one.
    async fn read(&self) -> Outcome<BoardReading>;
}

/// An interactive program behind a terminal: bytes in, bytes out, a size.
///
/// `Send`, because the desk that holds it is shared by every connection to
/// the page. Synchronous: writes to a terminal are small, and the output is
/// pumped by a thread of the desk's own.
pub trait TerminalIo: Send {
    /// Keystrokes, as the browser sends them.
    ///
    /// # Errors
    ///
    /// The terminal's own write error — the program is gone, most often.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;

    /// The browser's terminal changed size.
    ///
    /// # Errors
    ///
    /// The terminal's own error.
    fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()>;

    /// The program's output, handed over once; `None` the second time.
    fn output(&mut self) -> Option<Box<dyn Read + Send>>;

    /// Ends the program and waits for it.
    fn kill(&mut self);
}

/// What opens a terminal with the steward's program in it.
pub trait TerminalFactory: Send + Sync {
    /// Opens a terminal of this size and starts the program in it.
    ///
    /// # Errors
    ///
    /// Why it could not start — no pseudo-terminal, or the program is not on
    /// `PATH` — as a sentence the page can show.
    fn spawn(&self, cols: u16, rows: u16) -> Result<Box<dyn TerminalIo>, String>;

    /// The command line, for the page to name.
    fn command(&self) -> String;
}

/// A `harness watch` process of this checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Running {
    /// Its pid.
    pub pid: u32,
    /// Its process group — what a hard stop kills, lanes included.
    pub group: u32,
    /// The `--parallel` its command line carries: the most agents it runs
    /// at once. `None`: not on the line, so the launcher's own default.
    pub lanes: Option<usize>,
}

/// The signal a stop sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// `SIGTERM`: the watch drains its lanes, then exits.
    Term,
    /// `SIGKILL`: nothing finishes.
    Kill,
}

/// Who a signal goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// One process.
    Process(u32),
    /// A whole process group.
    Group(u32),
}

/// One line of the OS's process table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    /// Its pid.
    pub pid: u32,
    /// Its parent's pid; `1` once its parent is gone.
    pub parent: u32,
    /// Its process group.
    pub group: u32,
    /// Its command line.
    pub command: String,
}

/// The watch process, as the OS has it: found, started, signalled.
///
/// Synchronous: each call is one short `ps`, `lsof` or `kill`. `Send + Sync`
/// because the HTTP handlers call it.
pub trait Plant: Send + Sync {
    /// Every process of the machine.
    fn processes(&self) -> Vec<Process>;

    /// Whether `pid` works in this checkout — another plant's watch on the
    /// same machine is none of this page's business.
    fn works_here(&self, pid: u32) -> bool;

    /// Starts the watch, detached in a process group of its own, with at
    /// most `lanes` agents at once — `None` keeps the configured command.
    ///
    /// # Errors
    ///
    /// Why it could not start, as a sentence the page can show.
    fn start(&self, lanes: Option<usize>) -> Result<u32, String>;

    /// The last lines the watch printed — what says why it died at start.
    fn output_tail(&self) -> String;

    /// Sends `signal` to `target`.
    ///
    /// # Errors
    ///
    /// Why the signal could not be sent.
    fn send(&self, signal: Signal, target: Target) -> Result<(), String>;
}

/// One GitHub API rate-limit bucket (`core`, `graphql`, `search`, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GithubWindow {
    /// The bucket, as GitHub names it.
    pub name: String,
    /// Requests allowed per window.
    pub limit: u64,
    /// Requests already made in this window.
    pub used: u64,
    /// When the window resets, in seconds since the epoch.
    pub resets_at: u64,
}

/// The rate limits, read now rather than remembered.
///
/// Synchronous and `Send + Sync`: the server calls it off its thread, one
/// short process per read.
pub trait Limits: Send + Sync {
    /// Claude's subscription windows, as a minimal session sees them now.
    ///
    /// # Errors
    ///
    /// Why no reading came back — no `claude`, a timeout, no event.
    fn claude(&self) -> Result<Reading, String>;

    /// The GitHub API buckets of the account `gh` is logged in with — a read
    /// GitHub does not count against them.
    ///
    /// # Errors
    ///
    /// Why `gh` gave no answer.
    fn github(&self) -> Result<Vec<GithubWindow>, String>;
}

/// One thing lying in the yard — a folder or a file under `.llocal/` —
/// measured, not judged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Heap {
    /// Its path, relative to the yard: `agentic_workspaces/lane-0/target`.
    pub rel: String,
    /// Bytes under it, symlinks counted as themselves and never followed.
    pub bytes: u64,
    /// Seconds since anything in it, or just under it, was last written.
    pub idle_secs: u64,
    /// A folder, rather than a file.
    pub dir: bool,
}

/// A workspace the harness clones under `agentic_workspaces/`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkspaceHeap {
    /// The folder, as a heap: its whole weight, how long it has been idle.
    pub heap: Heap,
    /// It carries a `.git` — a checkout the harness made, not a stray.
    pub git: bool,
    /// The build outputs and dependency folders found in it, each as a heap
    /// of its own, weight included in the workspace's.
    pub caches: Vec<Heap>,
}

/// One run's traces under `logs/<workflow>/<run>/`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RunHeap {
    /// The line it ran on: `split`.
    pub workflow: String,
    /// The run id.
    pub run: String,
    /// The folder, as a heap.
    pub heap: Heap,
    /// The run's flow file beside the folder, when there is one.
    pub flow: Option<Heap>,
}

/// What lies in the yard, as of one walk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Survey {
    /// When the walk was made, as the traces write a clock.
    pub taken_at: String,
    /// Bytes under the yard, all told.
    pub total: u64,
    /// Everything directly under the yard, folders and files alike.
    pub folders: Vec<Heap>,
    /// The workspaces, each with its caches.
    pub workspaces: Vec<WorkspaceHeap>,
    /// The runs' traces, each with its flow file.
    pub runs: Vec<RunHeap>,
}

/// The yard — `.llocal/`, where the harness leaves its clones, its traces
/// and its stores — walked and swept.
///
/// Synchronous: a walk is a few seconds of disk at most and runs on a
/// blocking thread; a removal is one call. `Send + Sync` because the HTTP
/// handlers and the chronic tick share it.
pub trait Yard: Send + Sync {
    /// Walks the yard and weighs what lies in it.
    fn survey(&self) -> Survey;

    /// Removes `rel` — a path relative to the yard — and says how many bytes
    /// it weighed. A symlink is unlinked, never followed.
    ///
    /// # Errors
    ///
    /// The path leaves the yard, is not there, or the OS refused.
    fn remove(&self, rel: &str) -> Result<u64, String>;

    /// The janitor's settings as last written, `None` when never.
    fn read_settings(&self) -> Option<String>;

    /// Keeps the janitor's settings for the next start.
    ///
    /// # Errors
    ///
    /// The OS refused the write.
    fn write_settings(&self, json: &str) -> Result<(), String>;
}
