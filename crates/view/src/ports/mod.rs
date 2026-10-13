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
use serde::{Deserialize, Serialize};

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

impl BoardReading {
    /// Which side each task is on, by number: `true` for the data layer.
    /// A task the board does not list is absent, never guessed.
    #[must_use]
    pub fn task_sides(&self) -> std::collections::HashMap<String, bool> {
        self.milestones
            .iter()
            .flat_map(|m| m.tasks.iter())
            .map(|t| {
                (
                    t.number.to_string(),
                    t.labels
                        .iter()
                        .any(|l| l == harness_workflows::common::labels::DATA_LAYER),
                )
            })
            .collect()
    }

    /// The milestone each task — and each milestone itself, whose own
    /// refinement is billed on its number — belongs to, by number, as
    /// `#<n> <title>`.
    #[must_use]
    pub fn task_milestones(&self) -> std::collections::HashMap<String, String> {
        self.milestones
            .iter()
            .flat_map(|m| {
                let key = format!("#{} {}", m.issue.number, m.issue.title);
                std::iter::once(m.issue.number.to_string())
                    .chain(m.tasks.iter().map(|t| t.number.to_string()))
                    .map(move |number| (number, key.clone()))
            })
            .collect()
    }
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
    /// Open issues labelled `bug`: what turns a unit of the capabilities
    /// list red.
    pub bugs: Vec<Issue>,
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

/// The product's own repository — the one the harness builds — for the
/// files that describe it: its data model, live on the integration branch,
/// and every version the watch archived on `harness/data-model`.
///
/// `?Send`, like [`Board`]: the GitHub adapter wraps core's `GitHub`.
#[async_trait(?Send)]
pub trait Product {
    /// Where the files are read, in words: `owner/name`.
    fn origin(&self) -> String;

    /// The ref the live files are read at: the integration branch.
    fn live_ref(&self) -> String;

    /// The text of `path` at `git_ref`, `None` when the file is not there.
    ///
    /// # Errors
    ///
    /// The read failed for another reason than absence.
    async fn file(&self, path: &str, git_ref: &str) -> Outcome<Option<String>>;

    /// The product's branches named `prefix…` — its milestones', for one.
    ///
    /// # Errors
    ///
    /// The read failed.
    async fn branches(&self, prefix: &str) -> Outcome<Vec<String>>;

    /// Opens an issue on the product's board — the one write the data pane
    /// makes, on a human's click — and says where it is.
    ///
    /// # Errors
    ///
    /// GitHub refused, or there is no GitHub behind this product.
    async fn open_issue(&self, title: &str, body: &str, labels: &[&str]) -> Outcome<String>;
}

/// One pin as kept: the pin, and when it was deleted, if it was.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptNote {
    /// The pin.
    pub note: crate::domain::annotations::Note,
    /// When it was deleted — `None` while it is on its board.
    pub deleted_at: Option<String>,
}

/// Where the human's pins and their conversations are kept.
///
/// Synchronous, like [`Yard`]: a small local database. `Send + Sync`
/// because the HTTP handlers share it.
pub trait NoteStore: Send + Sync {
    /// Every board that holds a pin still on it, sorted.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn boards(&self) -> Result<Vec<String>, String>;

    /// A board's pins, deleted ones included, each with its conversation's
    /// turn count in `note.turns`.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn notes(&self, board: &str) -> Result<Vec<KeptNote>, String>;

    /// Keeps a pin — a new one, or a new version of one; its conversation and
    /// its deletion mark are left as they are.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn put(&self, board: &str, note: &crate::domain::annotations::Note) -> Result<(), String>;

    /// Marks a pin deleted at `at` (`None` brings it back).
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn mark_deleted(&self, board: &str, id: &str, at: Option<&str>) -> Result<(), String>;

    /// Destroys a pin and its conversation for good.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn destroy(&self, board: &str, id: &str) -> Result<(), String>;

    /// A pin's conversation, as JSON; `None` when it has none.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn conversation(&self, board: &str, id: &str) -> Result<Option<String>, String>;

    /// Keeps a pin's conversation and its turn count.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_conversation(
        &self,
        board: &str,
        id: &str,
        json: &str,
        turns: usize,
    ) -> Result<(), String>;
}

/// One question to Claude, answered in one go (`claude -p`): a summary, a
/// conclusion, a request. Blocking — a handler runs it on a blocking thread.
pub trait Oneshot: Send + Sync {
    /// The answer to `prompt`, on `model`.
    ///
    /// # Errors
    ///
    /// Claude could not be run, or answered nothing.
    fn ask(&self, model: &str, prompt: &str) -> Result<String, String>;

    /// The answer to `prompt`, on `model`, asked in `dir` with the files
    /// there readable — a moodboard's images, a reference drawing. Nothing
    /// is written. The default reads nothing: it is [`Oneshot::ask`].
    ///
    /// # Errors
    ///
    /// Claude could not be run, or answered nothing.
    fn ask_in(&self, model: &str, prompt: &str, dir: &std::path::Path) -> Result<String, String> {
        let _ = dir;
        self.ask(model, prompt)
    }

    /// The answer to `prompt`, on `model`, by a session at work in `dir`:
    /// it reads and **writes** the files there — a mock-up drawn — and
    /// answers what it did. Nothing outside `dir` is its business; nothing
    /// is committed. The default writes nothing and refuses.
    ///
    /// # Errors
    ///
    /// Claude could not be run, gave up, or this Claude cannot write.
    fn work_in(&self, model: &str, prompt: &str, dir: &std::path::Path) -> Result<String, String> {
        let _ = (model, prompt, dir);
        Err("this Claude cannot write files".to_string())
    }
}

/// What a new factory is made of, outside the view: its repository on
/// GitHub, its folder on this machine, the harness's `init-repo` run there.
/// Blocking — a desk runs it on a blocking thread.
pub trait Foundry: Send + Sync {
    /// The accounts a repository may be created under: the human's own
    /// login first, then their organisations.
    ///
    /// # Errors
    ///
    /// `gh` could not be asked, or is not signed in.
    fn owners(&self) -> Result<Vec<String>, String>;

    /// Creates `owner/name` on GitHub, with a first commit so a branch can
    /// be born from it, and answers its `owner/name`.
    ///
    /// # Errors
    ///
    /// GitHub refused: the name is taken, the owner is not the human's.
    fn create_repository(
        &self,
        owner: &str,
        name: &str,
        private: bool,
        description: &str,
    ) -> Result<String, String>;

    /// Makes the factory's folder at `root`: a checkout of the harness, so
    /// its skills are there to lend and its `.llocal/` is the factory's own.
    ///
    /// # Errors
    ///
    /// `root` exists already, or `git` refused.
    fn make_folder(&self, root: &std::path::Path) -> Result<(), String>;

    /// Runs `harness init-repo` for `slug` in `root`, which writes its
    /// `.env.local`, then adds there what the view needs to drive the
    /// factory from anywhere (the harness binary, by absolute path). Answers
    /// the report; `ready` is false when gaps remain that a human must close.
    ///
    /// # Errors
    ///
    /// The harness could not be run, or stopped.
    fn init_repo(
        &self,
        root: &std::path::Path,
        slug: &str,
        branch: &str,
    ) -> Result<Initialized, String>;
}

/// What `harness init-repo` said.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Initialized {
    /// Nothing blocks: the repository is ready to be worked.
    pub ready: bool,
    /// Its report, as printed.
    pub report: String,
}

/// Whether the account `gh` is signed in with may delete a repository,
/// as far as a read-only look can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rights {
    /// The token lists the `delete_repo` scope.
    Granted,
    /// The token lists its scopes and `delete_repo` is not among them.
    Missing,
    /// Nothing says: a fine-grained token lists no scopes, or `gh` did not answer.
    Unknown,
}

/// What taking a factory down touches outside the register: its repository
/// on GitHub, its folder on this machine. Blocking — a handler runs it on a
/// blocking thread.
pub trait Teardown: Send + Sync {
    /// Whether the token may delete a repository, asked of GitHub without
    /// deleting anything.
    fn delete_right(&self) -> Rights;

    /// Deletes `slug` (`owner/name`) on GitHub, for good.
    ///
    /// # Errors
    ///
    /// GitHub refused: no right to delete it, or `gh` lacks the
    /// `delete_repo` scope.
    fn delete_repository(&self, slug: &str) -> Result<(), String>;

    /// Removes the factory's folder `root`: a worktree of the harness
    /// checkout, with its `.llocal/`. A folder already gone is removed.
    ///
    /// # Errors
    ///
    /// `root` is the checkout the view runs from, or `git` and the disk both
    /// refused.
    fn remove_folder(&self, root: &std::path::Path) -> Result<(), String>;

    /// Removes what the harness keeps in the checkout `root` (`.llocal/`)
    /// and leaves the checkout itself, which somebody else keeps. Nothing
    /// else there is removed — `.env.local` least of all: it is the
    /// checkout's own settings, not a factory's, and another process may be
    /// reading it right now.
    ///
    /// # Errors
    ///
    /// The disk refused.
    fn wipe_harness_data(&self, root: &std::path::Path) -> Result<(), String>;
}

/// One `claude -p` call the view itself paid for — never a dev-loop run,
/// which has its own ledger (`LedgerRow`): a question the view asked on its
/// own account (the design system's tokens, a factory's grilling, a
/// mock-up drawn) to answer a human waiting on a page, not a task.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spend {
    /// ISO-8601 UTC, to the second.
    pub at: String,
    /// Who asked: a short tag fixed when the asker was built (`creation`,
    /// `kit`, `mf`, `notes`, `units`).
    pub who: String,
    /// The model asked.
    pub model: String,
    /// What the call could touch: `ask` (nothing), `read`, or `write`.
    pub mode: String,
    /// The API's own estimate; `None` when the answer carried none.
    pub cost_usd: Option<f64>,
    /// End to end.
    pub duration_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Where the view keeps what its own `claude -p` calls cost — never a run's
/// ledger, read by the dashboards' *Spending* screen beside it.
pub trait Usage: Send + Sync {
    /// Keeps one call. Never fails a call that answered: a handler logs the
    /// error and moves on.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn log(&self, spend: &Spend) -> Result<(), String>;

    /// Every call kept, newest first.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn all(&self) -> Result<Vec<Spend>, String>;
}

/// What Claude Code kept of a session it ran: its transcript.
pub trait Transcripts: Send + Sync {
    /// The transcript of session `id`, JSON lines; `None` when there is none.
    fn read(&self, id: &str) -> Option<String>;
}

/// Makes the terminal a pin's session runs in: a program, its folder, its
/// arguments.
pub trait TerminalMaker: Send + Sync {
    /// A factory for a terminal running the configured program in `cwd`
    /// with `args`.
    fn make(&self, cwd: &std::path::Path, args: Vec<String>)
    -> std::sync::Arc<dyn TerminalFactory>;
}

/// The prompt book and its drafts, as the view may touch them: the book read
/// only — the harness alone writes what runs — the drafts read and written.
///
/// `Send + Sync` because the HTTP handlers share it.
pub trait Prompts: Send + Sync {
    /// What runs.
    ///
    /// # Errors
    ///
    /// The book exists and cannot be read.
    fn book(&self) -> Result<harness_core::domain::prompt_book::PromptBook, String>;

    /// What waits for the next start.
    ///
    /// # Errors
    ///
    /// The drafts exist and cannot be read.
    fn drafts(&self) -> Result<harness_core::domain::prompt_book::Drafts, String>;

    /// Keeps every draft.
    ///
    /// # Errors
    ///
    /// The OS refused the write.
    fn keep_drafts(&self, drafts: &harness_core::domain::prompt_book::Drafts)
    -> Result<(), String>;

    /// The drafts the human deactivated.
    ///
    /// # Errors
    ///
    /// The shelf exists and cannot be read.
    fn shelf(&self) -> Result<harness_core::domain::prompt_book::Shelf, String>;

    /// Keeps every deactivated draft.
    ///
    /// # Errors
    ///
    /// The OS refused the write.
    fn keep_shelf(
        &self,
        deactivated: &harness_core::domain::prompt_book::Shelf,
    ) -> Result<(), String>;
}

/// What the human asks of a prompt's dry-run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialAsk {
    /// The trial's own name, a folder under `.llocal/trials/`.
    pub id: String,
    /// The prompt tried.
    pub key: String,
    /// How the human names the version tried.
    pub version: String,
    /// The text tried — handed to the trial, never written where a run reads.
    pub text: String,
    /// The line of the run replayed.
    pub workflow: String,
    /// The run replayed.
    pub run: String,
    /// The model the session opens on.
    pub model: String,
    /// Its effort.
    pub effort: String,
}

/// The prompts' dry-runs: started as `harness trial`, read back from their
/// reports.
pub trait Trials: Send + Sync {
    /// Starts one.
    ///
    /// # Errors
    ///
    /// Why it could not start, as a sentence the page can show.
    fn start(&self, ask: &TrialAsk) -> Result<(), String>;

    /// Every trial with a report, by id — one whose process died before
    /// reporting reads as failed.
    fn reports(&self) -> Vec<(String, harness_core::domain::prompt_book::TrialReport)>;
}

/// One manifest of the product, as one branch has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitFile {
    /// Relative to the product's root.
    pub path: String,
    /// Its text.
    pub text: String,
}

/// The manifests one branch carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUnits {
    /// The branch.
    pub git_ref: String,
    /// Its `capability.json`, `data-capability.json`, `micro-ui.json`,
    /// `aggregate.json` and `concept.json` files, and its persisted query
    /// registries (`queries/registry.json`).
    pub files: Vec<UnitFile>,
}

/// When a manifest was first and last committed, on any branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDates {
    /// Relative to the product's root.
    pub path: String,
    /// ISO-8601, the oldest commit that touched it.
    pub created: String,
    /// ISO-8601, the newest.
    pub modified: String,
}

/// The product's units, every branch at once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitsReading {
    /// One entry per branch.
    pub refs: Vec<RefUnits>,
    /// The Capabilities' and `DataCapabilities`' dates.
    pub dates: Vec<FileDates>,
}

/// The product's architecture manifests, read on every branch — what the
/// capabilities list is drawn from.
///
/// Synchronous and blocking (a fetch, a few `git` reads): a handler runs it
/// on a blocking thread. `Send + Sync` because the handlers share it.
pub trait Units: Send + Sync {
    /// Reads every branch afresh, or as recently as the adapter allows.
    ///
    /// # Errors
    ///
    /// The repository cannot be reached or read.
    fn read(&self) -> Result<UnitsReading, String>;
}

/// The harness's own commands the data pane starts — a snapshot, a draft —
/// and where each one is.
pub trait Builds: Send + Sync {
    /// Starts `what` for `branch`, unless the same build is running, and
    /// answers its id.
    ///
    /// # Errors
    ///
    /// The same build is running, or the harness could not be started.
    fn start(&self, what: crate::domain::builds::What, branch: &str) -> Result<String, String>;

    /// Every build started since the view started, newest first.
    fn builds(&self) -> Vec<crate::domain::builds::Build>;

    /// A build's whole output so far, by its id; `None` when there is none.
    fn log(&self, id: &str) -> Option<String>;
}

/// One file of a mock-up, and when it was last written.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MockFile {
    /// Its path under `mock/`.
    pub path: String,
    /// When it was last written, in seconds since the epoch; 0 when it is
    /// read as committed.
    pub at: u64,
}

/// One page of the product's mock-up, as `mock/pages.json` lists it
/// (contract N).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockPage {
    /// The page's file, relative to `mock/`: `pages/credit-review.html`.
    pub path: String,
    /// What a human calls it.
    pub title: String,
    /// The user goal it was created from.
    #[serde(default)]
    pub objective: String,
    /// The frozen screen it mocks, once there is one.
    #[serde(default)]
    pub screen: String,
    /// The micro-frontends it injects, by name.
    #[serde(default)]
    pub components: Vec<String>,
    /// The fingerprint of the page's file as it is — what a draft version
    /// remembers to know the page moved under it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash: String,
}

/// One micro-frontend of the mock-up, as `mock/pages.json` lists it: a
/// Micro-UI as the mock shows it (contract N).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockComponent {
    /// The Micro-UI's name: `RiskBadge`.
    pub name: String,
    /// Its system.
    #[serde(default)]
    pub system: String,
    /// What it shows and lets the user do.
    #[serde(default)]
    pub description: String,
    /// Its markup and script, relative to `mock/`.
    #[serde(default)]
    pub fragment: String,
    /// What its Capabilities would answer, relative to `mock/`: one file,
    /// or several when it reads several — written `"fixtures/a.json"` or
    /// `["fixtures/a.json", "fixtures/b.json"]` (contract N), one written
    /// back as one.
    #[serde(default, with = "one_or_many", skip_serializing_if = "Vec::is_empty")]
    pub fixture: Vec<String>,
    /// The Capabilities it reads.
    #[serde(default)]
    pub needs: Vec<String>,
    /// The `DataCapabilities` a user action may trigger.
    #[serde(default)]
    pub actions: Vec<String>,
    /// The props it takes, `name: type`.
    #[serde(default)]
    pub props: std::collections::BTreeMap<String, String>,
    /// The Micro-UI's folder it is bundled from, when it is the real code.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// What the bundle was built from, as `mock/build.mjs` wrote it.
    #[serde(
        default,
        rename = "builtFrom",
        skip_serializing_if = "String::is_empty"
    )]
    pub built_from: String,
}

/// A field contract N writes as one string or a list of them.
mod one_or_many {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Written {
        One(String),
        Many(Vec<String>),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(read: D) -> Result<Vec<String>, D::Error> {
        Ok(match Written::deserialize(read)? {
            Written::One(one) if one.trim().is_empty() => Vec::new(),
            Written::One(one) => vec![one],
            Written::Many(many) => many,
        })
    }

    // serde hands `serialize_with` a reference to the field, whatever its type.
    #[allow(clippy::ptr_arg)]
    pub fn serialize<S: Serializer>(values: &Vec<String>, write: S) -> Result<S::Ok, S::Error> {
        match values.as_slice() {
            [one] => one.serialize(write),
            many => many.serialize(write),
        }
    }
}

/// The mock-up of one branch, as it is right now: what the committed branch
/// carries, or what its workspace holds once a session edited it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MockState {
    /// The branch asked for.
    pub branch: String,
    /// `mock/pages.json` is there: the product has a mock-up.
    pub present: bool,
    /// Its pages, in the manifest's order.
    pub pages: Vec<MockPage>,
    /// Its micro-frontends, in the manifest's order.
    #[serde(default)]
    pub components: Vec<MockComponent>,
    /// A workspace holds changes under `mock/` no commit carries yet.
    pub edited: bool,
    /// The `mockup/*` branch the workspace was saved to, if it was.
    #[serde(default)]
    pub saved: String,
    /// A discarded workspace is kept: `restore` brings it back.
    #[serde(default)]
    pub discarded: bool,
    /// Changes whenever a file of the mock-up changes: what the page reloads on.
    pub stamp: String,
}

/// The product's mock-up: its pages and files on every branch, a workspace
/// a session edits, and the branch the human saves that workspace to.
///
/// Synchronous and blocking (`git`): a handler runs it on a blocking thread.
/// `Send + Sync` because the handlers share it.
pub trait Mockups: Send + Sync {
    /// The mock-up of `branch`: from its workspace when one exists, else
    /// from the branch as committed.
    ///
    /// # Errors
    ///
    /// The repository cannot be reached or read.
    fn state(&self, branch: &str) -> Result<MockState, String>;

    /// The bytes of `mock/<path>` on `branch` — from its workspace when one
    /// exists — or `None` when there is no such file.
    ///
    /// # Errors
    ///
    /// The repository cannot be read.
    fn file(&self, branch: &str, path: &str) -> Result<Option<Vec<u8>>, String>;

    /// The folder a session edits `branch`'s mock-up in, made on first use:
    /// a checkout of the branch, detached, kept until it is saved or
    /// discarded.
    ///
    /// # Errors
    ///
    /// The checkout could not be made.
    fn workspace(&self, branch: &str) -> Result<std::path::PathBuf, String>;

    /// Commits what the workspace changed under `mock/` on a new branch
    /// `mockup/<name>`, pushes it, and answers the branch's name.
    ///
    /// # Errors
    ///
    /// No workspace, nothing changed, or `git` refused.
    fn save(&self, branch: &str, name: &str) -> Result<String, String>;

    /// Drops the workspace and what it changed: the branch's mock-up is
    /// shown as committed again. What it changed is kept aside, the last
    /// discard only, for `restore`.
    ///
    /// # Errors
    ///
    /// The workspace could not be kept aside or removed.
    fn discard(&self, branch: &str) -> Result<(), String>;

    /// Brings the last discarded workspace back, its changes uncommitted
    /// again over the branch as it is now.
    ///
    /// # Errors
    ///
    /// Nothing was discarded, a workspace edits the branch already, or the
    /// checkout failed.
    fn restore(&self, branch: &str) -> Result<(), String>;

    /// Writes `mock/<path>` in the branch's workspace — made first if it is
    /// not there — with `bytes`: a page as the draft shows it, a forked
    /// fragment, the draft's own file.
    ///
    /// # Errors
    ///
    /// The workspace could not be made or written.
    fn write(&self, branch: &str, path: &str, bytes: &[u8]) -> Result<(), String>;

    /// Removes `mock/<path>` from the branch's workspace, if it is there.
    ///
    /// # Errors
    ///
    /// The workspace could not be made or written.
    fn remove(&self, branch: &str, path: &str) -> Result<(), String>;

    /// Every branch of the product, the data archive's aside.
    ///
    /// # Errors
    ///
    /// The repository cannot be read.
    fn branches(&self) -> Result<Vec<String>, String>;

    /// Every file of the mock-up of `branch`, oldest written first: from
    /// its workspace when one exists — what a session is writing — else as
    /// committed.
    ///
    /// # Errors
    ///
    /// The repository cannot be read.
    fn files(&self, branch: &str) -> Result<Vec<MockFile>, String>;

    /// Makes `branch` exist, from `from` when it does not yet — the design
    /// system's branch is born on the first visit — and pushes it.
    ///
    /// # Errors
    ///
    /// Neither branch exists, or `git` refused.
    fn ensure_branch(&self, branch: &str, from: &str) -> Result<(), String>;

    /// Commits what the branch's workspace changed under `mock/` **on the
    /// branch itself** and pushes it — the design system's branch is the
    /// one truth, never saved aside as `mockup/*`. Answers the commit.
    ///
    /// # Errors
    ///
    /// No workspace, nothing changed, or `git` refused.
    fn publish(&self, branch: &str, message: &str) -> Result<String, String>;

    /// Carries the `folders` of `mock/` as the commit `from` has them onto
    /// the branch `to`, in one commit of their own pushed there: what `to`
    /// has elsewhere stays, a file gone from `from` goes. Answers the
    /// commit, `None` when `to` already had them so.
    ///
    /// # Errors
    ///
    /// No branch `to`, or `git` refused — the push tried again on a branch
    /// that moved meanwhile.
    fn carry(
        &self,
        from: &str,
        to: &str,
        folders: &[&str],
        message: &str,
    ) -> Result<Option<String>, String>;
}

/// Where a page's draft versions are kept (`domain::drafts`).
///
/// Synchronous, like [`NoteStore`]: the same small local database.
pub trait DraftStore: Send + Sync {
    /// Every board that holds a version, sorted.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn boards(&self) -> Result<Vec<String>, String>;

    /// A board's versions, oldest first.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn versions(&self, board: &str) -> Result<Vec<crate::domain::drafts::Draft>, String>;

    /// Keeps a version — a new one, or a new state of one.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn put(&self, board: &str, draft: &crate::domain::drafts::Draft) -> Result<(), String>;

    /// Destroys a version for good.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn destroy(&self, board: &str, version: u32) -> Result<(), String>;
}
