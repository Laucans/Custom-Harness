//! The launcher of the first workflow: what wires the real adapters.
//!
//! **The only place in the crate that builds something concrete.** A `GhCli`,
//! a `GitCli`, a `claude` session factory, a ledger on disk — none of this is
//! named elsewhere, and that is what lets the rest have only one test seam per
//! port.
//!
//! **The loop assembly itself no longer lives here.** The `DevLoop`, its round
//! factory, its preflight gates — all of that is generic to the workflow and
//! lives in `harness_workflows::dev_loop::run`, alongside the stage table and
//! the round it composes. This file builds what no workflow has the right to
//! name — the concrete adapters, the tooling gates, the `Cli` — and passes it
//! to `run::build`.
//!
//! The order here is the one that the mount imposes: the tooling gates run
//! **before** the mount because a multi-hundred-megabyte clone must not precede
//! the discovery that `gh` is not authenticated; those that talk about the
//! workspace run **after**, because they talk about what is inside it.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use harness_core::adapters::agent::claude_cli::ClaudeCliFactory;
use harness_core::adapters::agent::rehearsal::Rehearsal;
use harness_core::adapters::shell::disk::RealDisk;
use harness_core::adapters::shell::git::{GitCli, GitRepos};
use harness_core::adapters::shell::github::GhCli;
use harness_core::adapters::shell::process;
use harness_core::adapters::store::checkpoint::Checkpoint;
use harness_core::domain::Verdict;
use harness_core::domain::workspace::{Wanted, Workspace};
use harness_core::execution::provisioning::{Mount, Provisioner, Run};
use harness_core::execution::{Context, Gate, Guarded, Settings, Verification};
use harness_core::ports::agent::SessionFactory;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::{Repo, Repos};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::checkpoint::Checkpoints;
use harness_core::ports::store::spending::Spending;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::dev_loop::config::Config;
use harness_workflows::dev_loop::data::signatures;
use harness_workflows::dev_loop::data::state::Loop;
use harness_workflows::dev_loop::orchestration::stages;
use harness_workflows::dev_loop::ports::Ports;
use harness_workflows::dev_loop::run;

use crate::adapters::sink::Both;
use crate::adapters::spending::{self, LedgerSpending};
use crate::cli::RunArgs;
use crate::dispatch::tooling;

/// What a run ended up doing.
pub struct Ran {
    /// The workflow verdict.
    pub verdict: Verdict,
    /// Where the log was written.
    pub log: PathBuf,
}

/// Runs the loop: preflight, mount, turns, unmount.
///
/// # Errors
///
/// The first [`harness_core::domain::Halt`] that stops the run — a gate that
/// refuses, a session that stops on its own, a quota exhausted. Its
/// `exit_code` is what the caller returns to the system.
pub async fn run(args: &RunArgs, here: &Path) -> harness_core::domain::Outcome<Ran> {
    let run_id = spending::run_id();
    let source = Workspace::new(here);
    let sink = Rc::new(
        Both::new(&source.loop_dir().join(&run_id).join("run.log")).map_err(|e| {
            harness_core::domain::Halt::Failed(format!("log file cannot be opened: {e}"))
        })?,
    );
    let log_path = sink.path().to_path_buf();
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, verbosity(args));

    let disk: Rc<dyn Disk> = Rc::new(RealDisk);
    let repos: Rc<dyn Repos> = Rc::new(GitRepos);
    // Tooling gates first, on the repository from which the run is launched: a
    // clone must not precede the discovery that `gh` is not authenticated.
    let outside: Rc<dyn GitHub> = Rc::new(GhCli::new(source.root()));
    tooling_pre(args, &source, outside)
        .verify(&bare(args, log.clone()))
        .await?;

    let mount = mount(args, &source, &repos, &disk, &run_id, &log).await?;
    let workspace = &mount.workspace;
    // Before the gates, because one of them checks for the skills: the
    // harness lends its own rather than expecting the target repo to carry
    // copies of them (`dispatch::skills`).
    if mount.mounted() {
        crate::dispatch::skills::lend(source.root(), workspace.root(), disk.as_ref(), &log)?;
    }
    // The two target-facing gates run here, against the mounted workspace —
    // D4: once the target comes from a setting rather than the cwd, running
    // them pre-mount would check the *harness's own* branch and CI.
    tooling_post(args, workspace, &disk)
        .verify(&bare(args, log.clone()))
        .await?;
    let provisioner = Provisioner {
        repos: Rc::clone(&repos),
        disk: Rc::clone(&disk),
    };

    let outcome = turns(args, workspace, &run_id, Rc::clone(&disk), &log, &log_path).await;
    // Unmount happens no matter what: it's what keeps a workspace with work,
    // and deletes one that has none.
    provisioner.unmount(&mount, &log).await;
    Ok(Ran {
        verdict: outcome?,
        log: log_path,
    })
}

/// The verbosity level requested for the console. The file keeps everything.
const fn verbosity(args: &RunArgs) -> Verbosity {
    if args.verbose {
        Verbosity::Verbose
    } else if args.quiet {
        Verbosity::Quiet
    } else {
        Verbosity::Normal
    }
}

/// A context with no useful state, for gates that do not read any.
fn bare(args: &RunArgs, log: Logbook) -> Context<Loop> {
    Context::new(settings(args), Loop::default(), log)
}

fn settings(args: &RunArgs) -> Settings {
    Settings {
        dry_run: args.dry_run,
        stages: args.stages.clone(),
    }
}

/// Pre-mount: tools and auth only.
///
/// Nothing here reads the target's branch or CI: once the target comes from
/// a setting rather than this checkout (D4), this checkout is not the right
/// place to ask either question — see [`tooling_post`]. `claude`'s version
/// comes first because it is the only check that costs no network call.
fn tooling_pre(args: &RunArgs, source: &Workspace, gh: Rc<dyn GitHub>) -> Gate<Loop> {
    let git: Rc<dyn Repo> = Rc::new(GitCli::new(source.root()));
    Gate {
        name: "tooling",
        checks: vec![
            Box::new(tooling::ClaudeIsRecentEnough::default()),
            Box::new(tooling::GhIsAuthenticated { gh }),
            Box::new(tooling::WorkingTreeIsClean {
                git,
                // A run that works in a clone does not touch the human's tree:
                // the gate only makes sense in-place.
                allowed: args.allow_dirty || !args.no_workspace,
            }),
        ],
    }
}

/// Post-mount: the two target-facing gates, wired against the mounted
/// workspace.
///
/// Moved here from pre-mount (D4, `docs/to_build.md` §6.3): the target repo
/// no longer has to be the repository the harness was launched from, so
/// checking its branch and CI must wait until the workspace carrying it is
/// known.
fn tooling_post(args: &RunArgs, workspace: &Workspace, disk: &Rc<dyn Disk>) -> Gate<Loop> {
    let git: Rc<dyn Repo> = Rc::new(GitCli::new(workspace.root()));
    Gate {
        name: "tooling (post-mount)",
        checks: vec![
            Box::new(tooling::TheIntegrationBranch {
                git: Rc::clone(&git),
                branch: args.branch.clone(),
                // In a clone, the current branch is set by the mount; requiring
                // it from the human's repository would force them to switch for
                // a run that does not touch their tree.
                check_current: args.no_workspace,
                // A dry run mounts nothing, so nothing put this checkout on the
                // branch. `origin` carrying it is the whole of what is left to
                // ask.
                checked_out: !args.dry_run,
            }),
            Box::new(tooling::CiTriggersOnTheBranch {
                disk: Rc::clone(disk),
                path: workspace.root().join(".github/workflows/ci.yml"),
                branch: args.branch.clone(),
            }),
        ],
    }
}

/// This run's workspace, mounted — or the repository here if we don't want one.
async fn mount(
    args: &RunArgs,
    source: &Workspace,
    repos: &Rc<dyn Repos>,
    disk: &Rc<dyn Disk>,
    run_id: &str,
    log: &Logbook,
) -> harness_core::domain::Outcome<Mount> {
    if args.no_workspace {
        // D4 step 4: a `TARGET_REPO_URL` naming another repo than this
        // checkout's `origin` would otherwise have the sessions edit the
        // harness checkout while the board lives elsewhere.
        if !args.target_repo_url.is_empty() {
            let origin = repos.at(source.root()).remote_url("origin").await?;
            if !harness_core::domain::same_repo(&origin, &args.target_repo_url) {
                return Err(harness_core::domain::Halt::Halted(format!(
                    "TARGET_REPO_URL is {} but this checkout's origin is {origin} — \
                     --no-workspace would edit the wrong repository's board",
                    args.target_repo_url
                )));
            }
        }
        log.say("workspace: none — the run works in this repository");
        return Ok(Mount::in_place(source.clone()));
    }
    // D4 step 1: `--workspace-url` wins when given; `TARGET_REPO_URL`
    // otherwise; empty here still means "this checkout's own `origin`" —
    // `Provisioner::mount`'s own fallback (`url_for`) is unchanged.
    let url = if args.workspace_url.is_empty() {
        args.target_repo_url.clone()
    } else {
        args.workspace_url.clone()
    };
    let wanted = Wanted {
        strategy: args
            .strategy()
            .map_err(harness_core::domain::Halt::Halted)?,
        url,
        id: args.use_workspace.clone(),
        base: args.workspaces_dir.clone(),
        keep: args.keep_workspace,
        force_reset: args.force_reset,
        // `--use-workspace` is a human naming a workspace they already have:
        // a typo must be refused, never cloned beside it.
        create_if_missing: false,
    };
    Provisioner {
        repos: Rc::clone(repos),
        disk: Rc::clone(disk),
    }
    .mount(
        &Run {
            source,
            wanted: &wanted,
            branch: &args.branch,
            run_id,
            dry_run: args.dry_run,
        },
        log,
    )
    .await
}

/// The checkout's build and test configuration, read verbatim off the clone.
///
/// Here and not in the workflow: it is I/O against the mounted workspace, which
/// is this layer's job. Read once per run — it is the same for every stage and
/// every turn, and a block that changed between turns would miss the prompt
/// cache that makes carrying it affordable.
///
/// **A failure is not a failure of the run.** Nothing downstream requires the
/// digest: an empty one puts the session back where it was before this existed,
/// reading the config itself. So a repository git cannot list is logged and
/// skipped, never halted on — the opposite trade from the gates, which stop a
/// run precisely because a session cannot recover from what they catch.
async fn configuration_digest(workspace: &Workspace, disk: &dyn Disk, log: &Logbook) -> String {
    use harness_workflows::dev_loop::data::stack;

    let repo: Rc<dyn Repo> = Rc::new(GitCli::new(workspace.root()));
    let tracked = match repo.tracked_files().await {
        Ok(found) => found,
        Err(why) => {
            log.warn(&format!(
                "the repository's configuration could not be read ({why}) — \
                 sessions will read it themselves, as before"
            ));
            return String::new();
        }
    };
    let found = stack::detect(&tracked);
    if found.is_empty() {
        log.say("no known configuration file in this checkout — no digest injected");
        return String::new();
    }
    log.say(&format!(
        "repository configuration: {} file(s) carried into every prompt ({})",
        found.len(),
        found.join(", ")
    ));
    stack::digest(workspace.root(), disk, &found, Some(log))
}

/// How long `tsc` is given to emit declarations before the index is given up on.
///
/// It measured 1.9 seconds on the target checkout. The timeout is for the case
/// it does not finish at all — a misconfigured project, a cold `node_modules` —
/// and a run must not wait on an index it can do without.
const INDEX_TIMEOUT: Duration = Duration::from_secs(90);

/// Every `.d.ts` under `root`, by the source path it describes.
///
/// `tsc` mirrors the source tree into `--outDir`, so `core/place.d.ts` comes from
/// `src/core/place.ts` — the `src/` prefix is put back by matching against the
/// tracked files rather than assumed, since `rootDir` is a project's to choose.
fn declarations_in(root: &Path, under: &Path, found: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(under) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            declarations_in(root, &path, found);
        } else if path.to_string_lossy().ends_with(".d.ts")
            && let (Ok(relative), Ok(text)) =
                (path.strip_prefix(root), std::fs::read_to_string(&path))
        {
            found.push((relative.to_string_lossy().replace(".d.ts", ".ts"), text));
        }
    }
}

/// The public shape of the checkout, by the mechanism its ecosystem calls for.
///
/// **Which mechanism is a measured decision, not a preference** — see
/// [`signatures`](harness_workflows::dev_loop::data::signatures). Rust is read
/// out of the source text, because a `pub struct` block holds only its fields.
/// TypeScript needs `tsc`, because a `class` block holds its method bodies and
/// no pattern strips those reliably.
///
/// **A failure is never the run's failure.** Nothing downstream requires an
/// index: an empty one leaves the session reading files as it always did. So
/// every branch here logs and returns empty rather than halting — the opposite
/// trade from a gate.
async fn signature_index(
    workspace: &Workspace,
    disk: &dyn Disk,
    log: &Logbook,
) -> signatures::Index {
    let repo: Rc<dyn Repo> = Rc::new(GitCli::new(workspace.root()));
    let Ok(tracked) = repo.tracked_files().await else {
        return signatures::Index::default();
    };
    let ecosystem = signatures::detect(&tracked);
    let by_path = match ecosystem {
        signatures::Ecosystem::Rust => rust_index(workspace.root(), disk, &tracked),
        signatures::Ecosystem::TypeScript => typescript_index(workspace, &tracked, log).await,
        signatures::Ecosystem::Unknown => {
            log.say("no known ecosystem in this checkout — no signature index");
            return signatures::Index::default();
        }
    };
    log.say(&format!(
        "signature index: {} file(s) shaped ({ecosystem:?}); each prompt carries \
         only the ones its task names",
        by_path.len()
    ));
    signatures::Index {
        ecosystem,
        by_path,
        tracked,
    }
}

/// Rust: the shape is read out of the source, with no subprocess at all.
fn rust_index(
    root: &Path,
    disk: &dyn Disk,
    tracked: &[String],
) -> std::collections::BTreeMap<String, String> {
    tracked
        .iter()
        .filter(|path| Path::new(path).extension().is_some_and(|ext| ext == "rs"))
        .filter_map(|path| {
            let text = disk.read_to_string(&root.join(path))?;
            let shape = signatures::rust_shape(&text);
            (!shape.trim().is_empty()).then(|| (path.clone(), shape))
        })
        .collect()
}

/// TypeScript: `tsc` emits the declarations, into a folder nothing else reads.
///
/// `--noEmit false` is not optional. The target checkout sets `noEmit: true` in
/// its `tsconfig.json`, and with it `--emitDeclarationOnly` writes **nothing**
/// and still exits 0 — a silent success that would have shipped an empty index
/// with no sign anything went wrong.
async fn typescript_index(
    workspace: &Workspace,
    tracked: &[String],
    log: &Logbook,
) -> std::collections::BTreeMap<String, String> {
    let out = workspace.log_dir("signatures");
    let _ = std::fs::remove_dir_all(&out);
    let args: Vec<String> = [
        "tsc",
        "--emitDeclarationOnly",
        "--declaration",
        "--noEmit",
        "false",
        "--outDir",
        &out.to_string_lossy(),
    ]
    .iter()
    .map(|arg| (*arg).to_string())
    .collect();
    if let Err(why) = process::run_for("npx", &args, workspace.root(), INDEX_TIMEOUT).await {
        log.warn(&format!(
            "tsc could not emit the signature index ({why}) — sessions will read \
             the files themselves, as before"
        ));
        return std::collections::BTreeMap::new();
    }
    let mut found: Vec<(String, String)> = Vec::new();
    declarations_in(&out, &out, &mut found);
    if found.is_empty() {
        log.warn(
            "tsc emitted no declaration — check that the project compiles; \
             sessions will read the files themselves",
        );
    }
    // `tsc` writes `core/place.d.ts` for `src/core/place.ts`: it strips the
    // inferred `rootDir`. Keying the index on the emitted path would mean no
    // issue body ever matches it, so each one is resolved back to the tracked
    // file it describes. Unresolved entries are dropped rather than guessed —
    // a key nothing can name is a key nothing will read.
    found
        .into_iter()
        .filter_map(|(emitted, text)| {
            let real = tracked
                .iter()
                .find(|path| path.as_str() == emitted || path.ends_with(&format!("/{emitted}")))?;
            Some((real.clone(), text))
        })
        .collect()
}

/// Now, in seconds since the epoch.
///
/// Zero when the clock is before the epoch, which no reading can then be stale
/// against — the safe direction: a reading judged fresh is believed, and a
/// believed reading is what stops a run, never what starts one wrongly.
fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The workflow, wired against the mounted workspace, then executed.
async fn turns(
    args: &RunArgs,
    workspace: &Workspace,
    run_id: &str,
    disk: Rc<dyn Disk>,
    log: &Logbook,
    run_log: &Path,
) -> harness_core::domain::Outcome<Verdict> {
    // Everything that follows talks to the **clone**, not the repository from
    // which the run is launched: that is where sessions edit, and that is where
    // `gh` must respond.
    let gh: Rc<dyn GitHub> = Rc::new(GhCli::new(workspace.root()));
    let sessions: Rc<dyn SessionFactory> = if args.dry_run {
        // A dry-run is a wiring choice, not a framework branch.
        Rc::new(Rehearsal::new(log.clone()))
    } else {
        Rc::new(ClaudeCliFactory::new(
            workspace.root(),
            &args.permission_mode,
            Some(crate::dispatch::shared::trace_dir(run_log)),
        ))
    };
    let spending: Rc<dyn Spending> = Rc::new(
        LedgerSpending::new(&workspace.ledger(), run_id, &spending::hostname().await)
            // Where the next run's preflight reads what this one saw of the
            // rate-limit windows. On the store, not on the agent adapter: the
            // carrier translates, the store keeps.
            .keeping_quota_at(workspace.quota()),
    );
    let store: Rc<dyn Checkpoints> = Rc::new(Checkpoint::new(&workspace.loop_dir()));
    // Read before the first turn: the pointer says which task an interrupted
    // run was doing, and it **takes precedence** over the table's choice.
    let resuming = if args.no_resume {
        None
    } else {
        store.pointer()?.task
    };
    if let Some(task) = &resuming {
        log.say(&format!("resuming: task #{task} (use --no-resume to skip)"));
    }

    let ports = Ports {
        gh: Rc::clone(&gh),
        sessions,
        spending,
        disk: Rc::clone(&disk),
    };
    let signatures = Rc::new(if args.no_signatures {
        // Said out loud, like every other gate that stops applying: a run whose
        // prompts silently lost a block would be the one measurement nobody
        // could interpret afterwards.
        log.say("--no-signatures — no signature index, and no tsc run either");
        signatures::Index::default()
    } else {
        signature_index(workspace, disk.as_ref(), log).await
    });
    let config = Config {
        integration_branch: args.branch.clone(),
        model: args.model.clone(),
        effort: args.effort.clone(),
        restart: args.restart,
        stack: configuration_digest(workspace, disk.as_ref(), log).await,
        signatures,
    };
    announce(args, &ports, &config, run_id, log);

    let pre = run::workspace_gates(
        &ports,
        &config,
        workspace.root().to_path_buf(),
        args.dry_run,
        Rc::clone(&disk),
        Rc::clone(&gh),
        run::QuotaRoom {
            at: workspace.quota(),
            ignored: args.ignore_quota,
            now: now_seconds(),
        },
    );
    let built = run::build(
        &ports,
        &config,
        run::Request {
            rounds_budget: args.rounds,
            stages_filter: args.stages.clone(),
            resuming,
        },
        pre,
        Some(store),
        run_id.to_string(),
    );
    let mut ctx = Context::new(settings(args), Loop::default(), log.clone());
    built.execute(&mut ctx).await
}

/// Says what this run will do, once all gates are passed.
fn announce(args: &RunArgs, ports: &Ports, config: &Config, run_id: &str, log: &Logbook) {
    let table = stages::table(ports, config, 1);
    let cfg = settings(args);
    log.say(&format!(
        "run {run_id} — branch {}, {} round(s){}",
        args.branch,
        args.rounds,
        if args.dry_run { " [dry-run]" } else { "" }
    ));
    log.say(&format!("pipeline: {}", stages::summary(&table, &cfg)));
    // A run with `--stages code` said nothing about the stages it left out,
    // which reads as a pipeline that lost them.
    let left_out = stages::filtered_out(&table, &cfg);
    if !left_out.is_empty() {
        log.say(&format!("left out by --stages: {}", left_out.join(" ")));
    }
    log.say(&format!(
        "permission mode: {} (PreToolUse hooks of the target repository \
         always apply)",
        args.permission_mode
    ));
}

#[cfg(test)]
mod tests {
    //! What is tested here is what remains **specific to the launcher**: `Cli`
    //! parsing, tooling gates, console level. The wiring of the workflow itself
    //! — which round gets what — is tested against
    //! `harness_workflows::dev_loop::run`, with the fakes of that crate.

    use super::*;
    use harness_core::domain::{Halt, Outcome};
    use harness_core::ports::shell::process;
    use serial_test::serial;

    struct NoGitHub;

    #[async_trait::async_trait(?Send)]
    impl GitHub for NoGitHub {
        async fn authenticated(&self) -> Outcome<bool> {
            Err(refused())
        }
        async fn repo(&self) -> Outcome<String> {
            Err(refused())
        }
        async fn labels(&self) -> Outcome<Vec<String>> {
            Err(refused())
        }
        async fn issue(&self, _number: u64) -> Outcome<harness_core::domain::Issue> {
            Err(refused())
        }
        async fn issues_labelled(
            &self,
            _label: &str,
            _state: &str,
        ) -> Outcome<Vec<harness_core::domain::Issue>> {
            Err(refused())
        }
        async fn sub_issues(&self, _number: u64) -> Outcome<Vec<harness_core::domain::Issue>> {
            Err(refused())
        }
        async fn blocked_by(&self, _number: u64) -> Outcome<Vec<harness_core::domain::Issue>> {
            Err(refused())
        }
        async fn with_blockers(
            &self,
            _tasks: Vec<harness_core::domain::Issue>,
        ) -> Outcome<Vec<harness_core::domain::Issue>> {
            Err(refused())
        }
        async fn merged_prs(&self, _base: &str) -> Outcome<Vec<harness_core::domain::Issue>> {
            Err(refused())
        }
        async fn issue_comments(&self, _number: u64) -> Outcome<Vec<String>> {
            Err(refused())
        }
        async fn add_label(&self, _number: u64, _label: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn remove_label(&self, _number: u64, _label: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn set_body(&self, _number: u64, _body: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn post_issue_comment(&self, _number: u64, _body: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn close_issue(&self, _number: u64) -> Outcome<()> {
            Err(refused())
        }
        async fn pr(&self, _pr_ref: &str) -> Outcome<harness_core::domain::Pr> {
            Err(refused())
        }
        async fn pr_comments(&self, _num: &str) -> Outcome<String> {
            Err(refused())
        }
        async fn post_pr_comment(&self, _num: &str, _body_file: &std::path::Path) -> Outcome<()> {
            Err(refused())
        }
        async fn create_label(&self, _name: &str, _color: &str, _description: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn branch_sha(&self, _branch: &str) -> Outcome<Option<String>> {
            Err(refused())
        }
        async fn create_branch(&self, _branch: &str, _sha: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn default_branch(&self) -> Outcome<String> {
            Err(refused())
        }
        async fn set_default_branch(&self, _branch: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn protect_branch(&self, _branch: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn can_push(&self) -> Outcome<bool> {
            Err(refused())
        }
        async fn file_text(&self, _path: &str, _git_ref: &str) -> Outcome<Option<String>> {
            Err(refused())
        }
        async fn create_issue(&self, _title: &str, _body: &str, _labels: &[&str]) -> Outcome<u64> {
            Err(refused())
        }
        async fn create_sub_issue_link(&self, _parent: u64, _child: u64) -> Outcome<()> {
            Err(refused())
        }
        async fn add_blocked_by(&self, _number: u64, _blocker: u64) -> Outcome<()> {
            Err(refused())
        }
        async fn create_pr(
            &self,
            _head: &str,
            _base: &str,
            _title: &str,
            _body: &str,
        ) -> Outcome<String> {
            Err(refused())
        }
        async fn merge_pr(&self, _pr_ref: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn pr_checks_green(&self, _pr_ref: &str) -> Outcome<bool> {
            Err(refused())
        }
        async fn open_prs_labelled(&self, _label: &str) -> Outcome<Vec<harness_core::domain::Pr>> {
            Err(refused())
        }
        async fn pr_failing_checks(&self, _pr_ref: &str) -> Outcome<Vec<String>> {
            Err(refused())
        }
    }

    fn refused() -> Halt {
        Halt::Failed("no call should leave this test".to_string())
    }

    fn run_args(args: &[&str]) -> RunArgs {
        let mut full = vec!["harness"];
        full.extend_from_slice(args);
        <crate::cli::Cli as clap::Parser>::try_parse_from(full)
            .expect("valid arguments")
            .run
    }

    #[test]
    #[serial]
    fn the_console_level_follows_the_flags_and_the_file_keeps_everything() {
        assert_eq!(verbosity(&run_args(&[])), Verbosity::Normal);
        assert_eq!(verbosity(&run_args(&["--verbose"])), Verbosity::Verbose);
        assert_eq!(verbosity(&run_args(&["--quiet"])), Verbosity::Quiet);
    }

    #[test]
    #[serial]
    fn a_clone_does_not_demand_a_clean_tree_in_the_humans_checkout() {
        // The run does not touch its tree: requiring it clean would force them
        // to commit for a run that works elsewhere.
        let here = Workspace::new(Path::new("/depot"));
        // We cannot read inside a gate; what we freeze is the setting that
        // makes it permissive, and it depends on `--no-workspace`.
        let _ = tooling_pre(&run_args(&[]), &here, Rc::new(NoGitHub));
        assert!(!run_args(&[]).no_workspace);
        assert!(run_args(&["--no-workspace"]).no_workspace);
    }

    #[test]
    #[serial]
    fn the_pre_mount_gate_no_longer_names_the_two_target_facing_checks() {
        // D4: structural proxy — a `Box<dyn Verification<S>>` isn't otherwise
        // nameable at runtime, so the check count is what a test can assert.
        let here = Workspace::new(Path::new("/depot"));
        let gate = tooling_pre(&run_args(&[]), &here, Rc::new(NoGitHub));
        assert_eq!(gate.checks.len(), 3);
    }

    #[test]
    #[serial]
    fn the_post_mount_gate_holds_exactly_the_two_target_facing_checks() {
        let workspace = Workspace::new(Path::new("/clone"));
        let disk: Rc<dyn Disk> = Rc::new(RealDisk);
        let gate = tooling_post(&run_args(&[]), &workspace, &disk);
        assert_eq!(gate.checks.len(), 2);
    }

    /// A `git` that answers a fixed `origin`, for D4's `--no-workspace` guard
    /// — every other call is unreachable in this test.
    struct FixedOrigin(&'static str);

    #[async_trait::async_trait(?Send)]
    impl Repo for FixedOrigin {
        async fn current_branch(&self) -> Outcome<String> {
            unreachable!()
        }
        async fn head_sha(&self) -> Outcome<String> {
            unreachable!()
        }
        async fn dirty_files(&self) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn has_branch(&self, _name: &str) -> Outcome<bool> {
            unreachable!()
        }
        async fn origin_has_branch(&self, _name: &str) -> Outcome<bool> {
            unreachable!()
        }
        async fn remote_url(&self, _remote: &str) -> Outcome<String> {
            Ok(self.0.to_string())
        }
        async fn tracked_files(&self) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn default_branch(&self) -> Outcome<String> {
            unreachable!()
        }
        async fn local_branches(&self) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn stashes(&self) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn unpushed(&self) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn branches_at_risk(&self, _upstream: &str) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn clone_repo(&self, _url: &str, _name: &str) -> Outcome<process::Ran> {
            unreachable!()
        }
        async fn fetch(&self) -> Outcome<process::Ran> {
            unreachable!()
        }
        async fn checkout(&self, _branch: &str, _force: bool) -> Outcome<process::Ran> {
            unreachable!()
        }
        async fn reset_hard(&self, _reference: &str) -> Outcome<process::Ran> {
            unreachable!()
        }
        async fn clean(&self) -> Outcome<process::Ran> {
            unreachable!()
        }
        async fn delete_branch(&self, _name: &str) -> Outcome<process::Ran> {
            unreachable!()
        }
        async fn create_local_branch(&self, _name: &str, _from: &str) -> Outcome<process::Ran> {
            unreachable!()
        }
        async fn stage_all(&self) -> Outcome<process::Ran> {
            unreachable!()
        }
        async fn commit(&self, _message: &str) -> Outcome<process::Ran> {
            unreachable!()
        }
        async fn push(&self, _branch: &str) -> Outcome<process::Ran> {
            unreachable!()
        }
    }

    struct FixedOriginRepos(&'static str);

    impl Repos for FixedOriginRepos {
        fn at(&self, _root: &Path) -> Rc<dyn Repo> {
            Rc::new(FixedOrigin(self.0))
        }
    }

    #[tokio::test]
    #[serial]
    async fn no_workspace_with_a_mismatched_target_repo_url_halts() {
        let args = run_args(&["--no-workspace"]);
        let mut args = args;
        args.target_repo_url = "https://github.com/someone/else".to_string();
        let source = Workspace::new(Path::new("/depot"));
        let repos: Rc<dyn Repos> = Rc::new(FixedOriginRepos(
            "git@github.com:Laucans/event_assistant.git",
        ));
        let disk: Rc<dyn Disk> = Rc::new(RealDisk);
        let log = Logbook::null();

        let err = mount(&args, &source, &repos, &disk, "run-1", &log)
            .await
            .expect_err("should halt");
        assert!(matches!(err, Halt::Halted(_)));
    }

    #[tokio::test]
    #[serial]
    async fn no_workspace_with_a_matching_target_repo_url_does_not_halt() {
        let args = run_args(&["--no-workspace"]);
        let mut args = args;
        args.target_repo_url = "https://github.com/Laucans/event_assistant".to_string();
        let source = Workspace::new(Path::new("/depot"));
        let repos: Rc<dyn Repos> = Rc::new(FixedOriginRepos(
            "git@github.com:Laucans/event_assistant.git",
        ));
        let disk: Rc<dyn Disk> = Rc::new(RealDisk);
        let log = Logbook::null();

        mount(&args, &source, &repos, &disk, "run-1", &log)
            .await
            .expect("same repo, different spelling — must not halt");
    }
}
