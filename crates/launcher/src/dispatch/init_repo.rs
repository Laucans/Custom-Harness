//! The wiring of `harness init-repo`: the only place that builds
//! `GhCli::for_slug`, and a `GitRepos` and `Processes` for the install
//! clone.

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;

use harness_adapters::shell::disk::RealDisk;
use harness_adapters::shell::git::GitRepos;
use harness_adapters::shell::github::GhCli;
use harness_adapters::shell::process::Processes;
use harness_core::domain::Slug;
use harness_workflows::init_repo::config::Config;
use harness_workflows::init_repo::data::report::Verdict;
use harness_workflows::init_repo::ports::Ports;
use harness_workflows::init_repo::run;

use harness_core::traces::Logbook;

use crate::cli::InitRepoArgs;

/// Brings what the harness installed in the target up to this harness, at
/// a watch's start: a file the product never touched is rewritten, one
/// commit on the integration branch; one it changed is kept and named.
///
/// Never fails the watch: a target it cannot read is said and skipped.
pub async fn upgrade(target_url: &str, branch: &str, here: &Path, log: &Logbook) {
    let Some(slug) = Slug::parse(target_url) else {
        log.say("upgrade: no target repository named — skipped");
        return;
    };
    let ports = Ports {
        gh: Rc::new(GhCli::for_slug(&slug)),
        disk: Rc::new(RealDisk),
        repos: Rc::new(GitRepos),
        process: Rc::new(Processes),
    };
    let config = Config {
        slug,
        branch: branch.to_string(),
        env_file: here.join(".env.local"),
        write_env: false,
        force: false,
        dry_run: false,
        workdir: here.join(".llocal").join("init"),
        install: true,
    };
    match harness_workflows::init_repo::action::install::run(&ports, &config).await {
        Ok(lines) => {
            for line in lines {
                log.say(&format!("upgrade: {line}"));
            }
        }
        Err(halt) => log.warn(&format!("upgrade: {}", halt.reason())),
    }
}

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
        repos: Rc::new(GitRepos),
        process: Rc::new(Processes),
    };
    let config = Config {
        slug,
        branch,
        env_file,
        write_env: !args.no_env,
        force: args.force,
        dry_run: args.dry_run,
        // Under the harness's own local state, never in the target.
        workdir: here.join(".llocal").join("init"),
        install: !args.no_install,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_url_that_names_no_repository_fails_before_any_adapter_is_built() {
        let args = InitRepoArgs {
            url: "not a repository".to_string(),
            branch: None,
            dry_run: true,
            env_file: None,
            no_env: true,
            force: false,
            no_install: true,
        };
        assert_eq!(run(&args, Path::new("/nowhere")).await, ExitCode::from(1));
    }
}
