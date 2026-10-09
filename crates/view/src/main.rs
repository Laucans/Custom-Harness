#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// Same reason as at the root of `harness-core`: the board is read through
// core's `?Send` ports (migration decision #3), so the poller that holds an
// `Rc<dyn Board>` across an `.await` is non-`Send` by construction.
#![allow(clippy::future_not_send)]

//! The factory view: a local web server that renders the harness as an
//! isometric plant, read from the traces the harness leaves behind.
//!
//! Two pollers, one server, one steward, on one thread. The pollers read the
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
mod desk;
mod domain;
mod ports;
mod server;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser as _;
use harness_core::adapters::shell::github::GhCli;
use harness_core::domain::Slug;
use harness_core::domain::workspace::Workspace;
use harness_core::ports::shell::github::GitHub;
use tokio::sync::watch;

use crate::adapters::claude_bin;
use crate::adapters::fs_traces::FsTraces;
use crate::adapters::gh_board::GhBoard;
use crate::adapters::limits_cli::CliLimits;
use crate::adapters::pty::Pty;
use crate::adapters::watch_proc::WatchProcess;
use crate::cli::Cli;
use crate::desk::Desk;
use crate::domain::assemble::{self, Inputs};
use crate::domain::blueprint;
use crate::domain::limits::Read;
use crate::domain::observe::{Observed, observe};
use crate::domain::plant;
use crate::domain::snapshot::{Project, Snapshot};
use crate::domain::steward;
use crate::ports::{Board, BoardReading, Limits, Plant, TerminalFactory, Traces};
use crate::server::{AppState, router};
use harness_core::adapters::store::events::SqliteEvents;
use harness_core::domain::quota::Reading;
use harness_core::ports::store::events::EventLog;

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
    let board: Option<Rc<dyn Board>> = (!cli.no_board).then(|| -> Rc<dyn Board> {
        // A named target needs no checkout; an unnamed one reads this
        // checkout's own `origin`, as `harness watch` does.
        let gh: Rc<dyn GitHub> = Slug::parse(&cli.target_repo_url).map_or_else(
            || Rc::new(GhCli::new(&root)) as Rc<dyn GitHub>,
            |slug| Rc::new(GhCli::for_slug(&slug)) as Rc<dyn GitHub>,
        );
        Rc::new(GhBoard::new(gh))
    });
    let desk = desk_for(&cli, &root, &project);
    // A demo replays a finished run: there is no plant to switch.
    let plant: Option<Arc<dyn Plant>> = (!cli.demo).then(|| {
        Arc::new(WatchProcess::new(workspace.clone(), &cli.watch_command)) as Arc<dyn Plant>
    });
    let events = event_store(&workspace);
    let claude_read = Arc::default();

    // One per process: what tells a page the server it talks to was rebuilt.
    let build = jiff::Timestamp::now().to_string();
    let mut first = first_picture(&project, cli.demo);
    first.build.clone_from(&build);
    let (snapshot_tx, snapshot_rx) = watch::channel(Arc::new(first));
    let (board_tx, board_rx) = watch::channel(None);
    let state = AppState {
        snapshot: snapshot_rx,
        board: board_rx.clone(),
        traces: Arc::clone(&traces),
        static_dir: cli.static_dir.clone(),
        render_dir: root.join("crates/view/static/render"),
        desk: desk.clone(),
        plant: plant.clone(),
        start_grace: Duration::from_millis(1500),
        limits: Some(Arc::new(CliLimits::new(
            claude_bin::newest().map_or_else(|| PathBuf::from("claude"), |(path, _)| path),
        )) as Arc<dyn Limits>),
        claude_read: Arc::clone(&claude_read),
    };

    let listener = tokio::net::TcpListener::bind((cli.bind.as_str(), cli.port))
        .await
        .with_context(|| format!("binding {}:{}", cli.bind, cli.port))?;
    println!(
        "harness-view: http://{}:{} — reading {}{}{}",
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
        }
    );

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let traces_task = tokio::task::spawn_local(poll_traces(
                traces,
                Hands {
                    plant,
                    events,
                    claude_read,
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
            axum::serve(listener, router(state))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await
                .context("serving the page")?;
            traces_task.abort();
            if let Some(task) = board_task {
                task.abort();
            }
            if let Some(desk) = desk {
                desk.shutdown();
            }
            Ok(())
        })
        .await
}

/// The steward's desk, unless `--no-steward`: the newest Claude Code on the
/// machine, on the model the human chose, briefed on this plant.
fn desk_for(cli: &Cli, root: &Path, project: &Project) -> Option<Desk> {
    (!cli.no_steward).then(|| {
        // `claude` on PATH is often a stale wrapper: when the human named no
        // other program, the newest Claude Code on the machine takes the desk.
        let command = if cli.steward_command == "claude" {
            claude_bin::newest().map_or_else(
                || cli.steward_command.clone(),
                |(path, version)| {
                    println!("harness-view: steward runs {} ({version})", path.display());
                    path.to_string_lossy().into_owned()
                },
            )
        } else {
            cli.steward_command.clone()
        };
        let terminal: Arc<dyn TerminalFactory> = Arc::new(Pty::new(
            root,
            &command,
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

/// The picture before the first read: an empty plant with the right name.
fn first_picture(project: &Project, demo: bool) -> Snapshot {
    let observed = Observed::default();
    assemble::snapshot(&Inputs {
        observed: &observed,
        board: None,
        project,
        lines: &blueprint::lines(),
        now: jiff::Timestamp::now().as_second(),
        demo,
    })
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
}

/// How far back the notifications look.
const NOTIFY_SINCE: jiff::SignedDuration = jiff::SignedDuration::from_hours(24);

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
    harness_notify::feed(
        &events,
        &harness_notify::Facts {
            watch_running: observed.watch_process,
            waiting,
            claude,
            claude_at,
            now: u64::try_from(now.as_second()).unwrap_or(0),
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
        let mut snap = assemble::snapshot(&Inputs {
            observed: &observed,
            board: reading.as_deref(),
            project: &project,
            lines: &lines,
            now: now.as_second(),
            demo,
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
