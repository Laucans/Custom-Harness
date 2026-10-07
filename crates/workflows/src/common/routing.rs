//! Which workflow runs next, decided from a snapshot already read.
//!
//! Pure: no `gh` call lives here. The router (the launcher's polling loop)
//! does the reads this needs — including the facts that require more than
//! a flat issue list (`lowest_roadmap_has_milestone`, `ready_to_merge`,
//! `dev_loop_milestone`) — and hands this function the result. That is
//! what makes the decision testable without a fake adapter.
//!
//! Priority, top to bottom: a roadmap item that's `harness:ready` and has no
//! milestone yet outranks everything, because nothing else can be planned
//! against it; a milestone ready to split comes next; then the two things
//! that can be wrong with a pull request already in flight — a red one to
//! repair before a green one to review, since reviewing a change whose CI
//! is broken reviews something about to change; then finishing a milestone
//! that's done, a pending refinement (business, then technical), and finally grinding on a task.
//! Structural decomposition before unblocking what's in flight before
//! closing out before polish before grind.
//!
//! `harness:ready` is the same human gate at every issue level: it means
//! "plan this" on a roadmap item, "split this" on a milestone, and "build
//! this" on a task. `harness:to-review` and `harness:pr-fix` are the same
//! kind of gate on a **pull request**. The router never acts on something
//! nobody has opened the tap on.

use harness_core::domain::{Issue, Pr};

use crate::common::labels;

/// What the router decides to run next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Open the milestones of this roadmap item — `planner`.
    Planner {
        /// The roadmap issue's number.
        roadmap: u64,
    },
    /// Open the tasks of this milestone — `split`.
    Split {
        /// The milestone issue's number.
        milestone: u64,
    },
    /// Attempt one repair on this red PR — `pr_fix`.
    PrFix {
        /// The PR, as the ports name one.
        pr: String,
    },
    /// Review this PR — `pr_review`.
    PrReview {
        /// The PR, as the ports name one.
        pr: String,
        /// The branch it targets. Carried so the review judges its own skip
        /// rules against the branch this PR actually has, rather than
        /// against a fixed integration branch a task PR never targets.
        base: String,
    },
    /// Open or merge this milestone's PR into the base branch —
    /// `milestone_merge`.
    MergeMilestone {
        /// The milestone issue's number.
        milestone: u64,
    },
    /// Rewrite this issue's body — `refinement`.
    Refinement {
        /// The issue's number.
        issue: u64,
    },
    /// Write the technical sections of this issue's body — `refinement`,
    /// technical phase.
    TechRefinement {
        /// The issue's number.
        issue: u64,
    },
    /// Run the dev loop on this milestone's board.
    DevLoop {
        /// The milestone issue's number — so the dispatcher can derive the
        /// branch the dev loop works on (`common::branching`), rather than
        /// a fixed integration branch.
        milestone: u64,
    },
    /// Nothing is ready to run.
    Nothing,
}

/// Everything [`decide`] needs, already read.
pub struct Snapshot {
    /// Open `harness:roadmap` issues, any order.
    pub roadmap: Vec<Issue>,
    /// Whether the lowest-numbered open roadmap issue already has at least
    /// one milestone under it. Meaningless, and never read, if `roadmap`
    /// is empty or that issue isn't `harness:ready` yet.
    pub lowest_roadmap_has_milestone: bool,
    /// Open `harness:milestone` issues, any order.
    pub milestones: Vec<Issue>,
    /// The open PR carrying `harness:pr-fix` whose CI has actually broken,
    /// if any — the router pairs the label with a failing-check read, which
    /// a label list cannot answer alone.
    pub pr_to_fix: Option<Pr>,
    /// The open PR carrying `harness:to-review` that the review would not
    /// skip, if any — the router applies the review's own skip rules
    /// (`pr_review::data::skip_rules`) so a PR that would skip never costs a
    /// mounted checkout per poll.
    pub pr_to_review: Option<Pr>,
    /// The lowest-numbered open milestone whose tasks are all closed and
    /// that isn't `harness:waiting-merge` yet, if any — computed by the
    /// router against each milestone's own sub-issues, which a flat list
    /// cannot answer alone.
    pub ready_to_merge: Option<u64>,
    /// Open `harness:refinement` issues, any order.
    pub refining: Vec<Issue>,
    /// Open `harness:tech-refinement` issues, any order.
    pub tech_refining: Vec<Issue>,
    /// The current milestone, if its board has a runnable task right now —
    /// `None` either because there's no open milestone, or because the one
    /// there is has nothing runnable.
    pub dev_loop_milestone: Option<u64>,
}

/// Decides the next route from a snapshot.
#[must_use]
pub fn decide(snapshot: &Snapshot) -> Route {
    if let Some(lowest) = lowest_open(&snapshot.roadmap)
        && lowest.has(labels::READY)
        && !snapshot.lowest_roadmap_has_milestone
    {
        return Route::Planner {
            roadmap: lowest.number,
        };
    }
    if let Some(ready) = snapshot
        .milestones
        .iter()
        .filter(|issue| issue.is_open() && issue.has(labels::READY))
        .min_by_key(|issue| issue.number)
    {
        return Route::Split {
            milestone: ready.number,
        };
    }
    if let Some(pr) = &snapshot.pr_to_fix {
        return Route::PrFix { pr: pr.num.clone() };
    }
    if let Some(pr) = &snapshot.pr_to_review {
        return Route::PrReview {
            pr: pr.num.clone(),
            base: pr.base.clone(),
        };
    }
    if let Some(milestone) = snapshot.ready_to_merge {
        return Route::MergeMilestone { milestone };
    }
    if let Some(issue) = lowest_open(&snapshot.refining) {
        return Route::Refinement {
            issue: issue.number,
        };
    }
    if let Some(issue) = lowest_open(&snapshot.tech_refining) {
        return Route::TechRefinement {
            issue: issue.number,
        };
    }
    if let Some(milestone) = snapshot.dev_loop_milestone {
        return Route::DevLoop { milestone };
    }
    Route::Nothing
}

/// The lowest-numbered open issue, if any.
fn lowest_open(issues: &[Issue]) -> Option<&Issue> {
    issues
        .iter()
        .filter(|issue| issue.is_open())
        .min_by_key(|issue| issue.number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn empty() -> Snapshot {
        Snapshot {
            roadmap: Vec::new(),
            lowest_roadmap_has_milestone: false,
            milestones: Vec::new(),
            pr_to_fix: None,
            pr_to_review: None,
            ready_to_merge: None,
            refining: Vec::new(),
            tech_refining: Vec::new(),
            dev_loop_milestone: None,
        }
    }

    fn pr(num: &str, base: &str) -> Pr {
        Pr {
            num: num.to_string(),
            base: base.to_string(),
            ..Pr::default()
        }
    }

    #[test]
    fn a_ready_roadmap_item_with_no_milestone_outranks_everything() {
        let snapshot = Snapshot {
            roadmap: vec![
                issue(9, &[labels::ROADMAP, labels::READY]),
                issue(4, &[labels::ROADMAP, labels::READY]),
            ],
            lowest_roadmap_has_milestone: false,
            milestones: vec![issue(5, &[labels::MILESTONE, labels::READY])],
            pr_to_fix: Some(pr("32", "main_agent")),
            pr_to_review: Some(pr("33", "main_agent")),
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            tech_refining: vec![],
            dev_loop_milestone: Some(3),
        };
        assert_eq!(decide(&snapshot), Route::Planner { roadmap: 4 });
    }

    #[test]
    fn a_roadmap_item_that_already_has_a_milestone_is_not_replanned() {
        let snapshot = Snapshot {
            roadmap: vec![issue(4, &[labels::ROADMAP, labels::READY])],
            lowest_roadmap_has_milestone: true,
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Nothing);
    }

    #[test]
    fn a_roadmap_item_without_ready_is_not_planned() {
        let snapshot = Snapshot {
            roadmap: vec![issue(4, &[labels::ROADMAP])],
            lowest_roadmap_has_milestone: false,
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Nothing);
    }

    #[test]
    fn a_ready_milestone_outranks_merging_refinement_and_the_dev_loop() {
        let snapshot = Snapshot {
            milestones: vec![issue(9, &[labels::MILESTONE, labels::READY])],
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestone: Some(3),
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Split { milestone: 9 });
    }

    #[test]
    fn a_milestone_ready_to_merge_outranks_refinement_and_the_dev_loop() {
        let snapshot = Snapshot {
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestone: Some(3),
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::MergeMilestone { milestone: 7 });
    }

    #[test]
    fn a_red_pr_outranks_reviewing_merging_refinement_and_the_dev_loop() {
        // What is already in flight and broken comes before anything new:
        // the milestone it belongs to cannot close over a red PR anyway.
        let snapshot = Snapshot {
            pr_to_fix: Some(pr("32", "milestone/4-territory")),
            pr_to_review: Some(pr("33", "milestone/4-territory")),
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestone: Some(3),
            ..empty()
        };
        assert_eq!(
            decide(&snapshot),
            Route::PrFix {
                pr: "32".to_string()
            }
        );
    }

    #[test]
    fn a_pr_to_review_outranks_merging_refinement_and_the_dev_loop() {
        let snapshot = Snapshot {
            pr_to_review: Some(pr("33", "milestone/4-territory")),
            ready_to_merge: Some(7),
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestone: Some(3),
            ..empty()
        };
        assert_eq!(
            decide(&snapshot),
            Route::PrReview {
                pr: "33".to_string(),
                base: "milestone/4-territory".to_string(),
            }
        );
    }

    #[test]
    fn the_review_carries_the_branch_the_pr_actually_targets() {
        // A task PR targets its milestone's branch, never the integration
        // branch — a review judging against a fixed base would skip it as
        // "targets the wrong branch".
        let snapshot = Snapshot {
            pr_to_review: Some(pr("33", "milestone/9-cities")),
            ..empty()
        };
        let Route::PrReview { base, .. } = decide(&snapshot) else {
            panic!("a review");
        };
        assert_eq!(base, "milestone/9-cities");
    }

    #[test]
    fn a_roadmap_item_to_plan_still_outranks_a_broken_pr() {
        // Decomposition stays at the top: without a plan there is no task
        // whose PR could be broken in the first place.
        let snapshot = Snapshot {
            roadmap: vec![issue(4, &[labels::ROADMAP, labels::READY])],
            pr_to_fix: Some(pr("32", "main_agent")),
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Planner { roadmap: 4 });
    }

    #[test]
    fn a_milestone_without_ready_is_not_split() {
        let snapshot = Snapshot {
            milestones: vec![issue(9, &[labels::MILESTONE])],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Nothing);
    }

    #[test]
    fn a_closed_ready_milestone_is_not_split_again() {
        let mut closed = issue(9, &[labels::MILESTONE, labels::READY]);
        closed.state = "closed".to_string();
        let snapshot = Snapshot {
            milestones: vec![closed],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Nothing);
    }

    #[test]
    fn pending_refinement_outranks_the_dev_loop() {
        let snapshot = Snapshot {
            refining: vec![issue(6, &[labels::REFINEMENT])],
            dev_loop_milestone: Some(3),
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Refinement { issue: 6 });
    }

    #[test]
    fn the_lowest_numbered_refinement_issue_goes_first() {
        let snapshot = Snapshot {
            refining: vec![
                issue(9, &[labels::REFINEMENT]),
                issue(6, &[labels::REFINEMENT]),
            ],
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Refinement { issue: 6 });
    }

    #[test]
    fn business_refinement_goes_before_technical_and_both_before_the_dev_loop() {
        let snapshot = Snapshot {
            refining: vec![issue(9, &[labels::REFINEMENT])],
            tech_refining: vec![issue(6, &[labels::TECH_REFINEMENT])],
            dev_loop_milestone: Some(3),
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::Refinement { issue: 9 });
        let snapshot = Snapshot {
            refining: Vec::new(),
            ..snapshot
        };
        assert_eq!(decide(&snapshot), Route::TechRefinement { issue: 6 });
    }

    #[test]
    fn a_runnable_task_runs_the_dev_loop_when_nothing_outranks_it() {
        let snapshot = Snapshot {
            dev_loop_milestone: Some(3),
            ..empty()
        };
        assert_eq!(decide(&snapshot), Route::DevLoop { milestone: 3 });
    }

    #[test]
    fn an_entirely_quiet_repo_routes_to_nothing() {
        assert_eq!(decide(&empty()), Route::Nothing);
    }
}
