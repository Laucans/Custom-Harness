//! The integration branch's own CI, read by `harness watch` on every full
//! read of the board, before the snapshot: a red base holds what would
//! build on it or merge into it, and says so once on the board.
//!
//! Why: the integration branch (`main_agent`) is not protected and takes
//! direct pushes, so its CI goes red with no pull request to carry the
//! red. Before this, the watch kept starting tasks whose milestone branch
//! takes the integration branch first, and opening PRs that inherited the
//! red — for two hours, told to nobody (`ForgeDesMondes`, 2026-10-18).
//!
//! Once per full read, not once per tick: a quiet tick (`routing::may_skip`)
//! reads nothing and starts nothing, so the hold loses nothing by waiting
//! for the valve (`routing::FULL_READ_EVERY`); only the issue and the line
//! lag by as much. A red base then keeps the full reads going every tick,
//! like a pull request under CI.
//!
//! What it writes is one `harness:human` issue titled [`issue_title`],
//! found again by that title among the open ones so neither the next tick
//! nor a restart opens a second. What it decides is handed to the router as
//! [`Snapshot::base_checks`](crate::common::routing::Snapshot::base_checks):
//! the hold itself is [`routing::base_red`](crate::common::routing::base_red),
//! pure. **The hold lasts until green**: while that issue is open, runs
//! still in progress on a newer head — the fix pushed after the red — hold
//! the plant as the red did, so the unverified base spreads into no
//! milestone branch before its checks pass. Repairing the base is a
//! human's: the issue says what to look at.

use harness_core::domain::{Issue, Outcome};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook};

use crate::common::labels;

/// The title of the issue a red integration branch opens — exactly this,
/// since it is what finds the issue again.
#[must_use]
pub fn issue_title(branch: &str) -> String {
    format!("Integration branch {branch} is red")
}

/// The issue a red `branch` opened, among the open `harness:human` issues,
/// if it is still open.
#[must_use]
pub fn open_issue<'a>(human: &'a [Issue], branch: &str) -> Option<&'a Issue> {
    let title = issue_title(branch);
    human
        .iter()
        .find(|issue| issue.is_open() && issue.title.trim() == title)
}

/// The journal line a red base writes every tick it holds the plant.
#[must_use]
pub fn held_line(branch: &str) -> String {
    format!("integration branch {branch} is red: development and merges wait")
}

/// The journal line a base not green again yet after a red writes every
/// tick it still holds the plant.
#[must_use]
pub fn pending_line(branch: &str) -> String {
    format!(
        "integration branch {branch} is not green again yet: its checks run — development \
         and merges wait"
    )
}

/// Reads the integration branch's checks and keeps the board in step.
///
/// Red opens the issue once and says so each tick; green closes it, once; a
/// check still running, or none yet, changes nothing — the hold stays while
/// the red's issue is open, and lifts when none is. Answers what the router
/// holds on, for its snapshot: `Some(false)` is the hold, whether the base
/// is red or not green again yet. Under `dry_run` nothing is written: the
/// reads and the journal line only.
///
/// # Errors
/// [`harness_core::domain::Halt`] from the checks read, the issue list or
/// the issue writes — a base whose checks cannot be read stops the tick, as
/// any routing read that fails does. The head sha is read leniently: an
/// issue without it still opens.
pub async fn watch(
    gh: &dyn GitHub,
    branch: &str,
    dry_run: bool,
    log: &Logbook,
) -> Outcome<Option<bool>> {
    let checks = gh.branch_checks(branch).await?;
    match checks {
        Some(false) => {
            log.say(&held_line(branch));
            if dry_run {
                return Ok(checks);
            }
            let human = gh.issues_labelled(labels::HUMAN, "open").await?;
            if open_issue(&human, branch).is_none() {
                let sha = gh.branch_sha(branch).await.ok().flatten();
                gh.create_issue(
                    &issue_title(branch),
                    &body(branch, sha.as_deref()),
                    &[labels::HUMAN],
                )
                .await?;
                log.event(&Event::BaseRed {
                    branch: branch.to_string(),
                    sha,
                });
            }
        }
        Some(true) => {
            if dry_run {
                return Ok(checks);
            }
            let human = gh.issues_labelled(labels::HUMAN, "open").await?;
            if let Some(issue) = open_issue(&human, branch) {
                gh.post_issue_comment(
                    issue.number,
                    &format!(
                        "Integration branch `{branch}` is green again: development and merges \
                         resume. Closed by the harness."
                    ),
                )
                .await?;
                gh.close_issue(issue.number).await?;
                log.event(&Event::BaseGreen {
                    branch: branch.to_string(),
                });
            }
        }
        None => {
            // Runs in progress, or none yet. After a red, that is the fix's
            // push under CI: held until it passes, or the unverified base
            // would spread into every milestone branch a task takes it
            // into meanwhile. Only the open issue says a red came before;
            // a human who closed it by hand lifted the hold.
            let human = gh.issues_labelled(labels::HUMAN, "open").await?;
            if open_issue(&human, branch).is_some() {
                log.say(&pending_line(branch));
                return Ok(Some(false));
            }
        }
    }
    Ok(checks)
}

/// The issue's body: the head sha, what the harness holds, what to look at.
fn body(branch: &str, sha: Option<&str>) -> String {
    let at = sha.map_or_else(String::new, |sha| format!(" (`{sha}`)"));
    format!(
        "A workflow run on the newest head of the integration branch `{branch}`{at} concluded \
         in failure.\n\n\
         Until the branch is green again, the harness:\n\
         - starts no task — a task's milestone branch takes `{branch}` first, and would build \
         on the red;\n\
         - merges nothing into a milestone branch nor into `{branch}`;\n\
         - keeps reviewing and repairing pull requests, writing rules, paying test debt and \
         taking decisions.\n\n\
         What to look at:\n\
         - the branch's last workflow runs: `gh run list --branch {branch}`;\n\
         - the last push on `{branch}`: a direct push, or a milestone pull request merged on \
         red;\n\
         - fix it on `{branch}`, or revert that push.\n\n\
         The harness closes this issue once the branch is green."
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use harness_core::traces::{Sink, Verbosity};

    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};

    /// What the journal was told: the lines, and the events as data.
    #[derive(Default)]
    struct Capture {
        lines: RefCell<Vec<String>>,
        events: RefCell<Vec<Event>>,
    }

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.lines.borrow_mut().push(line.to_string());
        }

        fn record(&self, event: &Event) {
            self.events.borrow_mut().push(event.clone());
        }
    }

    fn capture() -> (Rc<Capture>, Logbook) {
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        (capture, log)
    }

    fn human_issue(number: u64, title: &str, state: &str) -> Issue {
        Issue {
            number,
            title: title.to_string(),
            state: state.to_string(),
            labels: vec![labels::HUMAN.to_string()],
            ..Issue::default()
        }
    }

    fn board(checks: Option<bool>, issues: Vec<Issue>) -> FakeGitHub {
        FakeGitHub {
            issues,
            branch_checks: HashMap::from([("main_agent".to_string(), checks)]),
            branch_shas: HashMap::from([("main_agent".to_string(), Some("abc1234".to_string()))]),
            ..FakeGitHub::default()
        }
    }

    #[tokio::test]
    async fn a_red_base_opens_one_human_issue_and_says_so_every_tick() {
        let gh = board(Some(false), Vec::new());
        let (journal, log) = capture();
        let read = watch(&gh, "main_agent", false, &log).await.expect("read");
        assert_eq!(read, Some(false));
        assert_eq!(
            gh.writes(),
            vec![Wrote::CreatedIssue(
                "Integration branch main_agent is red".to_string(),
                body("main_agent", Some("abc1234")),
                vec![labels::HUMAN.to_string()],
            )]
        );
        let told = journal.lines.borrow().clone();
        assert!(
            told.iter().any(|line| line == &held_line("main_agent")),
            "{told:?}"
        );
        // The line the brief asks for, by its letter: the helper is what
        // every other assertion compares to.
        assert_eq!(
            held_line("main_agent"),
            "integration branch main_agent is red: development and merges wait"
        );
        assert_eq!(
            journal.events.borrow().clone(),
            vec![Event::BaseRed {
                branch: "main_agent".to_string(),
                sha: Some("abc1234".to_string()),
            }]
        );
        let body_written = match &gh.writes()[0] {
            Wrote::CreatedIssue(_, body, _) => body.clone(),
            other => panic!("{other:?}"),
        };
        assert!(body_written.contains("abc1234") && body_written.contains("gh run list"));
        // The next tick, or a restart: the issue is found by its title and
        // not opened again — the line is said again, the event is not.
        let again = board(
            Some(false),
            vec![human_issue(
                7,
                "Integration branch main_agent is red",
                "open",
            )],
        );
        let (journal, log) = capture();
        watch(&again, "main_agent", false, &log)
            .await
            .expect("read");
        assert_eq!(again.writes(), [] as [Wrote; 0], "not a second issue");
        assert!(journal.lines.borrow().contains(&held_line("main_agent")));
        assert!(journal.events.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_closed_issue_of_an_earlier_red_does_not_stand_for_this_one() {
        let gh = board(
            Some(false),
            vec![human_issue(
                7,
                "Integration branch main_agent is red",
                "closed",
            )],
        );
        let (_, log) = capture();
        watch(&gh, "main_agent", false, &log).await.expect("read");
        assert!(
            gh.writes()
                .iter()
                .any(|w| matches!(w, Wrote::CreatedIssue(..))),
            "{:?}",
            gh.writes()
        );
    }

    #[tokio::test]
    async fn a_green_base_closes_the_issue_once_and_nothing_without_one() {
        let gh = board(
            Some(true),
            vec![
                human_issue(5, "Create the Stripe account", "open"),
                human_issue(7, "Integration branch main_agent is red", "open"),
            ],
        );
        let (journal, log) = capture();
        let read = watch(&gh, "main_agent", false, &log).await.expect("read");
        assert_eq!(read, Some(true));
        let writes = gh.writes();
        assert!(
            matches!(&writes[0], Wrote::Comment(7, said) if said.contains("green again")),
            "{writes:?}"
        );
        assert_eq!(writes[1], Wrote::Closed(7));
        assert_eq!(writes.len(), 2, "the other human issue is not touched");
        assert_eq!(
            journal.events.borrow().clone(),
            vec![Event::BaseGreen {
                branch: "main_agent".to_string(),
            }]
        );
        let quiet = board(Some(true), Vec::new());
        let (journal, log) = capture();
        watch(&quiet, "main_agent", false, &log)
            .await
            .expect("read");
        assert_eq!(quiet.writes(), [] as [Wrote; 0]);
        assert!(journal.events.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_check_still_running_after_a_red_holds_the_plant_until_green() {
        // A red spell, then the fix pushed: its runs are queued for minutes.
        // The hold lasts — or catch_up would merge the unverified base into
        // every milestone branch a task takes it into meanwhile.
        let gh = board(
            None,
            vec![human_issue(
                7,
                "Integration branch main_agent is red",
                "open",
            )],
        );
        let (journal, log) = capture();
        let read = watch(&gh, "main_agent", false, &log).await.expect("read");
        assert_eq!(read, Some(false), "held until green");
        assert_eq!(gh.writes(), [] as [Wrote; 0], "the issue stays until green");
        assert!(journal.events.borrow().is_empty(), "no second red");
        assert_eq!(
            journal.lines.borrow().clone(),
            vec![pending_line("main_agent")]
        );
        assert_eq!(
            pending_line("main_agent"),
            "integration branch main_agent is not green again yet: its checks run — \
             development and merges wait"
        );
    }

    #[tokio::test]
    async fn a_check_still_running_or_none_yet_with_no_red_before_changes_nothing() {
        let gh = board(
            None,
            vec![human_issue(5, "Create the Stripe account", "open")],
        );
        let (journal, log) = capture();
        let read = watch(&gh, "main_agent", false, &log).await.expect("read");
        assert_eq!(read, None);
        assert_eq!(gh.writes(), [] as [Wrote; 0]);
        assert!(journal.lines.borrow().is_empty());
        // A red closed by hand lifted the hold; an absent branch in the fake
        // is `None` too.
        let closed_by_hand = board(
            None,
            vec![human_issue(
                7,
                "Integration branch main_agent is red",
                "closed",
            )],
        );
        assert_eq!(
            watch(&closed_by_hand, "main_agent", false, &log)
                .await
                .expect("read"),
            None
        );
        let unknown = FakeGitHub::default();
        assert_eq!(
            watch(&unknown, "main_agent", false, &log)
                .await
                .expect("read"),
            None
        );
    }

    #[tokio::test]
    async fn a_dry_run_reads_and_says_but_writes_nothing() {
        let red = board(Some(false), Vec::new());
        let (journal, log) = capture();
        assert_eq!(
            watch(&red, "main_agent", true, &log).await.expect("read"),
            Some(false)
        );
        assert_eq!(red.writes(), [] as [Wrote; 0]);
        assert!(journal.lines.borrow().contains(&held_line("main_agent")));
        let green = board(
            Some(true),
            vec![human_issue(
                7,
                "Integration branch main_agent is red",
                "open",
            )],
        );
        watch(&green, "main_agent", true, &log).await.expect("read");
        assert_eq!(green.writes(), [] as [Wrote; 0]);
    }

    #[tokio::test]
    async fn an_unread_sha_does_not_hold_the_issue_back() {
        let gh = FakeGitHub {
            branch_checks: HashMap::from([("main_agent".to_string(), Some(false))]),
            ..FakeGitHub::default()
        };
        let (journal, log) = capture();
        watch(&gh, "main_agent", false, &log).await.expect("read");
        assert_eq!(
            gh.writes(),
            vec![Wrote::CreatedIssue(
                "Integration branch main_agent is red".to_string(),
                body("main_agent", None),
                vec![labels::HUMAN.to_string()],
            )]
        );
        assert!(!body("main_agent", None).contains("(`"), "no sha to name");
        assert_eq!(
            journal.events.borrow().clone(),
            vec![Event::BaseRed {
                branch: "main_agent".to_string(),
                sha: None,
            }]
        );
    }

    #[test]
    fn the_issue_is_found_by_its_exact_title_among_the_open_human_issues() {
        let human = vec![
            human_issue(5, "Integration branch main is red", "open"),
            human_issue(6, "Integration branch main_agent is red", "closed"),
            human_issue(7, " Integration branch main_agent is red ", "open"),
        ];
        assert_eq!(open_issue(&human, "main_agent").map(|i| i.number), Some(7));
        assert_eq!(open_issue(&human, "main").map(|i| i.number), Some(5));
        assert_eq!(open_issue(&human, "develop"), None);
    }
}
