//! Sequences the writes `init-repo` makes, honoring `--dry-run`/`--no-env`.
//!
//! The one place that follows the spec's steps 2 through 7 in order:
//! preconditions, labels, the branch, the install of the architecture's
//! files, the read-only audit, the link, the verdict.

use harness_core::domain::{Halt, Outcome};

use crate::common::labels;
use crate::init_repo::action::install;
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
    let plan = Plan::new(&existing, branch_sha_now.as_deref(), &default_sha);

    let labels_created = create_missing_labels(ports, config, &plan).await?;
    create_grill_backlog_label_if_needed(ports, config, &existing).await?;
    let branch_line = create_branch_if_needed(ports, config, &plan, &default).await?;
    let (mut guard_lines, mut protection_advisory) =
        guard_branches(ports, config, &default).await?;
    let (policy_lines, policy_advisory) = require_squash_merges(ports, config).await?;
    guard_lines.extend(policy_lines);
    protection_advisory.extend(policy_advisory);
    let install_lines = install::run(ports, config).await?;
    // After the install: a `CLAUDE.md` it just pushed is one the map reads.
    let audit = read_audit(ports, config).await?;
    let env_line = if config.write_env {
        apply_env_file(ports, config)?
    } else {
        "skipped (--no-env)".to_string()
    };

    let (blocking, mut advisory) = blocking_and_advisory(&audit, &config.branch);
    advisory.extend(protection_advisory);
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
            guard_lines,
            install_lines,
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
    // The skills are deliberately **not** audited here. They are the
    // harness's, lent into the clone at every mount
    // (`launcher::dispatch::skills`), so a target repo will never carry them
    // and reporting them missing would send a human to commit copies of the
    // harness into their own project — which is what this audit used to do.
    let claude_md = ports.gh.file_text("CLAUDE.md", &config.branch).await?;
    Ok(Audit {
        ci_triggers: audit::ci_triggers_on(ci.as_deref(), &config.branch),
        ci_triggers_on_milestones: audit::ci_triggers_on_milestones(ci.as_deref()),
        has_claude_md: audit::has_claude_md(claude_md.as_deref()),
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

/// Creates `grill:backlog` if absent, skipping the write under `--dry-run`.
///
/// Deliberately **not** folded into `create_missing_labels`/`plan.missing_labels`:
/// those are counted against `labels::ALL`, and this label lives outside
/// that namespace on purpose (`audit::GRILL_BACKLOG`'s own doc explains
/// why). A separate, small side effect, not reflected in the report.
async fn create_grill_backlog_label_if_needed(
    ports: &Ports,
    config: &Config,
    existing: &[String],
) -> Outcome<()> {
    let label = audit::GRILL_BACKLOG;
    if existing.iter().any(|name| name == label.name) {
        return Ok(());
    }
    if !config.dry_run {
        ports
            .gh
            .create_label(label.name, label.color, label.description)
            .await?;
    }
    Ok(())
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

/// Makes the integration branch the default one, and protects the release
/// branch it replaces.
///
/// Why: `Closes #n` only fires on a merge into the default branch, so a
/// default that is not the integration branch leaves every finished
/// milestone open and the loop stuck on it. The release branch (`main`) is
/// then protected: it only moves through a pull request.
///
/// Returns the report lines and the advisories. A refused protection is an
/// advisory, not an error: a private repo on a free plan cannot have any.
async fn guard_branches(
    ports: &Ports,
    config: &Config,
    default: &str,
) -> Outcome<(Vec<String>, Vec<String>)> {
    let mut lines = Vec::new();
    let mut advisory = Vec::new();
    let tense = |done: &'static str, would: &'static str| {
        if config.dry_run { would } else { done }
    };

    if default == config.branch {
        lines.push(format!("{} is already the default branch", config.branch));
    } else {
        if !config.dry_run {
            ports.gh.set_default_branch(&config.branch).await?;
        }
        lines.push(format!(
            "default branch {} {} {default}",
            tense("is now", "would become"),
            config.branch
        ));
    }

    let release = if default == config.branch {
        "main"
    } else {
        default
    };
    if release == config.branch || ports.gh.branch_sha(release).await?.is_none() {
        return Ok((lines, advisory));
    }
    if config.dry_run {
        lines.push(format!("{release} would be protected"));
        return Ok((lines, advisory));
    }
    match ports.gh.protect_branch(release).await {
        Ok(()) => lines.push(format!("{release} is protected (pull request only)")),
        Err(halt) => advisory.push(format!(
            "{release} is not protected — {} (protect it by hand, or move \
             to a plan that offers it)",
            halt.reason()
        )),
    }
    Ok((lines, advisory))
}

/// Allows only squash merges, the squash commit keeping the pull request's
/// title and number.
///
/// Why: the integration branch then carries one commit per pull request,
/// and the `(#n)` in its title is the way back to the branch's own commits —
/// the history is not lost, it is read on the pull request. A refusal is an
/// advisory, not an error: a token may push without administering.
async fn require_squash_merges(
    ports: &Ports,
    config: &Config,
) -> Outcome<(Vec<String>, Vec<String>)> {
    const WHAT: &str = "squash merges only (commit: PR title (#n), PR body)";
    if config.dry_run {
        return Ok((vec![format!("merges would be {WHAT}")], Vec::new()));
    }
    match ports.gh.require_squash_merges().await {
        Ok(()) => Ok((vec![format!("merges are {WHAT}")], Vec::new())),
        Err(halt) => Ok((
            Vec::new(),
            vec![format!(
                "merges are not squash only — {} (set it by hand in the \
                 repository's settings, under Pull Requests)",
                halt.reason()
            )],
        )),
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
    if !audit.ci_triggers_on_milestones {
        blocking.push((
            ".github/workflows/ci.yml does not trigger on milestone branches".to_string(),
            "add 'milestone/**' to the push/pull_request branches of \
             .github/workflows/ci.yml"
                .to_string(),
        ));
    }
    let advisory = if audit.has_claude_md {
        Vec::new()
    } else {
        vec![
            "no CLAUDE.md — the repository map reads it first, so a planner \
             run would work from the roadmap issue alone"
                .to_string(),
        ]
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
    use crate::common::fake_github::{FakeGitHub, Wrote};
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
        gh.files.insert(
            (
                ".github/workflows/ci.yml".to_string(),
                "main_agent".to_string(),
            ),
            "on:\n  push:\n    branches: [main_agent, 'milestone/**']\n".to_string(),
        );
        gh.files.insert(
            ("CLAUDE.md".to_string(), "main_agent".to_string()),
            "# the project\n\nwhat it is.\n".to_string(),
        );
        gh
    }

    #[tokio::test]
    async fn a_second_run_creates_nothing() {
        let ports = ports_fake::with(Rc::new(ready_gh()));
        let config = config_fake::config();
        let (report, verdict) = run(&ports, &config).await.expect("run");
        assert_eq!(report.labels_created, [] as [std::string::String; 0]);
        assert_eq!(report.labels_kept, labels::ALL.len());
        assert!(matches!(verdict, Verdict::Ready));
    }

    #[tokio::test]
    async fn the_integration_branch_becomes_the_default_and_main_is_protected() {
        let gh = Rc::new(ready_gh());
        let ports = ports_fake::with(Rc::clone(&gh));
        let (report, _) = run(&ports, &config_fake::config()).await.expect("run");
        let writes = gh.writes();
        assert!(writes.contains(&Wrote::DefaultBranch("main_agent".to_string())));
        assert!(writes.contains(&Wrote::ProtectedBranch("main".to_string())));
        assert!(writes.contains(&Wrote::SquashMergesRequired));
        assert_eq!(report.guard_lines.len(), 3);
        assert!(
            report
                .guard_lines
                .iter()
                .any(|l| l == "merges are squash merges only (commit: PR title (#n), PR body)")
        );
    }

    #[tokio::test]
    async fn a_refused_squash_policy_is_an_advisory_not_a_failure() {
        let mut gh = ready_gh();
        gh.squash_merges_refused = true;
        let ports = ports_fake::with(Rc::new(gh));
        let (report, verdict) = run(&ports, &config_fake::config()).await.expect("run");
        assert!(matches!(verdict, Verdict::Ready));
        assert!(
            report
                .advisory
                .iter()
                .any(|a| a.contains("merges are not squash only"))
        );
        assert!(!report.guard_lines.iter().any(|l| l.contains("squash")));
    }

    #[tokio::test]
    async fn a_refused_protection_is_an_advisory_not_a_failure() {
        let mut gh = ready_gh();
        gh.protection_refused = true;
        let ports = ports_fake::with(Rc::new(gh));
        let (report, verdict) = run(&ports, &config_fake::config()).await.expect("run");
        assert!(matches!(verdict, Verdict::Ready));
        assert!(
            report
                .advisory
                .iter()
                .any(|a| a.contains("main is not protected"))
        );
    }

    #[tokio::test]
    async fn a_second_run_still_protects_main_without_changing_the_default() {
        let mut gh = ready_gh();
        gh.default_branch_name = Some("main_agent".to_string());
        let gh = Rc::new(gh);
        let ports = ports_fake::with(Rc::clone(&gh));
        run(&ports, &config_fake::config()).await.expect("run");
        let writes = gh.writes();
        assert!(!writes.iter().any(|w| matches!(w, Wrote::DefaultBranch(_))));
        assert!(writes.contains(&Wrote::ProtectedBranch("main".to_string())));
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
        assert_eq!(report.labels_created.len(), labels::ALL.len());
        assert!(gh.writes().is_empty(), "the fake gh recorded a write");
        assert!(report.env_line.contains("would be set"));
        assert!(
            report
                .guard_lines
                .iter()
                .any(|l| l.starts_with("merges would be squash"))
        );
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
        config.install = false;

        let (report, _) = run(&ports, &config).await.expect("run");
        assert_eq!(report.env_line, "skipped (--no-env)");
        assert!(disk.written.borrow().is_empty());
    }

    #[tokio::test]
    async fn grill_backlog_is_created_when_absent_but_never_counted_in_the_report() {
        let gh = Rc::new(ready_gh());
        let ports = ports_fake::with(Rc::clone(&gh));
        let config = config_fake::config();

        let (report, _) = run(&ports, &config).await.expect("run");
        assert_eq!(
            report.labels_created.len(),
            0,
            "ALL's own labels, unaffected"
        );
        assert!(
            gh.writes().iter().any(|w| matches!(
                w,
                crate::common::fake_github::Wrote::CreatedLabel(name, _, _)
                    if name == audit::GRILL_BACKLOG.name
            )),
            "grill:backlog must still be created as a side effect"
        );
    }

    #[tokio::test]
    async fn grill_backlog_already_present_is_not_recreated() {
        let mut gh = ready_gh();
        gh.labels.push(audit::GRILL_BACKLOG.name.to_string());
        let gh = Rc::new(gh);
        let ports = ports_fake::with(Rc::clone(&gh));
        let config = config_fake::config();

        run(&ports, &config).await.expect("run");
        assert!(
            !gh.writes().iter().any(|w| matches!(
                w,
                crate::common::fake_github::Wrote::CreatedLabel(name, _, _)
                    if name == audit::GRILL_BACKLOG.name
            )),
            "an existing label must never be recreated"
        );
    }

    #[tokio::test]
    async fn a_ci_yml_with_no_milestone_trigger_still_blocks() {
        let mut gh = ready_gh();
        gh.files.insert(
            (
                ".github/workflows/ci.yml".to_string(),
                "main_agent".to_string(),
            ),
            "on:\n  push:\n    branches: [main_agent]\n".to_string(),
        );
        let ports = ports_fake::with(Rc::new(gh));
        let config = config_fake::config();

        let (report, verdict) = run(&ports, &config).await.expect("run");
        assert!(matches!(verdict, Verdict::StillBlocking));
        assert!(
            report
                .blocking
                .iter()
                .any(|(what, _)| what.contains("milestone branches"))
        );
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
