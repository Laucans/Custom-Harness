//! The wiring of `harness init-repo`: the only place that builds
//! `GhCli::for_slug`.

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;

use harness_core::adapters::shell::disk::RealDisk;
use harness_core::adapters::shell::github::GhCli;
use harness_core::domain::Slug;
use harness_workflows::init_repo::config::Config;
use harness_workflows::init_repo::data::report::Verdict;
use harness_workflows::init_repo::ports::Ports;
use harness_workflows::init_repo::run;

use crate::cli::InitRepoArgs;

/// Runs one `init-repo` invocation and returns the exit code the scheduler
/// reads.
pub async fn run(args: &InitRepoArgs, here: &Path) -> ExitCode {
    let Some(slug) = Slug::parse(&args.url) else {
        eprintln!("not a usable github.com URL: {}", args.url);
        return ExitCode::from(1);
    };
    let branch = args
        .branch
        .clone()
        .or_else(|| std::env::var("INTEGRATION_BRANCH").ok())
        .filter(|branch| !branch.is_empty())
        .unwrap_or_else(|| "main_agent".to_string());
    let env_file = args
        .env_file
        .clone()
        .unwrap_or_else(|| here.join(".env.local"));

    let ports = Ports {
        gh: Rc::new(GhCli::for_slug(&slug)),
        disk: Rc::new(RealDisk),
    };
    let config = Config {
        slug,
        branch,
        env_file,
        write_env: !args.no_env,
        force: args.force,
        dry_run: args.dry_run,
    };
    let built = run::build(&ports, &config, run::Request);

    match built.execute().await {
        Ok((report, Verdict::Ready)) => {
            println!("{}", report.render());
            ExitCode::SUCCESS
        }
        Ok((report, Verdict::StillBlocking)) => {
            println!("{}", report.render());
            ExitCode::from(1)
        }
        Err(halt) => {
            eprintln!("{}: {}", halt.prefix(), halt.reason());
            ExitCode::from(u8::try_from(halt.exit_code()).unwrap_or(2))
        }
    }
}
