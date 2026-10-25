//! A test-debt pull request, landed by the harness: merged once green, or
//! dropped — never repaired.
//!
//! A session merges no PR (`gh pr merge` is denied to it since 28a9439), so
//! a debt PR the session opened stayed open forever: nothing reviews a
//! `test/*` branch (`pr_review::data::skip_rules`), and nothing merged it.
//! The session now ends at the open PR, labelled `harness:debt-pr` by
//! the workflow itself (`action::publish::LabelThePr`); the router reads the
//! labelled PRs and asks [`ready`] what each one is.
//!
//! **One request, one attempt**: a PR with a failing check, a conflict, or
//! a path a test-only PR may not touch ([`forbidden`]) is dropped — closed
//! with the reason, its issue left open for the next rotation to label
//! again. No repair, no decision. Each attempt pushes a branch of its own
//! (`test/debt-<n>-<k>`): a dropped PR's branch stays on the remote, and a
//! second attempt on the same name would be refused at its push.

use harness_core::domain::{Outcome, Pr};
use harness_core::ports::shell::github::GitHub;

/// What every debt branch starts with, before the issue's number.
pub const BRANCH_PREFIX: &str = "test/debt-";

/// The branch attempt `attempt` of debt issue `issue` pushes.
#[must_use]
pub fn branch(issue: u64, attempt: u32) -> String {
    format!("{BRANCH_PREFIX}{issue}-{attempt}")
}

/// The debt issue a branch is for: `test/debt-<n>` or `test/debt-<n>-<k>`.
#[must_use]
pub fn debt_issue(head: &str) -> Option<u64> {
    let rest = head.strip_prefix(BRANCH_PREFIX)?;
    let (issue, attempt) = rest.split_once('-').unwrap_or((rest, ""));
    let digits = |text: &str| !text.is_empty() && text.chars().all(|c| c.is_ascii_digit());
    if !digits(issue) || !(attempt.is_empty() || digits(attempt)) {
        return None;
    }
    issue.parse().ok().filter(|n| *n > 0)
}

/// The next attempt's number for debt issue `issue`, given the branches the
/// remote holds: one past the highest attempt pushed, from 1.
#[must_use]
pub fn next_attempt(issue: u64, branches: &[String]) -> u32 {
    branches
        .iter()
        .filter(|name| debt_issue(name) == Some(issue))
        .map(|name| {
            name.rsplit_once('-')
                .and_then(|(_, k)| k.parse::<u32>().ok())
                .filter(|_| name.matches('-').count() >= 2)
                .unwrap_or(0)
        })
        .max()
        .map_or(1, |highest| highest.saturating_add(1))
}

/// The paths of `files` a test-only PR may not touch: the CI, the rules and
/// their tests, the scripts, the contracts, any manifest.
#[must_use]
pub fn forbidden(files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|path| {
            path.starts_with(".github/")
                || *path == "rules.json"
                || path.starts_with("tests/rules/")
                || path.starts_with("scripts/")
                || path.starts_with("contracts/")
                || path.ends_with("capability.json")
        })
        .cloned()
        .collect()
}

/// What the harness does with one debt PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landing {
    /// Green, mergeable, tests only, into the integration branch: merged,
    /// its issue closed.
    Merge {
        /// The debt issue.
        issue: u64,
        /// The PR.
        pr: Pr,
    },
    /// A failing check, a conflict, a forbidden path: closed with `why`,
    /// the issue left for the next rotation.
    Drop {
        /// The debt issue.
        issue: u64,
        /// The PR.
        pr: Pr,
        /// Why, in a sentence.
        why: String,
        /// Dropped for a failing check — what a red integration branch
        /// fails on every PR alike: held while the base is red
        /// (`routing`), the tests themselves may be fine.
        red: bool,
    },
}

/// Each open debt PR of `open` (labelled `harness:debt-pr`) that
/// lands now, in the order read: one whose checks still run is left.
///
/// # Errors
/// [`harness_core::domain::Halt`] from a files, mergeability or checks
/// read: a routing read that fails fails the tick.
pub async fn ready(gh: &dyn GitHub, open: &[Pr], integration: &str) -> Outcome<Vec<Landing>> {
    let mut found = Vec::new();
    for pr in open.iter().filter(|pr| pr.is_open()) {
        let Some(issue) = debt_issue(&pr.head) else {
            continue;
        };
        let drop = |why: String| Landing::Drop {
            issue,
            pr: pr.clone(),
            why,
            red: false,
        };
        if pr.base != integration {
            found.push(drop(format!(
                "it targets `{}`, not the integration branch `{integration}`",
                pr.base
            )));
            continue;
        }
        let touched = forbidden(&gh.pr_files(&pr.num).await?);
        if !touched.is_empty() {
            found.push(drop(format!(
                "a test-only PR may not touch {}",
                touched.join(", ")
            )));
            continue;
        }
        if gh.pr_mergeable(&pr.num).await? == Some(false) {
            found.push(drop(format!("it conflicts with `{integration}`")));
            continue;
        }
        let failing = gh.pr_failing_checks(&pr.num).await?;
        if !failing.is_empty() {
            found.push(Landing::Drop {
                issue,
                pr: pr.clone(),
                why: format!("its checks failed: {}", failing.join("; ")),
                red: true,
            });
            continue;
        }
        if gh.pr_checks_green(&pr.num).await? {
            found.push(Landing::Merge {
                issue,
                pr: pr.clone(),
            });
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use proptest::prelude::*;
    use std::collections::HashMap;

    proptest! {
        #[test]
        fn debt_issue_reads_back(issue in 1u64..1_000_000, attempt in 1u32..50) {
            prop_assert_eq!(debt_issue(&branch(issue, attempt)), Some(issue));
            prop_assert_eq!(debt_issue(&format!("{BRANCH_PREFIX}{issue}")), Some(issue));
        }

        #[test]
        fn anything_else_reads_as_no_debt(head in "[a-z/-]{0,20}") {
            prop_assert_eq!(debt_issue(&head), None);
        }
    }

    #[test]
    fn a_new_attempt_never_reuses_a_branch_the_remote_holds() {
        assert_eq!(next_attempt(12, &[]), 1);
        let held = [
            "test/debt-12".to_string(),
            "test/debt-12-2".to_string(),
            "test/debt-123-9".to_string(),
        ];
        assert_eq!(next_attempt(12, &held), 3);
        assert_eq!(next_attempt(123, &held), 10);
        assert_eq!(next_attempt(7, &held), 1);
    }

    #[test]
    fn forbidden_paths_are_named() {
        let files: Vec<String> = [
            "crates/a/src/lib.rs",
            ".github/workflows/ci.yml",
            "rules.json",
            "tests/rules/business/src/r1.rs",
            "scripts/test-scope.sh",
            "contracts/B.md",
            "crates/a/capabilities/x/capability.json",
            "crates/a/data-capabilities/y/data-capability.json",
            "crates/a/tests/it.rs",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(
            forbidden(&files),
            files[1..8].to_vec(),
            "the code and its tests are allowed, nothing else"
        );
    }

    fn pr(num: &str, head: &str) -> Pr {
        Pr {
            num: num.to_string(),
            head: head.to_string(),
            base: "main_agent".to_string(),
            state: "OPEN".to_string(),
            ..Pr::default()
        }
    }

    #[tokio::test]
    async fn green_mergeable_test_only_merges_and_pending_waits() {
        let gh = FakeGitHub {
            pr_files: HashMap::from([
                ("40".to_string(), vec!["crates/a/src/lib.rs".to_string()]),
                ("41".to_string(), vec!["crates/a/tests/it.rs".to_string()]),
            ]),
            failing_checks: HashMap::from([
                ("40".to_string(), Vec::new()),
                ("41".to_string(), Vec::new()),
            ]),
            pr_checks: HashMap::from([("40".to_string(), true), ("41".to_string(), false)]),
            ..FakeGitHub::default()
        };
        let found = ready(
            &gh,
            &[pr("40", "test/debt-7-1"), pr("41", "test/debt-8-1")],
            "main_agent",
        )
        .await
        .expect("read");
        assert_eq!(
            found,
            [Landing::Merge {
                issue: 7,
                pr: pr("40", "test/debt-7-1")
            }],
            "#41's checks still run: nothing yet"
        );
    }

    #[tokio::test]
    async fn red_conflicting_or_forbidden_drops_with_why() {
        let gh = FakeGitHub {
            pr_files: HashMap::from([
                ("50".to_string(), vec!["crates/a/src/lib.rs".to_string()]),
                ("51".to_string(), vec!["crates/a/src/lib.rs".to_string()]),
                (
                    "52".to_string(),
                    vec![".github/workflows/ci.yml".to_string()],
                ),
            ]),
            pr_mergeable: HashMap::from([("51".to_string(), Some(false))]),
            failing_checks: HashMap::from([("50".to_string(), vec!["rust".to_string()])]),
            ..FakeGitHub::default()
        };
        let found = ready(
            &gh,
            &[
                pr("50", "test/debt-1-1"),
                pr("51", "test/debt-2-1"),
                pr("52", "test/debt-3-1"),
                pr("53", "feat/not-debt"),
            ],
            "main_agent",
        )
        .await
        .expect("read");
        let whys: Vec<(u64, String)> = found
            .iter()
            .map(|landing| match landing {
                Landing::Drop {
                    issue, why, red, ..
                } => (*issue, format!("{red} {why}")),
                Landing::Merge { issue, .. } => (*issue, "merge".to_string()),
            })
            .collect();
        assert_eq!(
            whys,
            [
                (1, "true its checks failed: rust".to_string()),
                (2, "false it conflicts with `main_agent`".to_string()),
                (
                    3,
                    "false a test-only PR may not touch .github/workflows/ci.yml".to_string()
                ),
            ]
        );
    }
}
