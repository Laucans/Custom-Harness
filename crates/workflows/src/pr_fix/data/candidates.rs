//! Which open PRs the repair route is due on, read for the router — a few
//! GitHub reads per labelled PR, paid in the tick so a PR that would find
//! nothing to repair never costs a mounted checkout.

use harness_core::domain::{Outcome, Pr};
use harness_core::ports::shell::github::GitHub;

use crate::common::review;

/// The open PRs to repair, in the order read.
///
/// Those carrying `harness:pr-fix` (`to_fix`) **and** a check that
/// concluded in failure — the label alone would send a paid session at a PR
/// still building — or a branch that conflicts with its base, within the
/// repair cap: a milestone's landing PR the catch-up could not bring up
/// (`common::catch_up`) is labelled for its conflict, its checks green. And
/// those carrying `harness:to-review` (`to_review`) whose review blocks, or
/// whose branch conflicts, or a check failed, within the cap: a task's PR
/// ends its session at the push, and nobody else poses `harness:pr-fix` on
/// it (`ForgeDesMondes`, 2026-10-20: five PRs reviewed clean and red on
/// `rust` for up to nine hours, repaired by no one).
///
/// # Errors
/// [`harness_core::domain::Halt`] from any read: a routing read that fails
/// fails the tick.
pub async fn due(gh: &dyn GitHub, to_fix: &[Pr], to_review: &[Pr]) -> Outcome<Vec<Pr>> {
    let mut found: Vec<Pr> = Vec::new();
    for candidate in to_fix {
        if !gh.pr_failing_checks(&candidate.num).await?.is_empty()
            || (gh.pr_mergeable(&candidate.num).await? == Some(false)
                && repairs_left(gh, candidate).await?)
        {
            found.push(candidate.clone());
        }
    }
    for candidate in to_review {
        let comments = gh.pr_comments(&candidate.num).await?;
        let left = review::repairs(&comments) < review::MAX_FIXES;
        let blocked = review::status(&comments).wants_a_fix();
        let conflicting = left && gh.pr_mergeable(&candidate.num).await? == Some(false);
        let red = left && !gh.pr_failing_checks(&candidate.num).await?.is_empty();
        if blocked || conflicting || red {
            found.push(candidate.clone());
        }
    }
    Ok(found)
}

/// Whether `pr` has repairs left under the cap.
async fn repairs_left(gh: &dyn GitHub, pr: &Pr) -> Outcome<bool> {
    Ok(review::repairs(&gh.pr_comments(&pr.num).await?) < review::MAX_FIXES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use std::collections::HashMap;

    fn pr(num: &str) -> Pr {
        Pr {
            num: num.to_string(),
            ..Pr::default()
        }
    }

    fn nums(prs: &[Pr]) -> Vec<&str> {
        prs.iter().map(|pr| pr.num.as_str()).collect()
    }

    fn fixes(count: usize) -> String {
        review::FIX_MARKER.repeat(count)
    }

    #[tokio::test]
    async fn a_labelled_pr_is_due_when_red_or_conflicting_within_the_cap() {
        let gh = FakeGitHub {
            failing_checks: HashMap::from([
                ("1".to_string(), vec!["rust".to_string()]),
                ("2".to_string(), Vec::new()),
                ("3".to_string(), Vec::new()),
                ("4".to_string(), Vec::new()),
                ("5".to_string(), Vec::new()),
            ]),
            pr_mergeable: HashMap::from([
                ("1".to_string(), Some(false)),
                ("2".to_string(), Some(false)),
                ("3".to_string(), Some(false)),
                ("4".to_string(), Some(true)),
                ("5".to_string(), None),
            ]),
            pr_comment_bodies: vec![
                ("1".to_string(), fixes(review::MAX_FIXES)),
                ("2".to_string(), fixes(review::MAX_FIXES - 1)),
                ("3".to_string(), fixes(review::MAX_FIXES)),
            ],
            ..FakeGitHub::default()
        };
        let found = due(&gh, &[pr("1"), pr("2"), pr("3"), pr("4"), pr("5")], &[])
            .await
            .expect("read");
        assert_eq!(
            nums(&found),
            ["1", "2"],
            "red whatever its repairs; conflicting with a repair left; not past the cap, \
             not mergeable, not unknown"
        );
    }

    #[tokio::test]
    async fn a_reviewed_pr_is_due_when_blocked_conflicting_or_red_within_the_cap() {
        let blocking = format!("{}\n{}", review::MARKER, review::BLOCKING);
        let gh = FakeGitHub {
            failing_checks: HashMap::from([
                ("10".to_string(), Vec::new()),
                ("11".to_string(), Vec::new()),
                ("12".to_string(), vec!["rust".to_string()]),
                ("13".to_string(), vec!["rust".to_string()]),
                ("14".to_string(), Vec::new()),
            ]),
            pr_mergeable: HashMap::from([
                ("11".to_string(), Some(false)),
                ("13".to_string(), Some(false)),
            ]),
            pr_comment_bodies: vec![
                ("10".to_string(), blocking),
                ("13".to_string(), fixes(review::MAX_FIXES)),
            ],
            ..FakeGitHub::default()
        };
        let found = due(
            &gh,
            &[],
            &[pr("10"), pr("11"), pr("12"), pr("13"), pr("14")],
        )
        .await
        .expect("read");
        assert_eq!(nums(&found), ["10", "11", "12"]);
    }

    #[tokio::test]
    async fn a_failed_read_fails_the_tick() {
        let gh = FakeGitHub {
            broken: Some(harness_core::domain::Halt::Failed("HTTP 502".to_string())),
            ..FakeGitHub::default()
        };
        assert!(due(&gh, &[], &[pr("1")]).await.is_err());
        assert!(
            due(&gh, &[], &[]).await.expect("nothing read").is_empty(),
            "nothing expected"
        );
    }
}
