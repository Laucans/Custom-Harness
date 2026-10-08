//! The one place that sequences a merge attempt: read, decide, write.

use harness_core::domain::Outcome;

use crate::common::{branching, delivery, labels};
use crate::main_agent_merge::config::Config;
use crate::main_agent_merge::data::audit;
use crate::main_agent_merge::data::report::Outcome as MergeOutcome;
use crate::main_agent_merge::ports::Ports;

/// Attempts to merge one milestone.
///
/// # Errors
/// [`harness_core::domain::Halt`] from any read or write that fails outright
/// — reading the milestone's tasks, reading or writing a PR, labelling the
/// milestone. A PR not existing yet is **not** treated as such a failure:
/// see the inline note on why that can't be told apart from a genuine one
/// with the port as it stands, and why that is an accepted simplification.
pub async fn run(ports: &Ports, config: &Config, milestone: u64) -> Outcome<MergeOutcome> {
    let tasks = ports.gh.sub_issues(milestone).await?;
    if !audit::all_tasks_delivered(&tasks) {
        return Ok(MergeOutcome::NotReady);
    }

    let issue = ports.gh.issue(milestone).await?;
    let branch = branching::milestone_branch(milestone, &issue.title);

    // The label said delivered; this asks git. A task whose PR was opened
    // and never merged — red CI, abandoned — carries the same label as one
    // that shipped, and proposing this branch upward without its code would
    // be the one lie this whole flow exists to avoid.
    let merged_into_milestone = ports.gh.merged_prs(&branch).await?;
    if !delivery::every_task_merged(&tasks, &merged_into_milestone) {
        return Ok(MergeOutcome::NotReady);
    }

    // A failed `pr()` read is read as "no PR yet, open one" rather than
    // propagated — the port cannot currently distinguish "not found" from
    // a genuine failure (both come back `Halt::Failed`). If the real cause
    // is a genuine failure, `create_pr` below fails for the same reason,
    // and that error does propagate.
    if let Ok(pr) = ports.gh.pr(&branch).await {
        if ports.gh.pr_checks_green(&pr.num).await? {
            ports.gh.merge_pr(&pr.num).await?;
            ports.gh.add_label(milestone, labels::WAITING_MERGE).await?;
            Ok(MergeOutcome::Merged)
        } else {
            Ok(MergeOutcome::WaitingOnChecks)
        }
    } else {
        let title = format!("Milestone #{milestone}: {}", issue.title);
        let body = format!("Closes #{milestone}\n\nEvery task of this milestone is delivered.");
        let url = ports
            .gh
            .create_pr(&branch, &config.base_branch, &title, &body)
            .await?;
        Ok(MergeOutcome::OpenedPr(url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use crate::main_agent_merge::config::fake as config_fake;
    use crate::main_agent_merge::ports::fake as ports_fake;
    use harness_core::domain::{Issue, Pr};
    use std::collections::HashMap;
    use std::rc::Rc;

    fn issue(number: u64, state: &str, title: &str, labels: &[&str]) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            title: title.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn task(number: u64, state: &str) -> Issue {
        issue(number, state, "a task", &[])
    }

    /// The merged PRs that prove these tasks' code is on the branch — what
    /// `delivery::every_task_merged` reads, and what a milestone cannot be
    /// proposed upward without.
    fn merged_closing(numbers: &[u64]) -> Vec<Issue> {
        numbers
            .iter()
            .map(|number| Issue {
                number: 900 + number,
                body: format!("Closes #{number}"),
                ..Issue::default()
            })
            .collect()
    }

    #[tokio::test]
    async fn open_tasks_mean_not_ready_and_nothing_read_further() {
        let gh = Rc::new(FakeGitHub {
            subs: vec![(4, vec![task(11, "open")])],
            ..FakeGitHub::default()
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(outcome, MergeOutcome::NotReady);
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn all_closed_and_no_pr_yet_opens_one() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "open", "Territory tooling", &[labels::MILESTONE])],
            subs: vec![(4, vec![task(11, "closed")])],
            merged: merged_closing(&[11]),
            ..FakeGitHub::default()
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert!(matches!(outcome, MergeOutcome::OpenedPr(_)));
        let writes = gh.writes();
        assert!(writes.iter().any(|w| matches!(
            w,
            crate::common::fake_github::Wrote::CreatedPr(head, base, _)
                if head == "milestone/4-territory-tooling" && base == "main_agent"
        )));
    }

    #[tokio::test]
    async fn tasks_delivered_but_still_open_are_ready_to_merge() {
        // The regression this guards: the flow leaves every delivered task
        // open under `harness:waiting-merge`, because a PR merged into a
        // milestone branch closes nothing. Reading `closed` alone meant the
        // milestone PR was never opened, however much work had shipped.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "open", "Territory tooling", &[labels::MILESTONE])],
            subs: vec![(
                4,
                vec![issue(11, "open", "a task", &[labels::WAITING_MERGE])],
            )],
            merged: merged_closing(&[11]),
            ..FakeGitHub::default()
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert!(matches!(outcome, MergeOutcome::OpenedPr(_)), "{outcome:?}");
    }

    #[tokio::test]
    async fn a_task_labelled_delivered_whose_pr_never_merged_blocks_the_milestone() {
        // The label is posed when the PR is *opened*, so it alone cannot be
        // trusted here: a task whose PR went red and was abandoned would
        // otherwise let this branch be proposed upward without its code.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "open", "Territory tooling", &[labels::MILESTONE])],
            subs: vec![(
                4,
                vec![issue(11, "open", "a task", &[labels::WAITING_MERGE])],
            )],
            merged: Vec::new(),
            ..FakeGitHub::default()
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(outcome, MergeOutcome::NotReady);
        assert!(gh.writes().is_empty(), "nothing proposed, nothing written");
    }

    #[tokio::test]
    async fn an_existing_pr_with_red_checks_waits() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "open", "Territory tooling", &[labels::MILESTONE])],
            subs: vec![(4, vec![task(11, "closed")])],
            merged: merged_closing(&[11]),
            prs: vec![(
                "milestone/4-territory-tooling".to_string(),
                Pr {
                    num: "99".to_string(),
                    base: "main_agent".to_string(),
                    head: "milestone/4-territory-tooling".to_string(),
                    ..Pr::default()
                },
            )],
            pr_checks: HashMap::from([("99".to_string(), false)]),
            ..FakeGitHub::default()
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(outcome, MergeOutcome::WaitingOnChecks);
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn an_existing_pr_with_green_checks_is_merged_and_the_milestone_labelled() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "open", "Territory tooling", &[labels::MILESTONE])],
            subs: vec![(4, vec![task(11, "closed")])],
            merged: merged_closing(&[11]),
            prs: vec![(
                "milestone/4-territory-tooling".to_string(),
                Pr {
                    num: "99".to_string(),
                    base: "main_agent".to_string(),
                    head: "milestone/4-territory-tooling".to_string(),
                    ..Pr::default()
                },
            )],
            pr_checks: HashMap::from([("99".to_string(), true)]),
            ..FakeGitHub::default()
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(outcome, MergeOutcome::Merged);
        assert_eq!(
            gh.writes(),
            vec![
                crate::common::fake_github::Wrote::MergedPr("99".to_string()),
                crate::common::fake_github::Wrote::Label(4, labels::WAITING_MERGE.to_string()),
            ]
        );
    }
}
