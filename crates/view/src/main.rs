#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// Same reason as at the root of `harness-core`: the board is read through
// core's `?Send` ports (migration decision #3), so the poller that holds an
// `Rc<dyn Board>` across an `.await` is non-`Send` by construction.
#![allow(clippy::future_not_send)]

//! The factory view: a local web server that renders the harness as an
//! isometric plant, read from the traces the harness leaves behind.
//!
//! Two pollers, one server, one steward, one doctor, one janitor, on one thread. The pollers read the
//! traces every few seconds and the GitHub board every minute, and publish a
//! `Snapshot` on a `watch` channel; the server hands that picture to the page
//! and streams every change to it. The steward is an interactive Claude Code
//! in a pseudo-terminal, bridged to a terminal pane on the page.
//!
//! The view itself **writes nothing** the harness reads: it is a second outer
//! ring beside `harness-launcher`. What changes the plant is the human — at
//! the steward's terminal, or with the page's switch, which starts the watch
//! and signals it, and does nothing else.

mod adapters;
mod cli;
mod data_desk;
mod desk;
mod domain;
mod janitor;
mod note_desks;
mod ports;
mod server;
mod units_desk;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser as _;
use harness_adapters::shell::github::GhCli;
use harness_core::domain::Slug;
use harness_core::domain::workspace::Workspace;
use harness_core::ports::shell::github::GitHub;
use jiff::tz::TimeZone;
use tokio::sync::watch;

use crate::adapters::claude_bin;
use crate::adapters::claude_cli::{ClaudeOneshot, ClaudeTranscripts};
use crate::adapters::fs_traces::FsTraces;
use crate::adapters::fs_yard::FsYard;
use crate::adapters::gh_board::GhBoard;
use crate::adapters::git_units::{DirUnits, GitUnits, Mirror};
use crate::adapters::harness_builds::HarnessBuilds;
use crate::adapters::limits_cli::CliLimits;
use crate::adapters::mockups::{DirMockups, GitMockups};
use crate::adapters::product::{DirProduct, GhProduct};
use crate::adapters::prompt_files::PromptFiles;
use crate::adapters::pty::Pty;
use crate::adapters::pty::PtyMaker;
use crate::adapters::sqlite_notes::SqliteNotes;
use crate::adapters::trial_proc::TrialProcess;
use crate::adapters::watch_proc::WatchProcess;
use crate::cli::Cli;
use crate::data_desk::DataDesk;
use crate::desk::Desk;
use crate::domain::assemble::{self, Inputs};
use crate::domain::blueprint;
use crate::domain::cleanup;
use crate::domain::doctor;
use crate::domain::gates::{self, Told};
use crate::domain::limits::Read;
use crate::domain::observe::{Observed, observe};
use crate::domain::plant;
use crate::domain::snapshot::{Project, Snapshot};
use crate::domain::steward;
use crate::janitor::Janitor;
use crate::note_desks::NoteDesks;
use crate::ports::{
    Board, BoardReading, Builds, Limits, Mockups, NoteStore, Oneshot, Plant, Product,
    TerminalFactory, Traces, Trials, Units, Yard,
};
use crate::server::{AppState, router};
use crate::units_desk::UnitsDesk;
use harness_adapters::store::events::SqliteEvents;
use harness_core::domain::quota::Reading;
use harness_core::ports::store::events::EventLog;

/// The watch switch — none under `--demo`: a demo replays a finished run,
/// there is no plant to switch.
fn plant_for(cli: &Cli, workspace: &Workspace) -> Option<Arc<dyn Plant>> {
    (!cli.demo).then(|| {
        Arc::new(WatchProcess::new(workspace.clone(), &cli.watch_command)) as Arc<dyn Plant>
    })
}

/// The prompts' dry-runs — none under `--demo`, where no run is real.
fn trials(cli: &Cli, workspace: &Workspace) -> Option<Arc<dyn Trials>> {
    (!cli.demo).then(|| {
        Arc::new(TrialProcess::new(workspace.clone(), &cli.trial_command)) as Arc<dyn Trials>
    })
}

/// `current_thread`, like the harness: the board port is `?Send`, and one
/// human watching one plant needs no second thread.
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_target(false).init();
    let cli = Cli::parse();
    let root = repo_root().map_err(anyhow::Error::msg)?;
    let workspace = Workspace::new(&root);
    let traces: Arc<dyn Traces> = Arc::new(FsTraces::new(workspace.clone()));
    let project = project_of(&cli, &root);
    let (board, product) = github_for(&cli, &root, &project);
    let program = program_for(&cli);
    let (pins, note_desks) = notes_for(&cli, &root, &project, &program, &workspace)?;
    let desk = desk_for(&cli, &root, &project, &program);
    let doctor = doctor_for(&cli, &root, &project, &program);
    let plant = plant_for(&cli, &workspace);
    let events = event_store(&workspace);
    let janitor = janitor_for(&cli, &workspace);
    let claude_read = Arc::default();
    let diagnoses: server::Diagnoses = Arc::default();
    let (data, data_task) = data_for(product);
    let (units, mockups) = product_readers(&cli, &root, &project);

    let (snapshot_tx, snapshot_rx) = watch::channel(Arc::new(first_picture(&project, cli.demo)));
    let (board_tx, board_rx) = watch::channel(None);
    let state = AppState {
        snapshot: snapshot_rx,
        board: board_rx.clone(),
        traces: Arc::clone(&traces),
        static_dir: cli.static_dir.clone(),
        render_dir: root.join("crates/view/static/render"),
        desk: desk.clone(),
        doctor: doctor.clone(),
        diagnoses: Arc::clone(&diagnoses),
        asking: Arc::default(),
        plant: plant.clone(),
        start_grace: Duration::from_millis(1500),
        limits: Some(limits()),
        claude_read: Arc::clone(&claude_read),
        events: events.clone(),
        janitor: janitor.clone(),
        data,
        pins: Arc::clone(&pins),
        note_desks: note_desks.clone(),
        builds: builds_for(&cli, &root),
        integration_branch: cli.integration_branch.clone(),
        prompts: Arc::new(PromptFiles::new(workspace.state_root())),
        prompts_lock: Arc::default(),
        trials: trials(&cli, &workspace),
        restart: Arc::default(),
        restart_poll: Duration::from_secs(3),
        units,
        mockups,
        zone: TimeZone::system(),
    };

    let listener = tokio::net::TcpListener::bind((cli.bind.as_str(), cli.port))
        .await
        .with_context(|| format!("binding {}:{}", cli.bind, cli.port))?;
    println!("{}", banner(&cli, &workspace));

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let traces_task = tokio::task::spawn_local(poll_traces(
                traces,
                Hands {
                    plant,
                    events,
                    claude_read,
                    diagnoses,
                },
                board_rx,
                project,
                snapshot_tx,
                Duration::from_secs(cli.interval.max(1)),
                cli.demo,
            ));
            let board_task = board.map(|board| {
                tokio::task::spawn_local(poll_board(
                    board,
                    board_tx,
                    Duration::from_secs(cli.board_interval.max(5)),
                ))
            });
            let janitor_task = janitor.map(|janitor| tokio::task::spawn_local(chronic(janitor)));
            let data_task = data_task.map(tokio::task::spawn_local);
            axum::serve(listener, router(state))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await
                .context("serving the page")?;
            traces_task.abort();
            for task in [board_task, janitor_task, data_task].into_iter().flatten() {
                task.abort();
            }
            for desk in [desk, doctor].into_iter().flatten() {
                desk.shutdown();
            }
            if let Some(desks) = note_desks {
                desks.shutdown();
            }
            Ok(())
        })
        .await
}

/// The line printed when the page is up: where it listens, what it reads,
/// who is in.
fn banner(cli: &Cli, workspace: &Workspace) -> String {
    format!(
        "harness-view: http://{}:{} — reading {}{}{}{}{}",
        cli.bind,
        cli.port,
        workspace.rel(&workspace.logs()),
        if cli.demo {
            " (demo: the latest run is shown live)"
        } else {
            ""
        },
        if cli.no_steward {
            ""
        } else {
            " — the steward answers at the desk"
        },
        if cli.no_doctor {
            ""
        } else {
            " — the doctor is in"
        },
        if cli.no_janitor {
            ""
        } else {
            " — the janitor minds the yard"
        }
    )
}

/// The harness's commands the data pane starts, unless `--demo`, which has
/// no product to build. The binary: `--harness-bin`, else a `harness` beside
/// this view's executable — built with it, so it knows the same commands —
/// else the program the switch starts the watch with.
fn builds_for(cli: &Cli, root: &Path) -> Option<Arc<dyn Builds>> {
    if cli.demo {
        return None;
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("harness")));
    let watched = cli.watch_command.split_whitespace().next().map(|first| {
        let first = Path::new(first);
        if first.is_relative() && first.components().count() > 1 {
            root.join(first)
        } else {
            first.to_path_buf()
        }
    });
    let candidates: Vec<PathBuf> = cli.harness_bin.clone().map_or_else(
        || beside.into_iter().chain(watched).collect(),
        |chosen| vec![chosen],
    );
    Some(Arc::new(HarnessBuilds::new(candidates, root.to_path_buf())) as Arc<dyn Builds>)
}

/// The rate limits, read through the newest Claude Code and `gh`.
fn limits() -> Arc<dyn Limits> {
    Arc::new(CliLimits::new(
        claude_bin::newest().map_or_else(|| PathBuf::from("claude"), |(path, _)| path),
    ))
}

/// The janitor, unless `--no-janitor`.
fn janitor_for(cli: &Cli, workspace: &Workspace) -> Option<Janitor> {
    (!cli.no_janitor)
        .then(|| Janitor::new(Arc::new(FsYard::new(workspace.state_root())) as Arc<dyn Yard>))
}

/// The pins' store, and their AI sessions.
fn notes_for(
    cli: &Cli,
    root: &Path,
    project: &Project,
    program: &str,
    workspace: &Workspace,
) -> anyhow::Result<(Arc<dyn NoteStore>, Option<NoteDesks>)> {
    let pins: Arc<dyn NoteStore> = Arc::new(
        SqliteNotes::open(
            &workspace.events(),
            &workspace.state_root().join(".llocal/annotations.json"),
        )
        .map_err(anyhow::Error::msg)
        .context("opening the pins' store")?,
    );
    let desks = note_desks_for(cli, root, project, program, &pins);
    Ok((pins, desks))
}

/// The pins' AI sessions, unless `--no-steward`: Claude Code on the
/// steward's model, in the product's snapshot clone when there is one — a
/// brainstorm reads the product, not the harness.
fn note_desks_for(
    cli: &Cli,
    root: &Path,
    project: &Project,
    program: &str,
    pins: &Arc<dyn NoteStore>,
) -> Option<NoteDesks> {
    if cli.no_steward {
        return None;
    }
    let clone = root
        .join(".llocal/data-snapshot")
        .join(project.slug.replace('/', "-"));
    let cwd = if project.slug.is_empty() || !clone.is_dir() {
        root.to_path_buf()
    } else {
        clone
    };
    let claude = claude_bin::newest().map_or_else(|| PathBuf::from(program), |(path, _)| path);
    let projects = std::env::var_os("HOME").map_or_else(
        || PathBuf::from(".claude/projects"),
        |home| PathBuf::from(home).join(".claude/projects"),
    );
    Some(NoteDesks::new(note_desks::Setup {
        store: Arc::clone(pins),
        maker: Arc::new(PtyMaker::new(program)),
        oneshot: Arc::new(ClaudeOneshot::new(claude)),
        transcripts: Arc::new(ClaudeTranscripts::new(projects)),
        model: cli.steward_model.clone(),
        cwd,
        permission_mode: cli.permission_mode.clone(),
    }))
}

/// The Claude Code both desks run. `claude` on PATH is often a stale
/// wrapper: when the human named no other program, the newest Claude Code on
/// the machine takes the desks.
fn program_for(cli: &Cli) -> String {
    if cli.steward_command != "claude" || (cli.no_steward && cli.no_doctor) {
        return cli.steward_command.clone();
    }
    claude_bin::newest().map_or_else(
        || cli.steward_command.clone(),
        |(path, version)| {
            println!("harness-view: the desks run {} ({version})", path.display());
            path.to_string_lossy().into_owned()
        },
    )
}

/// The steward's desk, unless `--no-steward`: on the model the human chose,
/// briefed on this plant.
fn desk_for(cli: &Cli, root: &Path, project: &Project, program: &str) -> Option<Desk> {
    (!cli.no_steward).then(|| {
        let terminal: Arc<dyn TerminalFactory> = Arc::new(Pty::new(
            root,
            program,
            vec![
                "--model".to_string(),
                cli.steward_model.clone(),
                "--permission-mode".to_string(),
                cli.permission_mode.clone(),
                "--append-system-prompt".to_string(),
                steward::briefing(project),
            ],
        ));
        Desk::new(terminal)
    })
}

/// The doctor's desk, unless `--no-doctor`: the same program on the doctor's
/// model and effort, briefed on the instruments, the check-up already asked.
fn doctor_for(cli: &Cli, root: &Path, project: &Project, program: &str) -> Option<Desk> {
    (!cli.no_doctor).then(|| {
        let terminal: Arc<dyn TerminalFactory> = Arc::new(Pty::new(
            root,
            program,
            vec![
                "--model".to_string(),
                cli.doctor_model.clone(),
                "--effort".to_string(),
                cli.doctor_effort.clone(),
                "--permission-mode".to_string(),
                cli.permission_mode.clone(),
                "--append-system-prompt".to_string(),
                doctor::briefing(project),
                // The positional prompt: sent the moment the session opens.
                doctor::CHECKUP.to_string(),
            ],
        ));
        Desk::new(terminal)
    })
}

/// The picture before the first read: an empty plant with the right name and
/// the process's `build`.
fn first_picture(project: &Project, demo: bool) -> Snapshot {
    // One per process: what tells a page the server it talks to was rebuilt.
    let build = jiff::Timestamp::now().to_string();
    let build = build.as_str();
    let observed = Observed::default();
    let mut first = assemble::snapshot(&Inputs {
        observed: &observed,
        board: None,
        project,
        lines: &blueprint::lines(),
        now: jiff::Timestamp::now().as_second(),
        demo,
        told: &Told::default(),
        zone: &TimeZone::system(),
    });
    first.build = build.to_string();
    first
}

/// Reads the traces on a cadence and publishes the picture when it changed.
/// The harness's events, shared with every run of the checkout. Opening
/// creates the store when no run has yet: an empty table, nothing a run reads
/// differently.
fn event_store(workspace: &Workspace) -> Option<Arc<dyn EventLog + Send + Sync>> {
    match SqliteEvents::open(&workspace.events()) {
        Ok(store) => Some(Arc::new(store)),
        Err(why) => {
            eprintln!("harness-view: no notifications — {}", why.reason());
            None
        }
    }
}

/// What the traces poller reads besides the traces: the watch process, the
/// event store, and the last Claude reading the page paid for.
struct Hands {
    plant: Option<Arc<dyn Plant>>,
    events: Option<Arc<dyn EventLog + Send + Sync>>,
    claude_read: Arc<std::sync::Mutex<Option<Read<Reading>>>>,
    diagnoses: server::Diagnoses,
}

/// How far back the notifications look.
const NOTIFY_SINCE: jiff::SignedDuration = jiff::SignedDuration::from_hours(24);

/// How many of the newest events a tick reads for the gates.
pub const EVENTS_READ: usize = 5_000;

/// What to tell the human now: the last day's events, and what is observed.
fn notifications(
    hands: &Hands,
    observed: &Observed,
    board: Option<&BoardReading>,
    now: jiff::Timestamp,
) -> Vec<harness_notify::Notification> {
    let clock = |at: jiff::Timestamp| at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    let since = clock(now - NOTIFY_SINCE);
    let events = hands
        .events
        .as_ref()
        .and_then(|store| store.between(Some(&since), None, 5_000).ok())
        .unwrap_or_default();
    let waiting = board.map_or_else(Vec::new, |board| {
        harness_notify::waiting_on_a_human(
            board.roadmap.iter().chain(
                board
                    .milestones
                    .iter()
                    .flat_map(|m| std::iter::once(&m.issue).chain(m.tasks.iter())),
            ),
        )
    });
    // The freshest of what the runs kept and what the page probed.
    let probed = hands
        .claude_read
        .lock()
        .ok()
        .and_then(|read| read.as_ref().and_then(|read| read.value.clone()));
    let claude = [observed.quota.clone(), probed]
        .into_iter()
        .flatten()
        .max_by_key(|reading| reading.at);
    let claude_at = claude
        .as_ref()
        .and_then(|reading| jiff::Timestamp::from_second(i64::try_from(reading.at).ok()?).ok())
        .map(clock)
        .unwrap_or_default();
    // The doctor's diagnoses that ended, answered or not.
    let diagnoses = hands
        .diagnoses
        .lock()
        .map(|map| {
            map.iter()
                .filter_map(|(key, diagnosis)| {
                    let (workflow, run) = key.split_once('/')?;
                    let (answered, at) = match diagnosis {
                        doctor::Diagnosis::Done { at, .. } => (true, at),
                        doctor::Diagnosis::Lost { at, .. } => (false, at),
                        doctor::Diagnosis::Running { .. } => return None,
                    };
                    Some(harness_notify::Diagnosed {
                        workflow: workflow.to_string(),
                        run: run.to_string(),
                        answered,
                        at: at.clone(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    harness_notify::feed(
        &events,
        &harness_notify::Facts {
            watch_running: observed.watch_process,
            waiting,
            claude,
            claude_at,
            now: u64::try_from(now.as_second()).unwrap_or(0),
            diagnoses,
        },
    )
}

async fn poll_traces(
    traces: Arc<dyn Traces>,
    hands: Hands,
    board: watch::Receiver<Option<Arc<BoardReading>>>,
    project: Project,
    tx: watch::Sender<Arc<Snapshot>>,
    interval: Duration,
    demo: bool,
) {
    let lines = blueprint::lines();
    let zone = TimeZone::system();
    // The first picture carries this process's build; every later one too.
    let build = tx.borrow().build.clone();
    let mut last_body = String::new();
    loop {
        let mut observed = observe(&*traces, &lines);
        observed.watch_process = hands
            .plant
            .as_ref()
            .map(|plant| plant::running(plant.as_ref()).is_some());
        let reading = board.borrow().clone();
        let now = jiff::Timestamp::now();
        // The newest events, whatever their age: a gate's last verdict is
        // worth showing days later, and the store is a local SQLite file.
        let told = hands
            .events
            .as_ref()
            .and_then(|store| store.between(None, None, EVENTS_READ).ok())
            .map(|events| gates::told(&events))
            .unwrap_or_default();
        let mut snap = assemble::snapshot(&Inputs {
            observed: &observed,
            board: reading.as_deref(),
            project: &project,
            lines: &lines,
            now: now.as_second(),
            demo,
            told: &told,
            zone: &zone,
        });
        snap.build.clone_from(&build);
        snap.notifications = notifications(&hands, &observed, reading.as_deref(), now);
        // Compared before it is stamped: a picture that only differs by its
        // clock is the same picture, and pushing it would wake every page for
        // nothing.
        let body = serde_json::to_string(&snap).unwrap_or_default();
        if body != last_body {
            snap.at = now.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
            tx.send_replace(Arc::new(snap));
            last_body = body;
        }
        tokio::time::sleep(interval).await;
    }
}

/// The janitor's chronic round: the yard weighed at start and every two hours,
/// swept on their own when allowed and past the threshold.
async fn chronic(janitor: Janitor) {
    loop {
        let next = jiff::Timestamp::now()
            .checked_add(jiff::SignedDuration::try_from(cleanup::CHRONIC).unwrap_or_default())
            .unwrap_or_else(|_| jiff::Timestamp::now());
        janitor.plan(next.strftime("%Y-%m-%dT%H:%M:%SZ").to_string());
        if let Some(report) = janitor.round().await {
            tracing::info!(
                "the janitor swept on their own: {} freed, {} heap(s) gone, {} refused",
                cleanup::human(report.freed),
                report.removed.len(),
                report.failed.len()
            );
        }
        tokio::time::sleep(cleanup::CHRONIC).await;
    }
}

/// Where the data model is read, and how often.
type DataSource = (Rc<dyn Product>, Duration);

/// What the view reads on GitHub — the board, and the product's data
/// model — unless `--no-board`; the data model may come from a folder.
fn github_for(
    cli: &Cli,
    root: &Path,
    project: &Project,
) -> (Option<Rc<dyn Board>>, Option<DataSource>) {
    let gh: Option<Rc<dyn GitHub>> = (!cli.no_board).then(|| {
        // A named target needs no checkout; an unnamed one reads this
        // checkout's own `origin`, as `harness watch` does.
        Slug::parse(&cli.target_repo_url).map_or_else(
            || Rc::new(GhCli::new(root)) as Rc<dyn GitHub>,
            |slug| Rc::new(GhCli::for_slug(&slug)) as Rc<dyn GitHub>,
        )
    });
    let board = gh
        .clone()
        .map(|gh| Rc::new(GhBoard::new(gh)) as Rc<dyn Board>);
    (board, product_for(cli, root, project, gh.as_ref()))
}

/// Where the sample data model lies, for `--demo`.
const SAMPLE_DATA: &str = "crates/view/sample";

/// Where the product's data model is read, and how often: a folder the
/// human named, the sample under `--demo`, else GitHub at the integration
/// branch — nothing under `--no-board`.
fn product_for(
    cli: &Cli,
    root: &Path,
    project: &Project,
    gh: Option<&Rc<dyn GitHub>>,
) -> Option<DataSource> {
    let local = cli
        .data_dir
        .clone()
        .or_else(|| cli.demo.then(|| root.join(SAMPLE_DATA)));
    if let Some(dir) = local {
        // The sample carries an archive too, laid out as the branch is.
        let archive = (cli.data_dir.is_none()).then(|| dir.join("archive"));
        return Some((
            Rc::new(DirProduct::new(
                &dir,
                archive.as_deref(),
                &cli.integration_branch,
            )),
            Duration::from_secs(5),
        ));
    }
    let gh = Rc::clone(gh?);
    Some((
        Rc::new(GhProduct::new(gh, &project.slug, &cli.integration_branch)),
        Duration::from_secs(cli.board_interval.max(5)),
    ))
}

/// The data desk and the task that serves it, when the product can be read.
fn data_for(product: Option<DataSource>) -> (Option<DataDesk>, Option<impl Future<Output = ()>>) {
    product
        .map(|(product, every)| data_desk::open(product, every, TimeZone::system()))
        .unzip()
}

/// Where the sample units lie, for `--demo`: one folder per branch.
const SAMPLE_UNITS: &str = "crates/view/sample/units";

/// Where the capabilities list is read: the folder the human named, the
/// sample under `--demo`, else a mirror of the product on GitHub — nothing
/// under `--no-board` without a folder.
fn units_for(cli: &Cli, root: &Path, mirror: Option<Arc<Mirror>>) -> Option<UnitsDesk> {
    let units: Arc<dyn Units> = if cli.demo && cli.data_dir.is_none() {
        Arc::new(DirUnits::new(&root.join(SAMPLE_UNITS)))
    } else {
        Arc::new(GitUnits::new(mirror?))
    };
    let claude = claude_bin::newest().map(|(path, _)| path);
    let oneshot = claude.map(|path| Arc::new(ClaudeOneshot::new(path)) as Arc<dyn Oneshot>);
    Some(UnitsDesk::new(
        units,
        Duration::from_secs(60),
        &cli.integration_branch,
        oneshot,
        &cli.fusion_model,
    ))
}

/// What reads the product with `git`: the capabilities list and the
/// mock-up, over one mirror.
fn product_readers(
    cli: &Cli,
    root: &Path,
    project: &Project,
) -> (Option<UnitsDesk>, Option<Arc<dyn Mockups>>) {
    let mirror = mirror_for(cli, root, project);
    (
        units_for(cli, root, mirror.clone()),
        mockups_for(cli, root, mirror),
    )
}

/// The product's repository as `git` reads it: the folder the human named,
/// else a bare mirror of the product on GitHub, kept under
/// `.llocal/view-mirror/` — nothing under `--no-board` or `--demo` without
/// a folder.
fn mirror_for(cli: &Cli, root: &Path, project: &Project) -> Option<Arc<Mirror>> {
    if let Some(dir) = &cli.data_dir {
        return Some(Arc::new(Mirror::local(dir)));
    }
    if cli.demo || cli.no_board || project.slug.is_empty() {
        return None;
    }
    let dir = root
        .join(".llocal")
        .join(cleanup::VIEW_MIRROR)
        .join(format!("{}.git", project.slug.replace('/', "-")));
    Some(Arc::new(Mirror::bare(
        &format!("https://github.com/{}.git", project.slug),
        &dir,
        Duration::from_secs(60),
    )))
}

/// Where the product's mock-up is read and edited: the sample's folders
/// under `--demo`, else the mirror's branches with a worktree per edited
/// branch under `.llocal/view-mockups/`.
fn mockups_for(cli: &Cli, root: &Path, mirror: Option<Arc<Mirror>>) -> Option<Arc<dyn Mockups>> {
    if cli.demo && cli.data_dir.is_none() {
        return Some(Arc::new(DirMockups::new(&root.join(SAMPLE_UNITS))));
    }
    let workspaces = root.join(".llocal").join(cleanup::VIEW_MOCKUPS);
    mirror.map(|mirror| Arc::new(GitMockups::new(mirror, &workspaces)) as Arc<dyn Mockups>)
}

/// Reads the board on a slower cadence; a failed read keeps the last one.
async fn poll_board(
    board: Rc<dyn Board>,
    tx: watch::Sender<Option<Arc<BoardReading>>>,
    interval: Duration,
) {
    loop {
        match board.read().await {
            Ok(reading) => {
                tracing::info!(
                    "board: {} roadmap item(s), {} milestone(s) on {}",
                    reading.roadmap.len(),
                    reading.milestones.len(),
                    reading.slug
                );
                tx.send_replace(Some(Arc::new(reading)));
            }
            Err(halt) => tracing::warn!("board unreadable: {}", halt.reason()),
        }
        tokio::time::sleep(interval).await;
    }
}

/// The project on the sign: the target repository when one is named, this
/// checkout's folder name otherwise.
fn project_of(cli: &Cli, root: &Path) -> Project {
    Slug::parse(&cli.target_repo_url).map_or_else(
        || Project {
            name: root.file_name().map_or_else(
                || "harness".to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
            slug: String::new(),
            url: String::new(),
            integration_branch: cli.integration_branch.clone(),
        },
        |slug| Project {
            name: slug.name.clone(),
            url: format!("https://github.com/{slug}"),
            slug: slug.to_string(),
            integration_branch: cli.integration_branch.clone(),
        },
    )
}

/// The repository from which the view is launched, by walking up from the
/// current directory — the same rule as the launcher, without a subprocess.
fn repo_root() -> Result<PathBuf, String> {
    let here =
        std::env::current_dir().map_err(|e| format!("cannot read the current directory: {e}"))?;
    walk_up(&here).ok_or_else(|| {
        format!(
            "{} is not inside a git checkout — run harness-view from the harness repository",
            here.display()
        )
    })
}

/// The first ancestor that carries a `.git`, including this one.
fn walk_up(from: &Path) -> Option<PathBuf> {
    from.ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_target_puts_its_name_on_the_sign() {
        let cli = Cli::try_parse_from(["harness-view", "--target-repo-url", "Laucans/dnd_helper"])
            .expect("parses");
        let project = project_of(&cli, Path::new("/somewhere/harness"));
        assert_eq!(project.name, "dnd_helper");
        assert_eq!(project.slug, "Laucans/dnd_helper");
        assert_eq!(project.url, "https://github.com/Laucans/dnd_helper");
        assert_eq!(project.integration_branch, "main_agent");
    }

    #[test]
    fn without_a_target_the_checkout_folder_names_the_plant() {
        let cli = Cli::try_parse_from(["harness-view"]).expect("parses");
        let project = project_of(&cli, Path::new("/somewhere/Custom-Harness"));
        assert_eq!(project.name, "Custom-Harness");
        assert_eq!(project.url, "");
    }

    #[test]
    fn the_repository_is_found_by_walking_up() {
        let here = std::env::current_dir().expect("cwd");
        let root = walk_up(&here).expect("inside a checkout");
        assert!(root.join(".git").exists());
        assert!(walk_up(Path::new("/")).is_none());
    }
}
