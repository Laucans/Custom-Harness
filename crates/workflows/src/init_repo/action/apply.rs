//! Sequences the writes `init-repo` makes, honoring `--dry-run`/`--no-env`.
//!
//! The one place that follows the spec's steps 2 through 7 in order:
//! preconditions, labels, the branch, the read-only audit, the link, the
//! verdict.

use harness_core::domain::{Halt, Outcome};

use crate::common::labels;
use crate::init_repo::config::Config;
use crate::init_repo::data::audit::{self, Audit};
use crate::init_repo::data::env_file;
use crate::init_repo::data::plan::{BranchAction, Plan};
use crate::init_repo::data::report::{BranchReportLine, Report, Verdict};
use crate::init_repo::ports::Ports;

/// Runs one `init-repo` invocation: reads the preconditions, plans the
/// writes, makes them (unless `--dry-run`), and reports.
///
/// # Errors
/// A [`Halt`] from an unreadable precondition, or an env-file conflict
/// without `--force`. A remaining BLOCKING audit line is **not** an `Err` —
/// it comes back as `Ok((report, Verdict::StillBlocking))`; the caller maps
/// that to its own exit code.
pub async fn run(ports: &Ports, config: &Config) -> Outcome<(Report, Verdict)> {
    check_preconditions(ports, config).await?;

    let existing = ports.gh.labels().await?;
    let default = ports.gh.default_branch().await?;
    let default_sha = ports
        .gh
        .branch_sha(&default)
        .await?
        .ok_or_else(|| Halt::Unreadable(format!("default branch {default} has no sha")))?;
    let branch_sha_now = ports.gh.branch_sha(&config.branch).await?;
    let audit = read_audit(ports, config).await?;
    let plan = Plan::new(&existing, branch_sha_now.as_deref(), &default_sha);

    let labels_created = create_missing_labels(ports, config, &plan).await?;
    let branch_line = create_branch_if_needed(ports, config, &plan, &default).await?;
    let env_line = if config.write_env {
        apply_env_file(ports, config)?
    } else {
        "skipped (--no-env)".to_string()
    };

    let (blocking, advisory) = blocking_and_advisory(&audit, &config.branch);
    let verdict = if blocking.is_empty() {
        Verdict::Ready
    } else {
        Verdict::StillBlocking
    };

    Ok((
        Report {
            repo: config.slug.to_string(),
            branch: config.branch.clone(),
            labels_created,
            labels_kept: labels::ALL.len() - plan.missing_labels.len(),
            branch_line,
            env_line,
            blocking,
            advisory,
            dry_run: config.dry_run,
        },
        verdict,
    ))
}

/// Step 2: `gh` authenticated, and push right on the target.
///
/// # Errors
/// [`Halt::Halted`] if either does not hold — nothing below can work without
/// them.
async fn check_preconditions(ports: &Ports, config: &Config) -> Outcome<()> {
    if !ports.gh.authenticated().await? {
        return Err(Halt::Halted(format!(
            "gh is not authenticated — run `gh auth status`, then retry against {}",
            config.slug
        )));
    }
    if !ports.gh.can_push().await? {
        return Err(Halt::Halted(format!(
            "no push right on {} — check `gh auth status` and the repo",
            config.slug
        )));
    }
    Ok(())
}

/// Step 5: the read-only audit, over the branch's own tree.
///
/// Every `file_text` error propagates via `?` as [`Halt::Unreadable`] here,
/// before an [`Audit`] is ever built: a failed read never reads as
/// "missing".
async fn read_audit(ports: &Ports, config: &Config) -> Outcome<Audit> {
    let ci = ports
        .gh
        .file_text(".github/workflows/ci.yml", &config.branch)
        .await?;
    let mut missing_skills = Vec::new();
    for skill in audit::SKILLS {
        let path = format!(".claude/skills/{skill}/SKILL.md");
        if ports.gh.file_text(&path, &config.branch).await?.is_none() {
            missing_skills.push(skill);
        }
    }
    let settings = ports
        .gh
        .file_text(".claude/settings.json", &config.branch)
        .await?;
    Ok(Audit {
        ci_triggers: audit::ci_triggers_on(ci.as_deref(), &config.branch),
        missing_skills,
        pr_review_hook: audit::declares_pr_review_hook(settings.as_deref()),
    })
}

/// Step 3: creates the missing labels, skipping the writes under `--dry-run`.
async fn create_missing_labels(
    ports: &Ports,
    config: &Config,
    plan: &Plan,
) -> Outcome<Vec<String>> {
    let mut labels_created = Vec::new();
    for label in &plan.missing_labels {
        if !config.dry_run {
            ports
                .gh
                .create_label(label.name, label.color, label.description)
                .await?;
        }
        labels_created.push(label.name.to_string());
    }
    Ok(labels_created)
}

/// Step 4: creates the integration branch if it is absent, skipping the
/// write under `--dry-run`.
async fn create_branch_if_needed(
    ports: &Ports,
    config: &Config,
    plan: &Plan,
    default: &str,
) -> Outcome<BranchReportLine> {
    match &plan.branch_action {
        BranchAction::AlreadyExists => Ok(BranchReportLine::AlreadyExists),
        BranchAction::Create { at_sha } => {
            if !config.dry_run {
                ports.gh.create_branch(&config.branch, at_sha).await?;
            }
            Ok(BranchReportLine::Created {
                sha: at_sha.clone(),
                default: default.to_string(),
            })
        }
    }
}

/// Step 7: what the audit turns into BLOCKING and advisory report lines.
fn blocking_and_advisory(audit: &Audit, branch: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut blocking = Vec::new();
    if !audit.ci_triggers {
        blocking.push((
            format!(".github/workflows/ci.yml does not trigger on {branch}"),
            format!("add {branch} to the push/pull_request branches of .github/workflows/ci.yml"),
        ));
    }
    for skill in &audit.missing_skills {
        blocking.push((
            format!(".claude/skills/{skill}/SKILL.md is missing"),
            "copy the skill into the target repo".to_string(),
        ));
    }
    let advisory = if audit.pr_review_hook {
        Vec::new()
    } else {
        vec![".claude/settings.json declares no pr-review hook on `gh pr create`".to_string()]
    };
    (blocking, advisory)
}

/// The link: `.env.local`, updated in place or created from `.env.example`.
///
/// # Errors
/// [`Halt::Halted`] if `TARGET_REPO_URL` is already set to a different repo
/// and `--force` was not given.
fn apply_env_file(ports: &Ports, config: &Config) -> Outcome<String> {
    let text = ports
        .disk
        .read_to_string(&config.env_file)
        .or_else(|| {
            ports
                .disk
                .read_to_string(&config.env_file.with_file_name(".env.example"))
        })
        .unwrap_or_default();

    let target = config.slug.to_string();
    if let Some(existing) = env_file::existing_target_repo_url(&text)
        && existing != target
        && !config.force
    {
        return Err(Halt::Halted(format!(
            "TARGET_REPO_URL is already {existing}, not {target} — rerun with --force to overwrite"
        )));
    }

    let updated = env_file::set_keys(&text, &target, &config.branch);
    if !config.dry_run {
        ports.disk.write_to_string(&config.env_file, &updated)?;
    }
    let verb = if config.dry_run {
        "would be set"
    } else {
        "set"
    };
    Ok(format!(
        "{} — TARGET_REPO_URL {verb}, INTEGRATION_BRANCH {verb}",
        config.env_file.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use crate::init_repo::config::fake as config_fake;
    use crate::init_repo::ports::fake::{self as ports_fake, FakeDisk};
    use std::rc::Rc;

    /// A `FakeGitHub` with every precondition satisfied, all eight labels
    /// already present, the branch absent, and a clean audit — the "nothing
    /// to do" baseline each test starts from and adjusts.
    fn ready_gh() -> FakeGitHub {
        let mut gh = FakeGitHub {
            can_push_answer: Some(true),
            default_branch_name: Some("main".to_string()),
            labels: labels::ALL.iter().map(|l| l.name.to_string()).collect(),
            ..FakeGitHub::default()
        };
        gh.branch_shas
            .insert("main".to_string(), Some("defaultsha".to_string()));
        gh.branch_shas
            .insert("main_agent".to_string(), Some("existingsha".to_string()));
        for skill in audit::SKILLS {
            gh.files.insert(
                (
                    format!(".claude/skills/{skill}/SKILL.md"),
                    "main_agent".to_string(),
                ),
                "# a skill".to_string(),
            );
        }
        gh.files.insert(
            (
                ".github/workflows/ci.yml".to_string(),
                "main_agent".to_string(),
            ),
            "on:\n  push:\n    branches: [main_agent]\n".to_string(),
        );
        gh.files.insert(
            (
                ".claude/settings.json".to_string(),
                "main_agent".to_string(),
            ),
            r#"{"hooks":{"PostToolUse":[{"hooks":[{"command":"pr-review"}]}]}}"#.to_string(),
        );
        gh
    }

    #[tokio::test]
    async fn a_second_run_creates_nothing() {
        let ports = ports_fake::with(Rc::new(ready_gh()));
        let config = config_fake::config();
        let (report, verdict) = run(&ports, &config).await.expect("run");
        assert!(report.labels_created.is_empty());
        assert_eq!(report.labels_kept, 8);
        assert!(matches!(verdict, Verdict::Ready));
    }

    #[tokio::test]
    async fn dry_run_records_zero_writes_on_either_port() {
        let mut gh = ready_gh();
        gh.labels = Vec::new(); // every label missing
        gh.branch_shas.insert("main_agent".to_string(), None); // branch absent
        let gh = Rc::new(gh);
        let ports = ports_fake::with(Rc::clone(&gh));
        let mut config = config_fake::config();
        config.dry_run = true;

        let (report, _verdict) = run(&ports, &config).await.expect("run");
        assert_eq!(report.labels_created.len(), 8);
        assert!(gh.writes().is_empty(), "the fake gh recorded a write");
        assert!(report.env_line.contains("would be set"));
    }

    #[tokio::test]
    async fn a_conflicting_target_repo_url_halts_unless_force() {
        let gh = Rc::new(ready_gh());
        let disk = Rc::new(FakeDisk {
            existing: std::iter::once((
                config_fake::config().env_file,
                "TARGET_REPO_URL=other/repo\n".to_string(),
            ))
            .collect(),
            ..FakeDisk::default()
        });
        let ports = ports_fake::with_disk(gh, Rc::clone(&disk));
        let config = config_fake::config();

        let err = run(&ports, &config).await.expect_err("should halt");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(disk.written_to(&config.env_file).is_none());
    }

    #[tokio::test]
    async fn force_overwrites_a_conflicting_target_repo_url() {
        let gh = Rc::new(ready_gh());
        let env_file = config_fake::config().env_file;
        let disk = Rc::new(FakeDisk {
            existing: std::iter::once((
                env_file.clone(),
                "TARGET_REPO_URL=other/repo\n".to_string(),
            ))
            .collect(),
            ..FakeDisk::default()
        });
        let ports = ports_fake::with_disk(gh, Rc::clone(&disk));
        let mut config = config_fake::config();
        config.force = true;

        run(&ports, &config).await.expect("run");
        assert!(
            disk.written_to(&env_file)
                .unwrap()
                .contains("TARGET_REPO_URL=o/r")
        );
    }

    #[tokio::test]
    async fn no_env_writes_nothing() {
        let gh = Rc::new(ready_gh());
        let disk = Rc::new(FakeDisk::default());
        let ports = ports_fake::with_disk(gh, Rc::clone(&disk));
        let mut config = config_fake::config();
        config.write_env = false;

        let (report, _) = run(&ports, &config).await.expect("run");
        assert_eq!(report.env_line, "skipped (--no-env)");
        assert!(disk.written.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_failed_read_never_reaches_the_audit_as_a_missing_file() {
        let mut gh = ready_gh();
        gh.broken = Some(Halt::Unreadable("token expired".to_string()));
        let ports = ports_fake::with(Rc::new(gh));
        let config = config_fake::config();

        let err = run(&ports, &config).await.expect_err("should halt");
        assert!(matches!(err, Halt::Unreadable(_)));
    }
}
