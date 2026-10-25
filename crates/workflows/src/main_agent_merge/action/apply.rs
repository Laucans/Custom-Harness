//! The one place that sequences a merge attempt: read, decide, write.

use harness_architecture::graph::{self, Graph};
use harness_core::domain::Outcome;

use crate::common::{branching, contest, delivery, labels, review};
use crate::main_agent_merge::config::Config;
use crate::main_agent_merge::data::audit;
use crate::main_agent_merge::data::merge_train;
use crate::main_agent_merge::data::report::Outcome as MergeOutcome;
use crate::main_agent_merge::ports::Ports;

/// Attempts to merge one milestone.
///
/// # Errors
/// [`harness_core::domain::Halt`] from any read or write that fails outright
/// — reading the milestone's tasks, reading or writing a PR, labelling the
/// milestone. The merge itself is the exception: a conflicting PR is
/// [`MergeOutcome::Conflicting`], a mergeability that cannot be read is
/// [`MergeOutcome::MergeabilityUnread`], a merge GitHub refuses is
/// [`MergeOutcome::MergeRefused`] — told, and the watch goes on. A PR not
/// existing yet is **not** treated as such a failure:
/// see the inline note on why that can't be told apart from a genuine one
/// with the port as it stands, and why that is an accepted simplification.
pub async fn run(ports: &Ports, config: &Config, milestone: u64) -> Outcome<MergeOutcome> {
    let tasks = ports.gh.sub_issues(milestone).await?;
    if !audit::all_tasks_delivered(&tasks) {
        return Ok(MergeOutcome::NotReady);
    }

    let issue = ports.gh.issue(milestone).await?;
    let branch = branching::milestone_branch(milestone, &issue.title);
    // The train's order: a consumer lands after the producers of what it
    // reads, whatever its number — those the graph names and those its
    // tasks need, closed tasks included.
    let graph = read_graph(ports, &config.base_branch).await;
    let open = ports.gh.issues_labelled(labels::MILESTONE, "open").await?;
    let waiting = merge_train::landing_waits(graph.as_ref(), milestone, &open, &tasks);
    if !waiting.is_empty() {
        return Ok(MergeOutcome::WaitingOnProducers(waiting));
    }

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
            // Green checks say nothing of a conflict: they ran on an older
            // base, and GitHub refuses the merge of a conflicting PR
            // (#423/#442 halted the watch that way). Asked first; the
            // refusal itself is told, never a halt.
            match ports.gh.pr_mergeable(&pr.num).await {
                Ok(Some(true)) => {}
                // GitHub has not computed it yet: a later tick reads it.
                Ok(None) => return Ok(MergeOutcome::WaitingOnChecks),
                // A failed read is told, not taken for "not computed yet".
                Err(halt) => {
                    return Ok(MergeOutcome::MergeabilityUnread {
                        pr: pr.reference(),
                        why: halt.reason().to_string(),
                    });
                }
                Ok(Some(false)) => {
                    // What landed on the base may resolve it; when GitHub
                    // can merge it in, the push reruns the checks.
                    return match ports.gh.merge_branch(&branch, &config.base_branch).await {
                        Ok(()) => Ok(MergeOutcome::WaitingOnChecks),
                        Err(halt) => conflicting(ports, config, &issue, &pr, halt).await,
                    };
                }
            }
            if let Err(halt) = ports.gh.merge_pr(&pr.num).await {
                return Ok(MergeOutcome::MergeRefused {
                    pr: pr.reference(),
                    why: halt.reason().to_string(),
                });
            }
            ports.gh.add_label(milestone, labels::WAITING_MERGE).await?;
            close_delivered(ports, &tasks, &pr.reference()).await?;
            let built = mark_built(ports, config, graph, milestone, &pr.num).await?;
            // The other milestones still building take the landing through
            // the catch-up route: it moved the integration branch's head,
            // which the watch reads every tick (`common::catch_up`).
            let roadmap = roadmap_landed(ports, milestone).await?;
            Ok(MergeOutcome::Merged {
                pr: pr.reference(),
                roadmap,
                built,
            })
        } else {
            // Checks that ran on an older base do not run again by
            // themselves: a fix landed on the integration branch meanwhile
            // (#17 waited on a rules contest settled there) reaches the
            // milestone, and its push runs them anew. Nothing to take is a
            // no-op; a conflict is left to the PR, where it shows.
            let _ = ports.gh.merge_branch(&branch, &config.base_branch).await;
            // Not green is two things: still running, or failed. The first
            // waits; the second waits too — nothing merges on red — but is
            // told apart, since no later tick turns it green by itself, and
            // is the repair route's: no session owns a milestone's branch,
            // and #17's PR stayed red on a settled contest's CONTEST.md
            // that `pr_fix` knows to remove.
            let checks = ports
                .gh
                .pr_failing_checks(&pr.num)
                .await
                .unwrap_or_default();
            if checks.is_empty() {
                return Ok(MergeOutcome::WaitingOnChecks);
            }
            red(ports, config, &issue, &pr, checks).await
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

/// A red landing PR: its repair asked within the cap (`harness:pr-fix`);
/// past it, a decision asked on the milestone (`milestone_merge::escalate`)
/// — never a third repair asked every tick (`ForgeDesMondes`' #423: 21 to 25
/// repairs). Nothing is escalated while its repair still runs on a lane,
/// nor while it waits on an issue (`common::contest::held`).
async fn red(
    ports: &Ports,
    config: &Config,
    milestone: &harness_core::domain::Issue,
    pr: &harness_core::domain::Pr,
    checks: Vec<String>,
) -> Outcome<MergeOutcome> {
    let comments = ports.gh.pr_comments(&pr.num).await?;
    let number = pr.num.parse::<u64>().ok();
    if review::repairs(&comments) < review::MAX_FIXES {
        if let Some(number) = number {
            ports.gh.add_label(number, labels::PR_FIX).await?;
        }
        return Ok(MergeOutcome::Red {
            pr: pr.reference(),
            checks,
        });
    }
    let asked = past_its_repairs(ports, config, milestone, pr, &comments).await?;
    Ok(MergeOutcome::Escalated {
        pr: pr.reference(),
        checks,
        asked,
    })
}

/// A landing PR whose base cannot be merged in: told conflicting within
/// its repairs — the catch-up labels it for one (`common::catch_up`) — and,
/// past them, a decision asked on the milestone as for a red one: the
/// repair route takes it no more (`pr_fix::data::candidates::due`), and a
/// conflict told every tick asks nobody anything.
async fn conflicting(
    ports: &Ports,
    config: &Config,
    milestone: &harness_core::domain::Issue,
    pr: &harness_core::domain::Pr,
    halt: harness_core::domain::Halt,
) -> Outcome<MergeOutcome> {
    let comments = ports.gh.pr_comments(&pr.num).await?;
    if review::repairs(&comments) < review::MAX_FIXES {
        return Ok(MergeOutcome::Conflicting {
            pr: pr.reference(),
            why: halt.reason().to_string(),
        });
    }
    let asked = past_its_repairs(ports, config, milestone, pr, &comments).await?;
    Ok(MergeOutcome::Escalated {
        pr: pr.reference(),
        checks: vec![format!(
            "conflicts with {}: {}",
            config.base_branch,
            halt.reason()
        )],
        asked,
    })
}

/// Asks the decision on a landing PR past its repairs — never while a lane
/// repairs it, nor while it waits (`common::contest::held`); answers
/// whether it asked now.
async fn past_its_repairs(
    ports: &Ports,
    config: &Config,
    milestone: &harness_core::domain::Issue,
    pr: &harness_core::domain::Pr,
    comments: &str,
) -> Outcome<bool> {
    let repairing = pr
        .num
        .parse::<u64>()
        .is_ok_and(|n| config.repairing.contains(&n));
    Ok(!repairing
        && !contest::held(ports.gh.as_ref(), pr, comments).await?
        && crate::milestone_merge::escalate(ports.gh.as_ref(), milestone, pr, comments).await?)
}

/// The plan's graph on the base branch, when the plan wrote one.
async fn read_graph(ports: &Ports, base: &str) -> Option<Graph> {
    let text = ports.gh.file_text(graph::PATH, base).await.ok().flatten()?;
    Graph::parse(&text).ok()
}

/// Marks `built`, in the plan's graph, every id whose manifest the merged
/// pull request carried — the mock server stops standing in for them — and
/// writes the graph back to the base branch when something changed.
async fn mark_built(
    ports: &Ports,
    config: &Config,
    graph: Option<Graph>,
    milestone: u64,
    pr: &str,
) -> Outcome<Vec<String>> {
    let Some(mut graph) = graph else {
        return Ok(Vec::new());
    };
    let files = ports.gh.pr_files(pr).await.unwrap_or_default();
    let mut manifests = Vec::new();
    for path in files.iter().filter(|path| merge_train::is_manifest(path)) {
        if let Some(text) = ports.gh.file_text(path, &config.base_branch).await?
            && let Some(declared) = merge_train::declared(&text)
        {
            manifests.push(declared);
        }
    }
    let built = merge_train::mark_built(&mut graph, &manifests);
    if built.is_empty() {
        return Ok(built);
    }
    let text = graph.to_json().map_err(|e| {
        harness_core::domain::Halt::Failed(format!("the plan's graph cannot be written: {e}"))
    })?;
    ports
        .gh
        .put_file(
            graph::PATH,
            &config.base_branch,
            &text,
            &format!(
                "chore(plan): milestone #{milestone} built {} — {}",
                built.len(),
                built.join(", ")
            ),
        )
        .await?;
    Ok(built)
}

/// The roadmap item whose milestones have now all landed — this one the
/// last — labelled `harness:waiting-merge` and told so, once: ready to
/// release. A human closes it once released; until then it is no roadmap
/// item to plan (`routing`).
async fn roadmap_landed(ports: &Ports, milestone: u64) -> Outcome<Option<u64>> {
    let open = ports.gh.issues_labelled(labels::ROADMAP, "open").await?;
    for roadmap in open
        .iter()
        .filter(|r| r.is_open() && !r.has(labels::WAITING_MERGE))
    {
        let milestones = ports.gh.sub_issues(roadmap.number).await?;
        if !milestones.iter().any(|m| m.number == milestone) {
            continue;
        }
        let landed = milestones
            .iter()
            .all(|m| m.number == milestone || m.is_closed() || m.has(labels::WAITING_MERGE));
        if !landed {
            return Ok(None);
        }
        ports
            .gh
            .post_issue_comment(
                roadmap.number,
                "Every milestone of this roadmap item has landed on the integration branch: it \
                 is ready to release. Close it once it is released.",
            )
            .await?;
        ports
            .gh
            .add_label(roadmap.number, labels::WAITING_MERGE)
            .await?;
        return Ok(Some(roadmap.number));
    }
    Ok(None)
}

/// Closes every task still open once its milestone is in the integration
/// branch: a PR merged into a milestone branch closes nothing — GitHub's
/// closing keywords fire only on the default branch — so the tasks waited
/// in `waiting-merge` with their code already integrated.
async fn close_delivered(
    ports: &Ports,
    tasks: &[harness_core::domain::Issue],
    pr: &str,
) -> Outcome<()> {
    for task in tasks.iter().filter(|task| task.is_open()) {
        ports
            .gh
            .post_issue_comment(
                task.number,
                &format!(
                    "Delivered: its milestone was merged into the integration branch by PR {pr}."
                ),
            )
            .await?;
        ports
            .gh
            .remove_label(task.number, labels::WAITING_MERGE)
            .await?;
        ports.gh.close_issue(task.number).await?;
    }
    Ok(())
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
        assert_eq!(gh.writes(), [] as [crate::common::fake_github::Wrote; 0]);
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
    async fn an_existing_pr_with_pending_checks_waits() {
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
            failing_checks: HashMap::from([("99".to_string(), Vec::new())]),
            ..FakeGitHub::default()
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(outcome, MergeOutcome::WaitingOnChecks);
        assert_eq!(
            gh.writes(),
            [crate::common::fake_github::Wrote::MergedBranch(
                "milestone/4-territory-tooling".to_string(),
                "main_agent".to_string()
            )],
            "the milestone takes what landed on its base, nothing else"
        );
    }

    #[tokio::test]
    async fn an_existing_pr_with_failed_checks_is_refused_told_red_and_sent_to_repair() {
        // Nothing merges on red: the outcome tells a failed check apart from
        // one still running, and the repair is asked — no session owns a
        // milestone's branch, and #17's PR stayed red on a settled
        // contest's CONTEST.md that `pr_fix` knows to remove.
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
            failing_checks: HashMap::from([(
                "99".to_string(),
                vec!["rule-tests-untouched — FAILURE — https://ci/9".to_string()],
            )]),
            ..FakeGitHub::default()
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(
            outcome,
            MergeOutcome::Red {
                pr: "#99".to_string(),
                checks: vec!["rule-tests-untouched — FAILURE — https://ci/9".to_string()],
            }
        );
        let writes = gh.writes();
        assert!(
            !writes.contains(&crate::common::fake_github::Wrote::MergedPr(
                "99".to_string()
            )),
            "nothing merged"
        );
        assert!(
            !writes.contains(&crate::common::fake_github::Wrote::Label(
                4,
                labels::WAITING_MERGE.to_string()
            )),
            "the milestone is not delivered"
        );
        assert!(
            writes.contains(&crate::common::fake_github::Wrote::Label(
                99,
                labels::PR_FIX.to_string()
            )),
            "{writes:?}"
        );
    }

    fn red_landing(comments: &str) -> FakeGitHub {
        FakeGitHub {
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
            failing_checks: HashMap::from([("99".to_string(), vec!["rules-hold".to_string()])]),
            pr_comment_bodies: vec![("99".to_string(), comments.to_string())],
            ..FakeGitHub::default()
        }
    }

    #[tokio::test]
    async fn red_within_the_cap_is_labelled_past_it_escalated() {
        use crate::common::fake_github::Wrote;
        use crate::common::review;
        let used = review::FIX_MARKER.repeat(review::MAX_FIXES);
        let gh = Rc::new(red_landing(&used));
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(
            outcome,
            MergeOutcome::Escalated {
                pr: "#99".to_string(),
                checks: vec!["rules-hold".to_string()],
                asked: true,
            }
        );
        let writes = gh.writes();
        assert!(
            !writes.contains(&Wrote::Label(99, labels::PR_FIX.to_string())),
            "no third repair: {writes:?}"
        );
        assert!(writes.contains(&Wrote::Label(4, labels::NEEDS_DECISION.to_string())));
        // While its repair still runs on a lane, nothing is asked.
        let gh = Rc::new(red_landing(&used));
        let config = Config {
            repairing: vec![99],
            ..config_fake::config()
        };
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config, 4)
            .await
            .expect("a verdict");
        assert!(matches!(
            outcome,
            MergeOutcome::Escalated { asked: false, .. }
        ));
        assert!(
            !gh.writes()
                .iter()
                .any(|w| matches!(w, Wrote::Label(..) | Wrote::Body(..))),
            "{:?}",
            gh.writes()
        );
        // Nor while it waits on a contract change its repair stopped on.
        let gh = Rc::new(red_landing(&format!(
            "{used}{}",
            crate::common::contest::contract_wait_marker(503)
        )));
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert!(matches!(
            outcome,
            MergeOutcome::Escalated { asked: false, .. }
        ));
    }

    #[tokio::test]
    async fn the_merge_closes_the_tasks_it_integrated() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, "open", "Territory tooling", &[labels::MILESTONE])],
            subs: vec![(
                4,
                vec![issue(11, "open", "a task", &[labels::WAITING_MERGE])],
            )],
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
        assert_eq!(
            outcome,
            MergeOutcome::Merged {
                pr: "#99".to_string(),
                roadmap: None,
                built: Vec::new(),
            }
        );
        let writes = gh.writes();
        assert!(writes.contains(&crate::common::fake_github::Wrote::Closed(11)));
        assert!(
            writes.contains(&crate::common::fake_github::Wrote::Unlabelled(
                11,
                labels::WAITING_MERGE.to_string()
            ))
        );
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
        assert_eq!(
            outcome,
            MergeOutcome::Merged {
                pr: "#99".to_string(),
                roadmap: None,
                built: Vec::new(),
            }
        );
        assert_eq!(
            gh.writes(),
            vec![
                crate::common::fake_github::Wrote::MergedPr("99".to_string()),
                crate::common::fake_github::Wrote::Label(4, labels::WAITING_MERGE.to_string()),
            ]
        );
    }

    /// A delivered milestone whose PR #99 is green, with `tune` applied.
    fn green_pr(tune: impl FnOnce(FakeGitHub) -> FakeGitHub) -> Rc<FakeGitHub> {
        Rc::new(tune(FakeGitHub {
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
        }))
    }

    #[tokio::test]
    async fn a_green_pr_that_conflicts_is_told_conflicting_and_never_merged() {
        // #423/#442: green checks from an older base, a conflict since — the
        // merge GitHub refused halted the watch. Now it is asked first.
        let gh = green_pr(|gh| FakeGitHub {
            pr_mergeable: HashMap::from([("99".to_string(), Some(false))]),
            refused_merges: vec!["milestone/4-territory-tooling".to_string()],
            ..gh
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict, not a halt");
        assert_eq!(
            outcome,
            MergeOutcome::Conflicting {
                pr: "#99".to_string(),
                why: "milestone/4-territory-tooling conflicts with main_agent".to_string(),
            }
        );
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    }

    #[tokio::test]
    async fn a_conflicting_landing_pr_past_its_repairs_asks_its_milestone() {
        use crate::common::fake_github::Wrote;
        let gh = green_pr(|gh| FakeGitHub {
            pr_mergeable: HashMap::from([("99".to_string(), Some(false))]),
            refused_merges: vec!["milestone/4-territory-tooling".to_string()],
            pr_comment_bodies: vec![(
                "99".to_string(),
                review::FIX_MARKER.repeat(review::MAX_FIXES),
            )],
            ..gh
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert!(
            matches!(&outcome, MergeOutcome::Escalated { pr, checks, asked: true }
                if pr == "#99" && checks[0].starts_with("conflicts with main_agent")),
            "{outcome:?}"
        );
        assert!(
            gh.writes()
                .contains(&Wrote::Label(4, labels::NEEDS_DECISION.to_string()))
        );
        // Under repair on a lane: nothing asked.
        let gh = green_pr(|gh| FakeGitHub {
            pr_mergeable: HashMap::from([("99".to_string(), Some(false))]),
            refused_merges: vec!["milestone/4-territory-tooling".to_string()],
            pr_comment_bodies: vec![(
                "99".to_string(),
                review::FIX_MARKER.repeat(review::MAX_FIXES),
            )],
            ..gh
        });
        let config = Config {
            repairing: vec![99],
            ..config_fake::config()
        };
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config, 4)
            .await
            .expect("a verdict");
        assert!(matches!(
            outcome,
            MergeOutcome::Escalated { asked: false, .. }
        ));
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    }

    #[tokio::test]
    async fn a_conflict_the_base_resolves_waits_for_the_checks_its_push_runs() {
        use crate::common::fake_github::Wrote;
        let gh = green_pr(|gh| FakeGitHub {
            pr_mergeable: HashMap::from([("99".to_string(), Some(false))]),
            ..gh
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(outcome, MergeOutcome::WaitingOnChecks);
        assert_eq!(
            gh.writes(),
            [Wrote::MergedBranch(
                "milestone/4-territory-tooling".to_string(),
                "main_agent".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn a_mergeability_that_cannot_be_read_is_told_never_taken_for_not_computed() {
        let gh = green_pr(|gh| FakeGitHub {
            unreadable_mergeability: vec!["99".to_string()],
            ..gh
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict, not a halt");
        assert_eq!(
            outcome,
            MergeOutcome::MergeabilityUnread {
                pr: "#99".to_string(),
                why: "HTTP 502: Bad Gateway".to_string(),
            }
        );
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    }

    #[tokio::test]
    async fn a_mergeability_github_has_not_computed_yet_waits() {
        let gh = green_pr(|gh| FakeGitHub {
            pr_mergeable: HashMap::from([("99".to_string(), None)]),
            ..gh
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(outcome, MergeOutcome::WaitingOnChecks);
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    }

    #[tokio::test]
    async fn a_merge_github_refuses_is_a_warning_not_a_halt() {
        let gh = green_pr(|gh| FakeGitHub {
            refused_pr_merges: vec!["99".to_string()],
            ..gh
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict, not a halt");
        assert_eq!(
            outcome,
            MergeOutcome::MergeRefused {
                pr: "#99".to_string(),
                why: "Pull request #99 is not mergeable".to_string(),
            }
        );
        assert!(
            gh.writes().is_empty(),
            "nothing labelled, nothing closed: {:?}",
            gh.writes()
        );
    }

    #[tokio::test]
    async fn the_last_milestone_of_a_roadmap_item_landing_marks_it_ready_to_release() {
        let milestone =
            |number, state: &str, labels: &[&str]| issue(number, state, "a milestone", labels);
        let gh = Rc::new(FakeGitHub {
            issues: vec![
                milestone(4, "open", &[labels::MILESTONE]),
                issue(2, "open", "The PoC", &[labels::ROADMAP]),
                issue(7, "open", "Elsewhere", &[labels::ROADMAP]),
            ],
            subs: vec![
                (4, vec![task(11, "closed")]),
                (
                    2,
                    vec![
                        milestone(3, "closed", &[labels::MILESTONE]),
                        milestone(4, "open", &[labels::MILESTONE]),
                    ],
                ),
                (7, vec![milestone(8, "open", &[labels::MILESTONE])]),
            ],
            merged: merged_closing(&[11]),
            prs: vec![(
                "milestone/4-a-milestone".to_string(),
                Pr {
                    num: "99".to_string(),
                    base: "main_agent".to_string(),
                    head: "milestone/4-a-milestone".to_string(),
                    ..Pr::default()
                },
            )],
            pr_checks: HashMap::from([("99".to_string(), true)]),
            ..FakeGitHub::default()
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(
            outcome,
            MergeOutcome::Merged {
                pr: "#99".to_string(),
                roadmap: Some(2),
                built: Vec::new(),
            }
        );
        let writes = gh.writes();
        assert!(writes.contains(&crate::common::fake_github::Wrote::Label(
            2,
            labels::WAITING_MERGE.to_string()
        )));
        assert!(
            !writes
                .iter()
                .any(|w| matches!(w, crate::common::fake_github::Wrote::Label(7, _))),
            "a roadmap item with a milestone still to land is not delivered"
        );
    }

    const GRAPH: &str = r#"{"version":1,"milestones":[
      {"number":5,"title":"Sessions","systems":["session"],"produces":["session.listScenes"],"consumes":[],"depends_on":[]},
      {"number":4,"title":"Territory tooling","systems":["territory"],"produces":["territory.listZones"],"consumes":["session.listScenes"],"depends_on":[5]}
    ],"ids":{
      "session.listScenes":{"kind":"capability","producer":5,"state":"mocked","version":1},
      "territory.listZones":{"kind":"capability","producer":4,"state":"planned","version":1}
    }}"#;

    // One scenario told whole — the train's order, then its landing — reads
    // better than two halves sharing a fixture.
    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn the_train_lands_a_consumer_after_its_producer_marks_what_it_built_and_touches_no_other_branch()
     {
        use crate::common::fake_github::Wrote;
        let territory = || {
            issue(
                4,
                "open",
                "Territory tooling",
                &[labels::MILESTONE, labels::TRIGGERED],
            )
        };
        let delivered = |gh: FakeGitHub| FakeGitHub {
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
            pr_files: HashMap::from([(
                "99".to_string(),
                vec![
                    "crates/territory/capabilities/list-zones/capability.json".to_string(),
                    "crates/territory/src/lib.rs".to_string(),
                ],
            )]),
            files: HashMap::from([
                (
                    ("plan/graph.json".to_string(), "main_agent".to_string()),
                    GRAPH.to_string(),
                ),
                (
                    (
                        "crates/territory/capabilities/list-zones/capability.json".to_string(),
                        "main_agent".to_string(),
                    ),
                    r#"{"capability":"territory.listZones","system":"territory"}"#.to_string(),
                ),
            ]),
            ..gh
        };
        // The producer still builds: the consumer waits, whatever its number.
        let gh = Rc::new(delivered(FakeGitHub {
            issues: vec![
                territory(),
                issue(
                    5,
                    "open",
                    "Sessions",
                    &[labels::MILESTONE, labels::TRIGGERED],
                ),
            ],
            ..FakeGitHub::default()
        }));
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(outcome, MergeOutcome::WaitingOnProducers(vec![5]));
        assert!(gh.writes().is_empty(), "gh.writes() should be empty");
        // The producer landed: the consumer lands, its id is marked built;
        // the branches still building are the catch-up route's, not this
        // landing's.
        let gh = Rc::new(delivered(FakeGitHub {
            issues: vec![
                territory(),
                issue(
                    5,
                    "open",
                    "Sessions",
                    &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
                ),
                issue(6, "open", "Media", &[labels::MILESTONE, labels::TRIGGERED]),
                issue(7, "open", "Sound", &[labels::MILESTONE, labels::TRIGGERED]),
                issue(8, "open", "Later", &[labels::MILESTONE, labels::READY]),
            ],
            refused_merges: vec!["milestone/7-sound".to_string()],
            ..FakeGitHub::default()
        }));
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(
            outcome,
            MergeOutcome::Merged {
                pr: "#99".to_string(),
                roadmap: None,
                built: vec!["territory.listZones".to_string()],
            }
        );
        let writes = gh.writes();
        let graph_written = writes.iter().find_map(|w| match w {
            Wrote::PutFile(path, branch, text) if path == "plan/graph.json" => {
                Some((branch.clone(), text.clone()))
            }
            _ => None,
        });
        let (branch, text) = graph_written.expect("the graph is written back");
        assert_eq!(branch, "main_agent");
        let graph = Graph::parse(&text).expect("a graph");
        assert_eq!(
            graph.ids["territory.listZones"].state,
            harness_architecture::graph::State::Built
        );
        assert!(
            !writes
                .iter()
                .any(|w| matches!(w, Wrote::MergedBranch(..))
                    || matches!(w, Wrote::Comment(6..=8, _))),
            "a landing touches no other branch: {writes:?}"
        );
    }

    #[tokio::test]
    async fn a_landing_waits_on_the_producer_of_what_a_closed_task_needed_with_no_graph_edge() {
        // #4 depends on #5 alone in the graph; its closed task #11 laid out
        // #3's Micro-UI — the edge the plan forgot (#28 on #24).
        const FORGOT: &str = r#"{"version":1,"milestones":[
          {"number":3,"title":"Brief","systems":["session"],"produces":["OralBrief"],"consumes":[],"depends_on":[]},
          {"number":5,"title":"Sessions","systems":["session"],"produces":["session.listScenes"],"consumes":[],"depends_on":[]},
          {"number":4,"title":"Territory tooling","systems":["territory"],"produces":[],"consumes":["session.listScenes"],"depends_on":[5]}
        ],"ids":{
          "OralBrief":{"kind":"micro-ui","producer":3,"state":"planned","version":1},
          "session.listScenes":{"kind":"capability","producer":5,"state":"mocked","version":1}
        }}"#;
        let screen = Issue {
            body: "## Architecture\n\nunit: composition\nsystem: territory\nneeds: OralBrief"
                .to_string(),
            ..task(11, "closed")
        };
        let gh = Rc::new(FakeGitHub {
            subs: vec![(4, vec![screen])],
            issues: vec![
                issue(
                    4,
                    "open",
                    "Territory tooling",
                    &[labels::MILESTONE, labels::TRIGGERED],
                ),
                issue(3, "open", "Brief", &[labels::MILESTONE, labels::TRIGGERED]),
                issue(
                    5,
                    "open",
                    "Sessions",
                    &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
                ),
            ],
            files: HashMap::from([(
                ("plan/graph.json".to_string(), "main_agent".to_string()),
                FORGOT.to_string(),
            )]),
            ..FakeGitHub::default()
        });
        let outcome = run(&ports_fake::with(Rc::clone(&gh)), &config_fake::config(), 4)
            .await
            .expect("a verdict");
        assert_eq!(outcome, MergeOutcome::WaitingOnProducers(vec![3]));
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    }
}
