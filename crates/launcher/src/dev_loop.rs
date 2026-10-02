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

use harness_core::adapters::agent::SessionFactory;
use harness_core::adapters::agent::claude_cli::ClaudeCliFactory;
use harness_core::adapters::agent::rehearsal::Rehearsal;
use harness_core::adapters::shell::disk::{Disk, RealDisk};
use harness_core::adapters::shell::git::{GitCli, GitRepos, Repo, Repos};
use harness_core::adapters::shell::github::{GhCli, GitHub};
use harness_core::adapters::store::checkpoint::Checkpoint;
use harness_core::adapters::store::spending::Spending;
use harness_core::domain::Verdict;
use harness_core::domain::workspace::{Wanted, Workspace};
use harness_core::execution::provisioning::{Mount, Provisioner, Run};
use harness_core::execution::{Context, Gate, Guarded, Settings, Verification};
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::dev_loop::config::Config;
use harness_workflows::dev_loop::data::state::Loop;
use harness_workflows::dev_loop::orchestration::stages;
use harness_workflows::dev_loop::ports::Ports;
use harness_workflows::dev_loop::run;

use crate::cli::RunArgs;
use crate::sink::Both;
use crate::spending::{self, LedgerSpending};
use crate::tooling;

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

    let outcome = turns(args, workspace, &run_id, Rc::clone(&disk), &log).await;
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

/// The workflow, wired against the mounted workspace, then executed.
async fn turns(
    args: &RunArgs,
    workspace: &Workspace,
    run_id: &str,
    disk: Rc<dyn Disk>,
    log: &Logbook,
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
        ))
    };
    let spending: Rc<dyn Spending> = Rc::new(LedgerSpending::new(
        &workspace.ledger(),
        run_id,
        &spending::hostname().await,
    ));
    let store = Rc::new(Checkpoint::new(&workspace.loop_dir()));
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

    // `/planner`'s grounding digests, if `*-grill-with-issues`/
    // `*-grill-with-docs` ever wrote any for this repo — keyed by `owner/name`
    // under the harness's own `state_root`, never the clone.
    let repo = gh.repo().await?;
    let grill_dir = workspace.state_root().join(".llocal/grill").join(&repo);

    let ports = Ports {
        gh: Rc::clone(&gh),
        sessions,
        spending,
        disk: Rc::clone(&disk),
    };
    let config = Config {
        integration_branch: args.branch.clone(),
        model: args.model.clone(),
        effort: args.effort.clone(),
        restart: args.restart,
        grill_dir,
    };
    announce(args, &ports, &config, run_id, log);

    let pre = run::workspace_gates(
        &ports,
        &config,
        workspace.root().to_path_buf(),
        args.dry_run,
        Rc::clone(&disk),
        Rc::clone(&gh),
    );
    let built = run::build(
        &ports,
        &config,
        run::Request {
            rounds_budget: args.rounds,
            stages_filter: args.stages.clone(),
            rollover: args.rollover,
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
    if args.rollover {
        log.say("rollover wired: a finished milestone will run /planner");
    }
}

#[cfg(test)]
mod tests {
    //! What is tested here is what remains **specific to the launcher**: `Cli`
    //! parsing, tooling gates, console level. The wiring of the workflow itself
    //! — which round gets what — is tested against
    //! `harness_workflows::dev_loop::run`, with the fakes of that crate.

    use super::*;
    use harness_core::adapters::shell::process;
    use harness_core::domain::{Halt, Outcome};
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
        async fn can_push(&self) -> Outcome<bool> {
            Err(refused())
        }
        async fn file_text(&self, _path: &str, _git_ref: &str) -> Outcome<Option<String>> {
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
