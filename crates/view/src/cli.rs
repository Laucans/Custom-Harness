//! What a human types to open the plant.
//!
//! The variables shared with the launcher (`TARGET_REPO_URL`,
//! `INTEGRATION_BRANCH`, `PERMISSION_MODE`) are declared with the same names
//! and defaults, so the `.env.local` a `harness watch` reads points the view
//! at the same repository without a second configuration.

use std::path::PathBuf;

use clap::Parser;

/// `harness-view`: a local web page that draws the harness as a plant.
#[derive(Debug, Parser)]
#[command(name = "harness-view", version, about)]
pub struct Cli {
    /// Port the page listens on.
    #[arg(long, env = "HARNESS_VIEW_PORT", default_value_t = 7878)]
    pub port: u16,

    /// Address to bind. Local by default: the page shows session logs and
    /// issue bodies, and the steward's terminal is a shell in this checkout.
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,

    /// Seconds between two reads of the traces.
    #[arg(long, default_value_t = 2)]
    pub interval: u64,

    /// Seconds between two reads of the GitHub board.
    #[arg(long, default_value_t = 60)]
    pub board_interval: u64,

    /// The repository the plant stands for: `owner/name` or a GitHub URL.
    /// Empty: this checkout's own `origin`, as `harness watch` does.
    #[arg(long, env = "TARGET_REPO_URL", default_value = "")]
    pub target_repo_url: String,

    /// The branch the agents integrate into — what the store calls the
    /// agents' version, and what the steward starts the watch on.
    #[arg(long, env = "INTEGRATION_BRANCH", default_value = "main_agent")]
    pub integration_branch: String,

    /// Never call `gh`: the sign and the store stay empty.
    #[arg(long)]
    pub no_board: bool,

    /// No steward: the plant has nobody to talk to.
    #[arg(long)]
    pub no_steward: bool,

    /// The program the steward's terminal runs, in this checkout. Left at
    /// `claude`, the view picks the newest Claude Code it can find on the
    /// machine rather than whatever `PATH` resolves to.
    #[arg(long, default_value = "claude")]
    pub steward_command: String,

    /// The model the steward's `claude` opens with (`--model`).
    #[arg(long, env = "STEWARD_MODEL", default_value = "claude-sonnet-5-5")]
    pub steward_model: String,

    /// Passed to the steward's `claude` as `--permission-mode`, like the
    /// launcher passes it to every session.
    #[arg(long, env = "PERMISSION_MODE", default_value = "bypassPermissions")]
    pub permission_mode: String,

    /// Serve the front-end from this directory instead of the embedded copy,
    /// so a drawing can be redone without a rebuild.
    #[arg(long)]
    pub static_dir: Option<PathBuf>,

    /// Show the most recent run as live even though it is over — to see the
    /// plant move when nothing runs.
    #[arg(long)]
    pub demo: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn the_defaults_are_a_local_page_on_a_quiet_port() {
        let cli = Cli::try_parse_from(["harness-view"]).expect("parses");
        assert_eq!(cli.port, 7878);
        assert_eq!(cli.bind, "127.0.0.1");
        assert_eq!(cli.interval, 2);
        assert_eq!(cli.steward_command, "claude");
        assert_eq!(cli.steward_model, "claude-sonnet-5-5");
        assert_eq!(cli.permission_mode, "bypassPermissions");
        assert!(!cli.no_board);
        assert!(!cli.no_steward);
        assert!(!cli.demo);
    }

    #[test]
    fn every_variable_the_view_reads_appears_in_the_help() {
        let help = Cli::command().render_long_help().to_string();
        for variable in [
            "HARNESS_VIEW_PORT",
            "TARGET_REPO_URL",
            "INTEGRATION_BRANCH",
            "PERMISSION_MODE",
            "STEWARD_MODEL",
        ] {
            assert!(help.contains(variable), "{variable} missing from --help");
        }
    }
}
