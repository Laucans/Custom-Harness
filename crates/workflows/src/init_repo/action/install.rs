//! The install: a throwaway clone of the target, the architecture's files
//! written where they are missing, one commit, one push.
//!
//! The only write `init-repo` makes into the repository's own tree. It goes
//! through `git` rather than the contents API so the sixteen files land as
//! **one** commit a human can read and revert, and so the clone is reused
//! by the next run (`fetch`, `checkout --force`, `reset --hard`) instead of
//! downloaded again.

use std::path::{Path, PathBuf};

use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::process::Ran;

use crate::init_repo::config::Config;
use crate::init_repo::data::install::{COMMIT_MESSAGE, InstallPlan};
use crate::init_repo::ports::Ports;

/// Installs the rules, and says what it did — one line per fact, as the
/// report prints them.
///
/// # Errors
/// [`Halt::Failed`] when `git` refuses a step: a clone that fails, a push
/// that is rejected. Nothing is retried; the next run starts from the clone
/// as it was left.
pub async fn run(ports: &Ports, config: &Config) -> Outcome<Vec<String>> {
    if !config.install {
        return Ok(vec!["skipped (--no-install)".to_string()]);
    }
    let clone = clone_dir(config);
    let plan = {
        let checkout = mount(ports, config, &clone).await?;
        if checkout.is_none() {
            // A dry run clones nothing: the plan is decided against an empty
            // tree, which is what a fresh repository looks like.
            InstallPlan::new(&|_| None, config.force)
        } else {
            InstallPlan::new(
                &|path| ports.disk.read_to_string(&clone.join(path)),
                config.force,
            )
        }
    };

    let mut lines = Vec::new();
    if config.dry_run {
        lines.push(format!(
            "would install {} file(s) on {} (read against an empty tree)",
            plan.writes.len(),
            config.branch
        ));
        return Ok(lines);
    }
    if plan.is_empty() {
        lines.push(format!(
            "nothing to install — {} already carries every file",
            config.branch
        ));
        return Ok(lines);
    }
    for (path, content) in &plan.writes {
        let target = clone.join(path);
        if let Some(parent) = target.parent() {
            ports.disk.create_dir_all(parent)?;
        }
        ports.disk.write_to_string(&target, content)?;
    }
    let repo = ports.repos.at(&clone);
    said("stage", &repo.stage_all().await?)?;
    said("commit", &repo.commit(COMMIT_MESSAGE).await?)?;
    said("push", &repo.push(&config.branch).await?)?;
    lines.push(format!(
        "installed {} file(s) on {}: {}",
        plan.writes.len(),
        config.branch,
        summary(&plan.writes)
    ));
    if !plan.kept.is_empty() {
        lines.push(format!(
            "kept as they were: {}{}",
            plan.kept.join(", "),
            if config.force {
                ""
            } else {
                " (rerun with --force to rewrite them)"
            }
        ));
    }
    Ok(lines)
}

/// Where the target is cloned: one folder per repository, reused.
fn clone_dir(config: &Config) -> PathBuf {
    config
        .workdir
        .join(format!("{}-{}", config.slug.owner, config.slug.name))
}

/// The checkout the install reads and writes, mounted: cloned the first
/// time, brought back to `origin/<branch>` after. `None` under `--dry-run`,
/// which mounts nothing.
async fn mount(ports: &Ports, config: &Config, clone: &Path) -> Outcome<Option<()>> {
    if config.dry_run {
        return Ok(None);
    }
    if ports.disk.exists(clone) {
        let repo = ports.repos.at(clone);
        said("fetch", &repo.fetch().await?)?;
        said("checkout", &repo.checkout(&config.branch, true).await?)?;
        said(
            "reset",
            &repo
                .reset_hard(&format!("origin/{}", config.branch))
                .await?,
        )?;
    } else {
        ports.disk.create_dir_all(&config.workdir)?;
        let name = clone
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let url = format!(
            "https://github.com/{}/{}.git",
            config.slug.owner, config.slug.name
        );
        said(
            "clone",
            &ports
                .repos
                .at(&config.workdir)
                .clone_repo(&url, &name)
                .await?,
        )?;
        said(
            "checkout",
            &ports.repos.at(clone).checkout(&config.branch, true).await?,
        )?;
    }
    Ok(Some(()))
}

/// What `git` said, read as a verdict: the last line of its stderr is all a
/// human needs when a step refuses.
fn said(verb: &str, ran: &Ran) -> Outcome<()> {
    if ran.ok() {
        return Ok(());
    }
    let why = ran
        .stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("no output")
        .trim();
    Err(Halt::Failed(format!(
        "git {verb} failed in the install clone: {why}"
    )))
}

/// The written paths, grouped so the line stays readable: the contracts
/// counted rather than listed.
fn summary(writes: &[(String, String)]) -> String {
    let contracts = writes
        .iter()
        .filter(|(path, _)| path.starts_with("contracts/"))
        .count();
    let mut parts: Vec<String> = writes
        .iter()
        .filter(|(path, _)| !path.starts_with("contracts/"))
        .map(|(path, _)| path.clone())
        .collect();
    if contracts > 0 {
        parts.push(format!("contracts/ ({contracts} files)"));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_github::FakeGitHub;
    use crate::init_repo::config::fake as config_fake;
    use crate::init_repo::data::install::{CLAUDE_MD, FILES, MARKER_OPEN};
    use std::rc::Rc;

    fn ports(disk: Rc<FakeDisk>, repos: FakeRepos) -> Ports {
        Ports {
            gh: Rc::new(FakeGitHub::default()),
            disk,
            repos: Rc::new(repos),
        }
    }

    #[tokio::test]
    async fn a_fresh_target_is_cloned_filled_committed_and_pushed() {
        let disk = Rc::new(FakeDisk::default());
        let repos = FakeRepos::default();
        let config = config_fake::config();
        let lines = run(&ports(Rc::clone(&disk), repos.clone()), &config)
            .await
            .expect("installed");
        let calls = repos.calls();
        assert_eq!(
            calls,
            vec![
                "/tmp/init: clone https://github.com/o/r.git o-r".to_string(),
                "/tmp/init/o-r: checkout main_agent --force".to_string(),
                "/tmp/init/o-r: stage_all".to_string(),
                format!("/tmp/init/o-r: commit {COMMIT_MESSAGE}"),
                "/tmp/init/o-r: push main_agent".to_string(),
            ]
        );
        let written = disk.written.borrow();
        assert_eq!(written.len(), FILES.len() + 1, "every asset and CLAUDE.md");
        assert!(written.iter().any(
            |(path, text)| path.ends_with("docs/ARCHITECTURE.md") && text.contains(MARKER_OPEN)
        ));
        assert!(lines[0].starts_with("installed 17 file(s) on main_agent"));
        assert!(lines[0].contains("contracts/ (13 files)"));
    }

    #[tokio::test]
    async fn an_existing_clone_is_refreshed_not_cloned_again_and_its_files_are_kept() {
        let clone = PathBuf::from("/tmp/init/o-r");
        let mut existing = std::collections::HashMap::new();
        existing.insert(clone.clone(), String::new());
        existing.insert(
            clone.join("contracts/a-concept.schema.json"),
            "{\"mine\": true}".to_string(),
        );
        existing.insert(clone.join(CLAUDE_MD), "# r\n\nmine.\n".to_string());
        let disk = Rc::new(FakeDisk {
            existing,
            ..FakeDisk::default()
        });
        let repos = FakeRepos::default();
        let lines = run(
            &ports(Rc::clone(&disk), repos.clone()),
            &config_fake::config(),
        )
        .await
        .expect("installed");
        assert!(!repos.asked("clone"));
        assert!(repos.asked("fetch") && repos.asked("reset_hard origin/main_agent"));
        let written = disk.written.borrow();
        assert!(
            !written
                .iter()
                .any(|(path, _)| path.ends_with("a-concept.schema.json")),
            "a human's contract is not overwritten"
        );
        let claude = written
            .iter()
            .find(|(path, _)| path.ends_with(CLAUDE_MD))
            .expect("CLAUDE.md appended");
        assert!(
            claude
                .1
                .starts_with("# r\n\nmine.\n\n<!-- harness:architecture v1 -->")
        );
        assert!(lines[1].contains("kept as they were: contracts/a-concept.schema.json"));
    }

    #[tokio::test]
    async fn a_target_that_has_everything_gets_no_commit() {
        let clone = PathBuf::from("/tmp/init/o-r");
        let mut existing = std::collections::HashMap::new();
        existing.insert(clone.clone(), String::new());
        for asset in &FILES {
            existing.insert(clone.join(asset.path), asset.content.to_string());
        }
        existing.insert(
            clone.join(CLAUDE_MD),
            crate::init_repo::data::install::CLAUDE_RULES.to_string(),
        );
        let disk = Rc::new(FakeDisk {
            existing,
            ..FakeDisk::default()
        });
        let repos = FakeRepos::default();
        let lines = run(
            &ports(Rc::clone(&disk), repos.clone()),
            &config_fake::config(),
        )
        .await
        .expect("nothing to do");
        assert!(!repos.asked("commit") && !repos.asked("push"));
        assert!(disk.written.borrow().is_empty());
        assert!(lines[0].starts_with("nothing to install"));
    }

    #[tokio::test]
    async fn a_dry_run_clones_nothing_writes_nothing_and_says_what_it_would() {
        let disk = Rc::new(FakeDisk::default());
        let repos = FakeRepos::default();
        let mut config = config_fake::config();
        config.dry_run = true;
        let lines = run(&ports(Rc::clone(&disk), repos.clone()), &config)
            .await
            .expect("dry");
        assert_eq!(repos.calls(), [] as [std::string::String; 0]);
        assert!(disk.written.borrow().is_empty());
        assert_eq!(
            lines,
            ["would install 17 file(s) on main_agent (read against an empty tree)"]
        );
    }

    #[tokio::test]
    async fn no_install_skips_everything() {
        let repos = FakeRepos::default();
        let mut config = config_fake::config();
        config.install = false;
        let lines = run(&ports(Rc::new(FakeDisk::default()), repos.clone()), &config)
            .await
            .expect("skipped");
        assert_eq!(lines, ["skipped (--no-install)"]);
        assert_eq!(repos.calls(), [] as [std::string::String; 0]);
    }

    #[tokio::test]
    async fn a_refused_push_is_a_failure_that_names_the_step() {
        let repos = FakeRepos::failing(&["push"]);
        let err = run(
            &ports(Rc::new(FakeDisk::default()), repos.clone()),
            &config_fake::config(),
        )
        .await
        .expect_err("push refused");
        assert!(matches!(err, Halt::Failed(ref why) if why.contains("git push failed")));
    }
}
