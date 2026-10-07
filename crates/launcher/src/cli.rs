//! The arguments of a run, and the environment variables that back them up.
//!
//! **Declared together.** The declarations and environment variables live in
//! the same place: the help is generated from the declarations, so it can no
//! longer be incomplete.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use harness_core::domain::workspace::Strategy;

/// The harness: runs an agentic workflow under verification gates, or
/// initializes a repository for one.
///
/// No subcommand: the dev loop, flattened straight into [`RunArgs`] — every
/// invocation that worked before this split (`harness --rounds 1 --stages
/// code`) keeps parsing exactly the same way.
#[derive(Debug, Parser)]
#[command(
    name = "harness",
    version,
    about = "Runs the agentic development loop.",
    long_about = None,
)]
pub struct Cli {
    /// The dev loop's own flags — flattened, so they stay at the top level
    /// when no subcommand is given.
    #[command(flatten)]
    pub run: RunArgs,

    /// What to run instead of the dev loop. Absent: the dev loop.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// A deterministic command, distinct from the dev loop.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Point the harness at a repository: labels, the integration branch, a
    /// read-only audit, and the link written to `.env.local`.
    InitRepo(InitRepoArgs),
    /// Poll on an interval, decide which workflow is ready to run next, and
    /// dispatch to it.
    Watch(WatchArgs),
    /// Read why the harness last stopped and repair what can be repaired.
    ///
    /// The same code `watch` runs on its next tick after a failure. By hand,
    /// it answers "why is the loop not moving" and unblocks it.
    Doctor(DoctorArgs),
}

/// `harness doctor`'s own arguments.
#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Say what would be repaired, change nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `harness watch`'s own arguments.
///
/// Booleans, for the same reason as [`RunArgs`]: a flag is present or absent,
/// and that is what a command line is.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args)]
pub struct WatchArgs {
    /// Seconds between two polls.
    #[arg(long, default_value_t = 30)]
    pub interval: u64,

    /// One pass, then exit — for a manual check or a test.
    #[arg(long)]
    pub once: bool,

    /// Read everything, write nothing — propagated to whichever workflow a
    /// tick dispatches to.
    #[arg(long)]
    pub dry_run: bool,

    /// The target repository. Empty: this checkout's own `origin`.
    #[arg(long, env = "TARGET_REPO_URL", default_value = "")]
    pub target_repo_url: String,

    /// The branch `dev_loop` works on, and the one `planner`/`split`/
    /// `refinement` mount their shared read-only checkout at.
    #[arg(long, env = "INTEGRATION_BRANCH", default_value = "main_agent")]
    pub branch: String,

    /// Passed to `claude -p`.
    #[arg(long, env = "PERMISSION_MODE", default_value = "bypassPermissions")]
    pub permission_mode: String,

    /// Do not repair after a failed tick.
    ///
    /// The repair is on by default: the failure it treats — a quota that ran
    /// out mid-session — is ordinary, and left alone it deadlocks every later
    /// tick. This flag is for watching that deadlock happen on purpose.
    #[arg(long)]
    pub no_doctor: bool,

    /// Let a dispatched `dev_loop` overwrite local work left in the workspace.
    ///
    /// Without it, a session cut mid-work — a quota that runs out is the
    /// ordinary case — leaves uncommitted files behind, and **every later
    /// tick refuses on them**: the gate names `--force-reset` as the gesture,
    /// and an unattended watch has no way to make it. The work discarded is
    /// the harness's own unfinished attempt, in a clone it owns, and the next
    /// session redoes it from the issue.
    #[arg(long)]
    pub force_reset: bool,
}

/// `harness init-repo <url>`'s own arguments.
#[derive(Debug, Args)]
pub struct InitRepoArgs {
    /// The target repository: `https://github.com/o/r[.git]`,
    /// `git@github.com:o/r.git`, `ssh://…`, or the `o/r` shorthand.
    pub url: String,

    /// The integration branch to create and to audit. Empty:
    /// `INTEGRATION_BRANCH`, else `main_agent`.
    #[arg(long)]
    pub branch: Option<String>,

    /// Read everything, write nothing — not GitHub, not the env file.
    #[arg(long)]
    pub dry_run: bool,

    /// Where the link is written. Empty: `.env.local` at the harness root.
    #[arg(long)]
    pub env_file: Option<PathBuf>,

    /// Do every GitHub step, write no env file.
    #[arg(long)]
    pub no_env: bool,

    /// Overwrite an existing `TARGET_REPO_URL` that names another repo.
    #[arg(long)]
    pub force: bool,
}

/// The dev loop's flags.
///
/// Fifteen booleans, and that is what a command line **is**: a flag present
/// or absent. `clap` generates help from these fields.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Do not execute and do not spend: say what would have run, and write
    /// the prompts that would have been sent.
    #[arg(long)]
    pub dry_run: bool,

    /// The only stages to run, separated by spaces. Empty: all.
    #[arg(long, env = "STAGES", default_value = "")]
    pub stages: String,

    /// Force model for the entire run. Empty: each stage keeps its own.
    #[arg(long, env = "MODEL", default_value = "")]
    pub model: String,

    /// Force effort for the entire run.
    #[arg(long, env = "EFFORT", default_value = "")]
    pub effort: String,

    /// Maximum number of rounds.
    #[arg(long, env = "MAX_ROUNDS", default_value_t = 3)]
    pub rounds: u32,

    /// The branch on which the loop works and merges.
    #[arg(long, env = "INTEGRATION_BRANCH", default_value = "main_agent")]
    pub branch: String,

    /// The permission mode passed to `claude`.
    ///
    /// The `PreToolUse` hooks of the target repository always apply: this
    /// setting does not bypass them.
    #[arg(long, env = "PERMISSION_MODE", default_value = "bypassPermissions")]
    pub permission_mode: String,

    /// Allow run even if the working tree is dirty.
    #[arg(long, env = "ALLOW_DIRTY", num_args = 0..=1, default_missing_value = "true", default_value = "false", value_parser = truthy)]
    pub allow_dirty: bool,

    /// Replay a stage that resume would skip.
    #[arg(long)]
    pub restart: bool,

    /// Build no signature index, so prompts carry no `PUBLIC SIGNATURES` block.
    ///
    /// The control arm of the experiment: run the same task twice, once with and
    /// once without, and compare how the session reads the files its plan names.
    /// Everything else — the configuration digest, the scope, the model — stays
    /// identical.
    ///
    /// It has been run once, on #65's `code` stage, and the index did **not**
    /// reduce total file reading — see
    /// [`signatures`](harness_workflows::dev_loop::data::signatures) for the
    /// numbers and for the three reasons one pair of runs settles little. Worth
    /// running again on the next task large enough to matter.
    ///
    /// Also the escape hatch if `tsc` ever misbehaves on a checkout: the index is
    /// an optimisation, and a run must never depend on one.
    #[arg(long, env = "NO_SIGNATURES", num_args = 0..=1, default_missing_value = "true", default_value = "false", value_parser = truthy)]
    pub no_signatures: bool,

    /// Start a dev phase even when the rate-limit window is nearly spent.
    ///
    /// The reserve exists because an interrupted stage is expensive: on #64 a
    /// window that ran out 60 turns into `code` cost 4,32 $ that delivered
    /// nothing. Refused sessions themselves are free. Set this when you would
    /// rather risk the interruption than wait.
    #[arg(long, env = "IGNORE_QUOTA", num_args = 0..=1, default_missing_value = "true", default_value = "false", value_parser = truthy)]
    pub ignore_quota: bool,

    /// Resume the task that the checkpoint designates. Default: yes.
    #[arg(long)]
    pub no_resume: bool,

    /// Everything, including what a session says in detail.
    #[arg(long, short, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Only stops and failures.
    #[arg(long, short)]
    pub quiet: bool,

    // --- workspace ---------------------------------------------------------
    /// Work in the repository from which the run is launched, without cloning.
    #[arg(long)]
    pub no_workspace: bool,

    /// What happens to the workspace directory. Empty: the domain default
    /// ([`Strategy::Permanent`]), read by [`RunArgs::strategy`].
    ///
    /// A `String`, not an `Option<Strategy>` with a rejecting `value_parser` —
    /// `clap` calls the parser as soon as the variable exists, even if empty,
    /// and `Strategy::parse("")` rightly rejects an unknown spelling.
    #[arg(long, env = "WORKSPACE_STRATEGY", default_value = "")]
    pub workspace_strategy: String,

    /// The URL to clone. Empty: `TARGET_REPO_URL`, else the `origin` of the
    /// repository from which the run is launched.
    #[arg(long, env = "WORKSPACE_URL", default_value = "")]
    pub workspace_url: String,

    /// The repository `init-repo` initialized and the loop mounts.
    ///
    /// Precedence: `--workspace-url` (above) wins when given;
    /// `TARGET_REPO_URL` otherwise; and when both are empty, this checkout's
    /// own `origin` — today's behavior, and the fallback is what keeps every
    /// invocation from before this setting existed working unchanged.
    #[arg(long, env = "TARGET_REPO_URL", default_value = "")]
    pub target_repo_url: String,

    /// Find this workspace by name. It is then **never deleted**.
    #[arg(long, default_value = "")]
    pub use_workspace: String,

    /// The directory that contains workspaces.
    #[arg(long, env = "AGENTIC_WORKSPACES_DIR", default_value = "")]
    pub workspaces_dir: String,

    /// Keep a disposable workspace that the run would delete.
    #[arg(long, env = "KEEP_WORKSPACE", num_args = 0..=1, default_missing_value = "true", default_value = "false", value_parser = truthy)]
    pub keep_workspace: bool,

    /// Overwrite local work of a reused workspace.
    ///
    /// Without this flag, a workspace with work stops the run by naming it.
    /// A human takes this output.
    #[arg(long)]
    pub force_reset: bool,
}

impl RunArgs {
    /// The workspace strategy requested, or the domain default if nothing is
    /// given.
    ///
    /// # Errors
    ///
    /// An error naming the mistake if `workspace_strategy` is neither empty nor
    /// a known strategy — refused rather than read as the default, for the same
    /// reason as [`Strategy::parse`]: `WORKSPACE_STRATEGY=permanant` that fell
    /// back to `tmp` would delete, once and without saying anything, the
    /// workspace the human thought they were keeping.
    pub fn strategy(&self) -> Result<Strategy, String> {
        if self.workspace_strategy.is_empty() {
            return Ok(Strategy::Permanent);
        }
        strategy(&self.workspace_strategy)
    }
}

/// An environment boolean, with usual spellings.
///
/// `clap` requires the literal string `true`/`false` for any `bool` field
/// combined with `env`. The flags that also write to environment variables
/// (`ALLOW_DIRTY`, `KEEP_WORKSPACE`) pass through this parser rather than
/// `clap`'s bare `bool` type: `num_args = 0..=1` and `default_missing_value =
/// "true"` keep the bare flag valid alone on the command line, and this
/// parser additionally accepts what an environment variable writes in
/// practice.
///
/// Empty is false **intentionally**: that is what an unfilled line in
/// `.env.local` writes, and it is the most common case — not a typo. A real
/// typo (`ALLOW_DIRTY=flase`) stays refused, for the same reason [`strategy`]
/// refuses rather than falling back to a default: a misspelled variable should
/// not read as silence.
fn truthy(text: &str) -> Result<bool, String> {
    match text.trim().to_lowercase().as_str() {
        "" | "0" | "false" | "no" => Ok(false),
        "1" | "true" | "yes" => Ok(true),
        other => Err(format!(
            "{other:?} is not a boolean value — known: 1/true/yes, \
             0/false/no, or empty"
        )),
    }
}

/// `Strategy::parse`, refusing rather than falling back to a default.
///
/// A `WORKSPACE_STRATEGY=permanant` that fell back to `tmp` would delete, once
/// per run and without saying anything, the workspace the human thought they
/// were keeping.
fn strategy(text: &str) -> Result<Strategy, String> {
    Strategy::parse(text)
        .ok_or_else(|| format!("{text:?} is not a strategy — known: {}", Strategy::known()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use serial_test::serial;

    #[test]
    fn the_declarations_are_coherent() {
        // clap's `debug_assert`: duplicate names, impossible conflicts, default
        // value that does not pass its own `value_parser`.
        Cli::command().debug_assert();
    }

    #[test]
    fn every_environment_variable_the_run_reads_shows_up_in_the_help() {
        // The help is generated from the `env = …` declarations.
        let help = Cli::command().render_long_help().to_string();
        for variable in [
            "STAGES",
            "MODEL",
            "EFFORT",
            "MAX_ROUNDS",
            "INTEGRATION_BRANCH",
            "PERMISSION_MODE",
            "ALLOW_DIRTY",
            "WORKSPACE_STRATEGY",
            "WORKSPACE_URL",
            "TARGET_REPO_URL",
            "AGENTIC_WORKSPACES_DIR",
            "KEEP_WORKSPACE",
        ] {
            assert!(help.contains(variable), "{variable} missing from --help");
        }
    }

    #[test]
    fn a_misspelled_strategy_is_refused_rather_than_defaulted() {
        assert!(strategy("permanant").is_err());
        assert_eq!(strategy("tmp"), Ok(Strategy::Tmp));
    }

    #[test]
    #[serial]
    fn the_defaults_are_what_the_loop_expects() {
        let cli = Cli::try_parse_from(["harness"]).expect("the defaults");
        assert_eq!(cli.run.rounds, 3);
        assert_eq!(cli.run.branch, "main_agent");
        assert_eq!(cli.run.permission_mode, "bypassPermissions");
        assert!(!cli.run.dry_run);
        // And no strategy forced on the command line: it is `RunArgs::strategy`
        // that falls back to the domain default, which does not delete anything.
        assert_eq!(cli.run.workspace_strategy, "");
        assert_eq!(cli.run.strategy(), Ok(Strategy::Permanent));
        assert!(cli.command.is_none());
    }

    #[test]
    #[serial]
    fn an_empty_workspace_strategy_defaults_rather_than_erroring() {
        // Same issue as the three booleans: `clap` would call
        // `Strategy::parse("")` as soon as `WORKSPACE_STRATEGY=` exists, even
        // if empty, if the field stayed as an `Option<Strategy>` with
        // `value_parser`.
        let cli = Cli::try_parse_from_with_env(["harness"], &[("WORKSPACE_STRATEGY", "")])
            .expect("must not fail argument parsing");
        assert_eq!(cli.run.strategy(), Ok(Strategy::Permanent));
    }

    #[test]
    #[serial]
    fn a_misspelled_workspace_strategy_is_refused_at_use_not_defaulted() {
        let cli = Cli::try_parse_from_with_env(["harness"], &[("WORKSPACE_STRATEGY", "permanant")])
            .expect("argument parsing no longer rejects — RunArgs::strategy does");
        assert!(cli.run.strategy().is_err());
    }

    #[test]
    #[serial]
    fn verbose_and_quiet_cannot_be_asked_for_together() {
        assert!(Cli::try_parse_from(["harness", "--verbose", "--quiet"]).is_err());
    }

    #[test]
    #[serial]
    fn a_bool_flag_still_takes_no_value_on_the_command_line() {
        let cli = Cli::try_parse_from(["harness", "--allow-dirty"]).expect("a bare flag");
        assert!(cli.run.allow_dirty);
    }

    #[test]
    fn truthy_accepts_the_usual_spellings() {
        for yes in ["1", "true", "yes", "TRUE", " yes "] {
            assert_eq!(truthy(yes), Ok(true), "{yes:?}");
        }
        for no in ["0", "false", "no", "FALSE"] {
            assert_eq!(truthy(no), Ok(false), "{no:?}");
        }
    }

    #[test]
    fn truthy_reads_empty_as_false_because_that_is_what_an_unfilled_env_var_is() {
        // What `.env.local` writes for an unfilled variable: an empty
        // `ALLOW_DIRTY=` line. `clap` requires literal `true`/`false` for any
        // `bool` combined with `env` — neither `1`, nor empty, nor
        // `action = SetTrue` changes that, which would fail `--allow-dirty`
        // alone as soon as `.env.local` existed, even with the variable
        // empty. That is what `truthy` works around.
        assert_eq!(truthy(""), Ok(false));
    }

    #[test]
    fn truthy_refuses_a_typo_rather_than_reading_it_as_unset() {
        // Same logic as `strategy`: a misspelled variable should not read as
        // silence.
        assert!(truthy("flase").is_err());
    }

    #[test]
    #[serial]
    fn an_empty_or_truthy_environment_variable_does_not_break_the_flag() {
        for (value, expect_allow_dirty) in [("", false), ("1", true), ("true", true)] {
            let cli = Cli::try_parse_from_with_env(["harness"], &[("ALLOW_DIRTY", value)])
                .expect("must not fail");
            assert_eq!(
                cli.run.allow_dirty, expect_allow_dirty,
                "ALLOW_DIRTY={value:?}"
            );
        }
    }

    #[test]
    #[serial]
    fn harness_rounds_1_stages_code_still_parses_as_the_dev_loop() {
        // The regression this guards: the flatten + optional-subcommand split
        // must not break the invocation every current user already has.
        let cli = Cli::try_parse_from(["harness", "--rounds", "1", "--stages", "code"])
            .expect("must still parse");
        assert!(cli.command.is_none());
        assert_eq!(cli.run.rounds, 1);
        assert_eq!(cli.run.stages, "code");
    }

    #[test]
    #[serial]
    fn init_repo_parses_as_a_subcommand() {
        let cli = Cli::try_parse_from(["harness", "init-repo", "https://github.com/o/r"])
            .expect("must parse");
        let Some(Command::InitRepo(args)) = cli.command else {
            panic!("expected Command::InitRepo");
        };
        assert_eq!(args.url, "https://github.com/o/r");
        assert!(!args.dry_run);
    }

    #[test]
    #[serial]
    fn init_repo_accepts_its_own_flags() {
        let cli = Cli::try_parse_from([
            "harness",
            "init-repo",
            "o/r",
            "--branch",
            "main_agent",
            "--dry-run",
            "--force",
        ])
        .expect("must parse");
        let Some(Command::InitRepo(args)) = cli.command else {
            panic!("expected Command::InitRepo");
        };
        assert_eq!(args.branch.as_deref(), Some("main_agent"));
        assert!(args.dry_run);
        assert!(args.force);
    }

    /// `Cli::try_parse_from`, with environment variables set for the duration
    /// of the call.
    ///
    /// `std::env::set_var` touches a global process state, shared among the
    /// threads that `cargo test` uses for the other tests in this file — setting
    /// and removing them in the same call, rather than in a test that would
    /// continue after, is what prevents a leak to a neighboring test even in
    /// case of panic in between.
    trait TryParseWithEnv: Sized {
        fn try_parse_from_with_env<I, T>(
            args: I,
            vars: &[(&str, &str)],
        ) -> Result<Self, clap::Error>
        where
            I: IntoIterator<Item = T>,
            T: Into<std::ffi::OsString> + Clone;
    }

    impl TryParseWithEnv for Cli {
        #[allow(unsafe_code)]
        fn try_parse_from_with_env<I, T>(
            args: I,
            vars: &[(&str, &str)],
        ) -> Result<Self, clap::Error>
        where
            I: IntoIterator<Item = T>,
            T: Into<std::ffi::OsString> + Clone,
        {
            // SAFETY: Set then removed before returning control, in the same
            // call — no other test can observe the intermediate state.
            unsafe {
                for (var, value) in vars {
                    std::env::set_var(var, value);
                }
            }
            let parsed = Self::try_parse_from(args);
            unsafe {
                for (var, _) in vars {
                    std::env::remove_var(var);
                }
            }
            parsed
        }
    }

    /// Every variable that `.env.example` declares, as it is written there.
    ///
    /// A single source for the two tests that follow, so that the day a line is
    /// added to `.env.example` without being added here, the gap shows — not to
    /// re-read the file automatically: `.env.example` is a template for a human,
    /// this array is what the code promises to accept.
    const ENV_EXAMPLE: &[(&str, &str)] = &[
        ("STAGES", ""),
        ("MODEL", ""),
        ("EFFORT", ""),
        ("MAX_ROUNDS", "3"),
        ("INTEGRATION_BRANCH", "main_agent"),
        ("PERMISSION_MODE", "bypassPermissions"),
        ("ALLOW_DIRTY", ""),
        ("WORKSPACE_STRATEGY", ""),
        ("WORKSPACE_URL", ""),
        ("TARGET_REPO_URL", ""),
        ("AGENTIC_WORKSPACES_DIR", ""),
        ("KEEP_WORKSPACE", ""),
    ];

    #[test]
    #[serial]
    fn a_dot_env_local_freshly_copied_from_the_template_parses_without_error() {
        // The scenario that broke twice in real use before this test existed:
        // `.env.local` copied as-is from `.env.example`, most lines left empty.
        // The two bugs — `bool` + `env`, and `Option<Strategy>` + `env` — would
        // both have shown up here.
        let cli = Cli::try_parse_from_with_env(["harness"], ENV_EXAMPLE)
            .expect("a fresh .env.local must always parse");
        assert_eq!(cli.run.strategy(), Ok(Strategy::Permanent));
    }

    #[test]
    fn every_line_of_the_template_is_covered_by_the_regression_test() {
        // Guard against the opposite gap: a line added to `.env.example`
        // without being added to `ENV_EXAMPLE` above would no longer be covered,
        // silently.
        let documented = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env.example"),
        )
        .expect(".env.example must be readable");
        for line in documented.lines() {
            let Some((name, _)) = line.split_once('=') else {
                continue;
            };
            if name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                assert!(
                    ENV_EXAMPLE.iter().any(|(known, _)| *known == name),
                    "{name} is in .env.example but not in ENV_EXAMPLE"
                );
            }
        }
    }
}
