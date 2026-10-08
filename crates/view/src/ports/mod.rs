//! What the view needs from the outside, as traits — and the values those
//! traits exchange. Two components: the traces the harness leaves on disk,
//! and the board it keeps on GitHub.
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
