//! The report `init-repo` prints — the command's user interface.
//!
//! Pure: a `String`, built from values already decided elsewhere.

/// What the integration branch line says.
#[derive(Debug)]
pub enum BranchReportLine {
    /// The branch already existed — never fast-forwarded, never touched.
    AlreadyExists,
    /// The branch was (or, under `--dry-run`, would be) created at this sha.
    Created {
        /// The sha it was created at — the default branch's tip.
        sha: String,
        /// The repo's default branch, named for context.
        default: String,
    },
}

/// Everything `init-repo` reports after one run.
#[derive(Debug)]
pub struct Report {
    /// `owner/name`.
    pub repo: String,
    /// The integration branch.
    pub branch: String,
    /// The labels created (or, under `--dry-run`, that would be).
    pub labels_created: Vec<String>,
    /// How many of the eight labels already existed.
    pub labels_kept: usize,
    /// What the integration branch line says.
    pub branch_line: BranchReportLine,
    /// The link line, already rendered — it depends on `--no-env`/`--force`
    /// in ways only [`crate::init_repo::action::apply`] knows.
    pub env_line: String,
    /// What still blocks the first paid run: (what, the fix).
    pub blocking: Vec<(String, String)>,
    /// Advisory-only findings — the review hook is advisory by design.
    pub advisory: Vec<String>,
    /// Whether this was a `--dry-run`: writes are described as "would".
    pub dry_run: bool,
}

/// What the report resolves to.
///
/// Distinct from [`harness_core::domain::Verdict`] (no `Skip`/`NothingLeft`
/// — those don't apply to an idempotent one-shot command). Never
/// re-exported past `init_repo::data`.
#[derive(Debug)]
pub enum Verdict {
    /// Nothing remains blocking: the first paid run can proceed.
    Ready,
    /// At least one BLOCKING line remains.
    StillBlocking,
}

impl Report {
    /// Renders the report, exactly as a human reads it.
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;

        let mut out = format!("init-repo {} — branch {}\n\n", self.repo, self.branch);

        let labels_word = if self.dry_run {
            "would create"
        } else {
            "created"
        };
        let _ = write!(
            out,
            "{:<12}{:<7} {}",
            "labels",
            labels_word,
            self.labels_created.len()
        );
        if !self.labels_created.is_empty() {
            let _ = write!(out, "  {}", self.labels_created.join(" "));
        }
        out += "\n";
        let _ = writeln!(out, "{:<12}{:<7} {}", "", "kept", self.labels_kept);

        let branch_value = match &self.branch_line {
            BranchReportLine::AlreadyExists => format!("{} already exists", self.branch),
            BranchReportLine::Created { sha, default } => {
                let verb = if self.dry_run {
                    "would be created"
                } else {
                    "created"
                };
                format!("{} {verb} at {sha} (default: {default})", self.branch)
            }
        };
        let _ = writeln!(out, "{:<12}{branch_value}", "branch");
        let _ = writeln!(out, "{:<12}{}", "link", self.env_line);

        if !self.blocking.is_empty() {
            out += "\nstill blocking the first run:\n";
            for (what, fix) in &self.blocking {
                let _ = writeln!(out, "  {what}\n    → {fix}");
            }
        }
        if !self.advisory.is_empty() {
            out += "\nadvisory:\n";
            for line in &self.advisory {
                let _ = writeln!(out, "  {line}");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_matches_the_spec_block_verbatim() {
        let report = Report {
            repo: "Laucans/event_assistant".to_string(),
            branch: "main_agent".to_string(),
            labels_created: vec![
                "harness:ready".to_string(),
                "harness:spec-written".to_string(),
                "harness:waiting-merge".to_string(),
            ],
            labels_kept: 5,
            branch_line: BranchReportLine::Created {
                sha: "9f21ac3".to_string(),
                default: "main".to_string(),
            },
            env_line: ".env.local — TARGET_REPO_URL set, INTEGRATION_BRANCH set".to_string(),
            blocking: vec![
                (
                    ".github/workflows/ci.yml does not trigger on main_agent".to_string(),
                    "add main_agent to the push/pull_request branches of .github/workflows/ci.yml"
                        .to_string(),
                ),
                (
                    ".claude/skills/create-test/SKILL.md is missing".to_string(),
                    "copy the skill into the target repo".to_string(),
                ),
            ],
            advisory: vec![
                ".claude/settings.json declares no pr-review hook on `gh pr create`".to_string(),
            ],
            dry_run: false,
        };

        let expected = concat!(
            "init-repo Laucans/event_assistant — branch main_agent\n",
            "\n",
            "labels      created 3  harness:ready harness:spec-written harness:waiting-merge\n",
            "            kept    5\n",
            "branch      main_agent created at 9f21ac3 (default: main)\n",
            "link        .env.local — TARGET_REPO_URL set, INTEGRATION_BRANCH set\n",
            "\n",
            "still blocking the first run:\n",
            "  .github/workflows/ci.yml does not trigger on main_agent\n",
            "    → add main_agent to the push/pull_request branches of .github/workflows/ci.yml\n",
            "  .claude/skills/create-test/SKILL.md is missing\n",
            "    → copy the skill into the target repo\n",
            "\n",
            "advisory:\n",
            "  .claude/settings.json declares no pr-review hook on `gh pr create`\n",
        );

        assert_eq!(report.render(), expected);
    }

    #[test]
    fn zero_creations_omits_the_label_list() {
        let report = Report {
            repo: "o/r".to_string(),
            branch: "main_agent".to_string(),
            labels_created: vec![],
            labels_kept: 8,
            branch_line: BranchReportLine::AlreadyExists,
            env_line: ".env.local — TARGET_REPO_URL set, INTEGRATION_BRANCH set".to_string(),
            blocking: vec![],
            advisory: vec![],
            dry_run: false,
        };
        assert!(report.render().contains("created 0\n"));
        assert!(!report.render().contains("still blocking"));
        assert!(!report.render().contains("advisory"));
    }
}
