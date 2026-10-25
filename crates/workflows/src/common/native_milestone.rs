//! GitHub's native Milestone field on every task, kept true by the harness.
//!
//! A `harness:milestone` issue holds its tasks as sub-issues: that link is
//! the harness's own truth, the one every route reads. GitHub's native
//! Milestone field is for the humans and GitHub's own views (the issue
//! list, the milestone page): each milestone issue is mirrored by a native
//! milestone — titled `#<n> <title>`, found again by the marker
//! `<!-- harness-milestone: #<n> -->` in its description
//! ([`GitHub::ensure_native_milestone`]), open while the issue is open and
//! not landed ([`native_open`]) — and each task opened under the issue is
//! put on it, right after its sub-issue link. The same convention as the backfill run on `ForgeDesMondes`
//! (`harness-dnd2-run/tools/native-milestones.py`).
//!
//! **A refusal never fails the opening**: every function here answers the
//! warning to journal instead of a [`Halt`](harness_core::domain::Halt), and
//! the task stays opened and linked.

use harness_core::domain::Issue;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Logbook;

use crate::common::labels;

/// Whether the native milestone of `milestone` reads open: its issue open
/// and not landed. A landing merges into the integration branch, where
/// `Closes #<n>` does not fire, so the issue stays open under
/// `harness:waiting-merge` until promotion while its native already reads
/// closed; every later re-sync keeps it closed.
fn native_open(milestone: &Issue) -> bool {
    milestone.is_open() && !milestone.has(labels::WAITING_MERGE)
}

/// The native milestone mirroring `milestone`, made to exist and to share
/// its state ([`native_open`]): its number.
///
/// # Errors
/// The warning to journal when GitHub cannot list, create or turn it.
pub async fn ensure(gh: &dyn GitHub, milestone: &Issue) -> Result<u64, String> {
    gh.ensure_native_milestone(milestone.number, &milestone.title, native_open(milestone))
        .await
        .map_err(|halt| {
            format!(
                "#{}'s native milestone could not be kept: {} — its tasks stay its \
                 sub-issues, without GitHub's Milestone field",
                milestone.number,
                halt.reason()
            )
        })
}

/// Puts `task` on the native milestone `native`, the mirror of the
/// milestone issue `milestone`.
///
/// # Errors
/// The warning to journal when GitHub refuses.
pub async fn assign(gh: &dyn GitHub, native: u64, milestone: u64, task: u64) -> Result<(), String> {
    gh.set_issue_milestone(task, native).await.map_err(|halt| {
        format!(
            "#{task} is a sub-issue of #{milestone} but not on its native milestone: {}",
            halt.reason()
        )
    })
}

/// Puts `task` on the native milestone of the milestone issue `milestone`,
/// read for its title and state, the native made to exist first.
///
/// # Errors
/// The warning to journal when the issue cannot be read or GitHub refuses.
pub async fn place(gh: &dyn GitHub, milestone: u64, task: u64) -> Result<(), String> {
    let issue = gh.issue(milestone).await.map_err(|halt| {
        format!(
            "#{task} is a sub-issue of #{milestone} but not on its native milestone: #{milestone} \
             could not be read ({})",
            halt.reason()
        )
    })?;
    let native = ensure(gh, &issue).await?;
    assign(gh, native, milestone, task).await
}

/// [`place`], its warning journalled.
pub async fn place_or_warn(gh: &dyn GitHub, milestone: u64, task: u64, log: &Logbook) {
    if let Err(why) = place(gh, milestone, task).await {
        log.warn(&why);
    }
}

/// Closes the native milestone of `milestone` — its issue closed, or
/// landed (open under `harness:waiting-merge` until promotion).
///
/// # Errors
/// The warning to journal when GitHub cannot list or close it.
pub async fn close(gh: &dyn GitHub, milestone: &Issue) -> Result<(), String> {
    let closed = Issue {
        state: "closed".to_string(),
        ..milestone.clone()
    };
    ensure(gh, &closed).await.map(|_| ())
}

/// Closes the native milestone of `issue` when it is a milestone issue the
/// harness just closed (a decision's cut, a landing merged on a decision),
/// its refusal journalled; nothing for any other issue.
pub async fn close_if_milestone(gh: &dyn GitHub, issue: &Issue, log: &Logbook) {
    if !issue.has(labels::MILESTONE) {
        return;
    }
    if let Err(why) = close(gh, issue).await {
        log.warn(&why);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Halt;
    use harness_core::traces::{Sink, Verbosity};

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    fn milestone(number: u64, state: &str) -> Issue {
        Issue {
            number,
            title: "Media".to_string(),
            state: state.to_string(),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_task_is_placed_on_its_milestone_s_mirror_in_the_issue_s_state() {
        let gh = FakeGitHub {
            issues: vec![milestone(28, "open"), milestone(9, "closed")],
            ..FakeGitHub::default()
        };
        place(&gh, 28, 30).await.expect("placed");
        place(&gh, 28, 31).await.expect("placed");
        place(&gh, 9, 32).await.expect("placed");
        assert_eq!(
            gh.writes(),
            vec![
                Wrote::NativeMilestone(28, "Media".to_string(), true),
                Wrote::IssueMilestone(30, 1),
                Wrote::NativeMilestone(28, "Media".to_string(), true),
                Wrote::IssueMilestone(31, 1),
                Wrote::NativeMilestone(9, "Media".to_string(), false),
                Wrote::IssueMilestone(32, 2),
            ]
        );
    }

    #[tokio::test]
    async fn a_task_opened_under_a_landed_milestone_keeps_its_mirror_closed() {
        let landed = Issue {
            labels: vec![
                labels::MILESTONE.to_string(),
                labels::WAITING_MERGE.to_string(),
            ],
            ..milestone(28, "open")
        };
        let gh = FakeGitHub {
            issues: vec![landed],
            ..FakeGitHub::default()
        };
        place(&gh, 28, 30).await.expect("placed");
        assert_eq!(
            gh.writes(),
            vec![
                Wrote::NativeMilestone(28, "Media".to_string(), false),
                Wrote::IssueMilestone(30, 1),
            ]
        );
    }

    #[tokio::test]
    async fn a_refusal_is_a_warning_naming_the_task_and_its_milestone() {
        let refusing = FakeGitHub {
            issues: vec![milestone(28, "open")],
            natives_broken: Some(harness_core::domain::Halt::Halted(
                "Resource not accessible by integration".to_string(),
            )),
            ..FakeGitHub::default()
        };
        let why = place(&refusing, 28, 30).await.expect_err("refused");
        assert!(
            why.starts_with("#28's native milestone could not be kept: "),
            "{why}"
        );
        assert!(why.contains("Resource not accessible"), "{why}");
        let why = assign(&refusing, 1, 28, 30).await.expect_err("refused");
        assert!(
            why.starts_with("#30 is a sub-issue of #28 but not on its native milestone: "),
            "{why}"
        );
        assert_eq!(refusing.writes(), Vec::new());
        // The milestone unreadable: told, nothing written.
        let unreadable = FakeGitHub {
            broken: Some(Halt::Unreadable("HTTP 401".to_string())),
            ..FakeGitHub::default()
        };
        let why = place(&unreadable, 28, 30).await.expect_err("unread");
        assert!(why.contains("#28 could not be read (HTTP 401"), "{why}");
        // Journalled as a warning, never a halt.
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        place_or_warn(&refusing, 28, 30, &log).await;
        let said = capture.0.borrow().clone();
        assert_eq!(said.len(), 1);
        assert!(
            said[0].contains("warning: #28's native milestone"),
            "{said:?}"
        );
        let quiet = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&quiet) as Rc<dyn Sink>, Verbosity::Normal);
        let gh = FakeGitHub {
            issues: vec![milestone(28, "open")],
            ..FakeGitHub::default()
        };
        place_or_warn(&gh, 28, 30, &log).await;
        assert_eq!(quiet.0.borrow().clone(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_closed_milestone_closes_its_mirror_whatever_the_issue_still_says() {
        let gh = FakeGitHub::default();
        close(&gh, &milestone(28, "open")).await.expect("closed");
        assert_eq!(
            gh.writes(),
            vec![Wrote::NativeMilestone(28, "Media".to_string(), false)]
        );
        let refusing = FakeGitHub {
            natives_broken: Some(harness_core::domain::Halt::Halted(
                "Resource not accessible by integration".to_string(),
            )),
            ..FakeGitHub::default()
        };
        assert!(close(&refusing, &milestone(28, "open")).await.is_err());
    }

    #[tokio::test]
    async fn only_a_milestone_issue_closes_a_mirror() {
        let gh = FakeGitHub::default();
        let task = milestone(30, "closed");
        close_if_milestone(&gh, &task, &Logbook::null()).await;
        assert_eq!(gh.writes(), Vec::new(), "a task has no mirror");
        let cut = Issue {
            labels: vec![labels::MILESTONE.to_string()],
            ..milestone(28, "open")
        };
        close_if_milestone(&gh, &cut, &Logbook::null()).await;
        assert_eq!(
            gh.writes(),
            vec![Wrote::NativeMilestone(28, "Media".to_string(), false)]
        );
        let refusing = FakeGitHub {
            natives_broken: Some(harness_core::domain::Halt::Halted(
                "Resource not accessible by integration".to_string(),
            )),
            ..FakeGitHub::default()
        };
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        close_if_milestone(&refusing, &cut, &log).await;
        let said = capture.0.borrow().clone();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("warning: #28's native milestone"),
            "{said:?}"
        );
    }
}
