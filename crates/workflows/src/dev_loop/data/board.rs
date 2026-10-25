//! The board: the current milestone, its tasks, and which one comes next.
//!
//! The **read** side of the issue model, in one place. The round needs it to
//! choose, preflight to verify the board is reachable, a status report to say
//! where we are. Many readers, one way to read.
//!
//! Here, not in `harness-core::adapters`: assembling "the lowest-numbered open
//! milestone, then its sub-issues, then their blockers" is a policy, and an
//! adapter decides nothing.
//!
//! **`Board` is pure data**, unlike Python, where it carried the `gh` client
//! to avoid reopening one. In Rust the client is passed, and a board without
//! client is tested by building it by hand.

use harness_core::domain::{Halt, Issue, Outcome};
use harness_core::ports::shell::github::GitHub;

use crate::common::routing::{self, Graph};
use crate::common::{branching, labels};
use crate::dev_loop::data::tasks;
use crate::main_agent_merge::data::merge_train;

/// What the loop sees of the board, at a given moment.
#[derive(Debug, Clone, Default)]
pub struct Board {
    /// The current milestone.
    pub milestone: Issue,
    /// Its sub-issues, blockers included.
    pub tasks: Vec<Issue>,
    /// The task the resumption point designates, if any.
    ///
    /// It **overrides** the board's choice, and this is what lets a round
    /// finish when `/code` already merged: the issue is closed, so the board
    /// wouldn't pick it, and `/create-test` would be lost.
    pub resuming: Option<Issue>,
}

impl Board {
    /// Open agent tasks — empty means the milestone has nothing left to run.
    #[must_use]
    pub fn open_agents(&self) -> Vec<&Issue> {
        tasks::open_agent_tasks(&self.tasks)
    }

    /// The next task by the four rules, if any.
    #[must_use]
    pub fn next(&self) -> Option<&Issue> {
        tasks::next_task(&self.tasks)
    }

    /// The task with this number, if it can run right now — what a
    /// `--task` run picks instead of the board's own choice. A screen's
    /// milestone blockers are the router's to judge on the graph
    /// ([`tasks::runnable_for_lane`]): the lane reads none.
    #[must_use]
    pub fn wanted(&self, number: u64) -> Option<&Issue> {
        self.tasks
            .iter()
            .find(|task| task.number == number && tasks::runnable_for_lane(task))
    }

    /// The sub-issue of this milestone with this number, **open or not**.
    ///
    /// Closed included, by design: a round interrupted after `/code` merged
    /// must be able to finish, and the issue it finishes is already closed.
    #[must_use]
    pub fn find(&self, key: &str) -> Option<&Issue> {
        let number: u64 = key.parse().ok()?;
        self.tasks.iter().find(|task| task.number == number)
    }

    /// The stop message when tasks remain but none can run.
    ///
    /// This case is **not** "nothing left", and confusing them stops the loop
    /// when it should instead keep waiting: the milestone is not done, it
    /// awaits a human. It remains to say **which** gesture it awaits, because
    /// they're not the same — delivering everything and awaiting a merge is
    /// not the same answer as an unchecked `harness:ready` box.
    #[must_use]
    pub fn stuck(&self) -> String {
        let open = self.open_agents();
        let head = format!(
            "milestone {} has {} open task(s) but none can run:\n{}\n",
            self.milestone.reference(),
            open.len(),
            tasks::stuck_report(&self.tasks)
        );
        if !open.is_empty() && open.iter().all(|task| tasks::waiting_merge(task)) {
            return head
                + "Everything is delivered on the integration branch and \
                   waiting for you to merge it and close these issues. Nothing \
                   here is the harness's to do.";
        }
        head + &format!(
            "A task without {} was held by a human or opened by hand: pose it on \
             the one to work on next, or close what blocks it.",
            labels::READY
        )
    }
}

/// The board, read now.
///
/// # Errors
///
/// - [`Halt::Halted`] if there is no open milestone: nothing to work on,
///   and saying so names the unblocking gesture;
/// - adapter failure, propagated as-is. A read that fails never becomes
///   an empty board, which would read "milestone done" and trigger `/planner`.
pub async fn read(gh: &dyn GitHub) -> Outcome<Board> {
    read_on(gh, "").await
}

/// The board the branch names.
///
/// A milestone's own branch (`milestone/<n>-…`, [`branching::milestone_number`])
/// reads that milestone — several build at once, and a lane on #20's branch
/// must not pick #18's tasks — anything else the lowest open one, as [`read`]
/// does.
///
/// # Errors
/// [`Halt::Halted`] when the milestone the branch names is not open, or no
/// milestone is; adapter failure propagated as-is.
pub async fn read_on(gh: &dyn GitHub, branch: &str) -> Outcome<Board> {
    let found = gh.issues_labelled(labels::MILESTONE, "open").await?;
    let milestone = if let Some(number) = branching::milestone_number(branch) {
        let Some(named) = found.iter().find(|m| m.number == number && m.is_open()) else {
            return Err(Halt::Halted(format!(
                "milestone #{number}, which {branch} belongs to, is not open — nothing to \
                 work from on this branch"
            )));
        };
        named
    } else {
        let Some(lowest) = tasks::current_milestone(&found) else {
            return Err(Halt::Halted(format!(
                "no open {} issue — there is nothing to work from. /planner opens \
                 one from a {} issue, triggered on its own rather than from here.",
                labels::MILESTONE,
                labels::ROADMAP
            )));
        };
        lowest
    };
    read_of(gh, milestone.clone()).await
}

/// This milestone's board: its sub-issues, the open ones' blockers read —
/// a closed task runs no more, and its blockers were one GitHub call each.
///
/// # Errors
/// Adapter failure, propagated as-is.
pub async fn read_of(gh: &dyn GitHub, milestone: Issue) -> Outcome<Board> {
    read_of_those(gh, milestone, &|_| true).await
}

/// This milestone's board, the blockers read for the open tasks `may_run`
/// admits only.
///
/// A task held anyway — a crate waiting on its producers — is not worth one
/// GitHub call per tick, and eight boards of them spent the rate limit.
///
/// # Errors
/// Adapter failure, propagated as-is.
pub async fn read_of_those(
    gh: &dyn GitHub,
    milestone: Issue,
    may_run: &dyn Fn(&Issue) -> bool,
) -> Outcome<Board> {
    let subs = gh.sub_issues(milestone.number).await?;
    let open: Vec<Issue> = subs
        .iter()
        .filter(|t| t.is_open() && may_run(t))
        .cloned()
        .collect();
    let held = gh.with_blockers(open).await?;
    let tasks = subs
        .into_iter()
        .map(|task| {
            held.iter()
                .find(|h| h.number == task.number)
                .cloned()
                .unwrap_or(task)
        })
        .collect();
    Ok(Board {
        milestone,
        tasks,
        resuming: None,
    })
}

/// The boards of the milestones building now, read until they offer `lanes`
/// runnable tasks.
///
/// In [`routing::building`]'s order: the critical path's first, the lowest
/// open milestone's when none is split yet. A tick fills that many lanes at
/// most, and every board is one GitHub call per open task. A board that
/// cannot be read is left out.
///
/// While a milestone before it in plan order still builds
/// ([`merge_train::unlanded_before`]), only what may run ahead
/// ([`tasks::runs_ahead`], judged on what each task needs) is worth its
/// blockers' GitHub calls and counts as
/// offered — counting the held crates stopped the reads at two boards and
/// left the lanes empty.
pub async fn building(
    gh: &dyn GitHub,
    milestones: &[Issue],
    graph: Option<&Graph>,
    lanes: usize,
) -> Vec<Board> {
    let building = routing::building(milestones, graph);
    let mut boards = Vec::with_capacity(building.len());
    let mut offered = 0usize;
    for number in building {
        if offered >= lanes {
            break;
        }
        let waiting_on = merge_train::unlanded_before(graph, number, milestones);
        let may_run = |task: &Issue| {
            waiting_on.is_empty() || tasks::runs_ahead(task, number, graph, &waiting_on)
        };
        if let Some(milestone) = milestones.iter().find(|m| m.number == number)
            && let Ok(board) = read_of_those(gh, milestone.clone(), &may_run).await
        {
            // Counted as the lanes take them: startable among what may run,
            // one per aggregate — six "offered" that could not start stopped
            // the reads at two boards and left the lanes empty.
            let could_start = tasks::startable_tasks(&board.tasks, &[], graph, &may_run).len();
            offered = offered.saturating_add(could_start);
            boards.push(board);
        }
    }
    boards
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("issue {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn task(number: u64) -> Issue {
        issue(number, &[labels::AGENT, labels::READY])
    }

    #[tokio::test]
    async fn the_board_reads_the_lowest_open_milestone_and_its_tasks() {
        let gh = FakeGitHub {
            issues: vec![
                issue(9, &[labels::MILESTONE]),
                issue(4, &[labels::MILESTONE]),
            ],
            subs: vec![(4, vec![task(11), task(12)])],
            ..FakeGitHub::default()
        };
        let board = read(&gh).await.expect("a board");
        assert_eq!(board.milestone.number, 4);
        assert_eq!(board.next().expect("a task").number, 11);
    }

    #[tokio::test]
    async fn a_milestone_branch_reads_its_own_milestone_s_board() {
        let gh = FakeGitHub {
            issues: vec![
                issue(9, &[labels::MILESTONE]),
                issue(4, &[labels::MILESTONE]),
            ],
            subs: vec![(4, vec![task(11)]), (9, vec![task(21)])],
            ..FakeGitHub::default()
        };
        let board = read_on(&gh, "milestone/9-sessions").await.expect("a board");
        assert_eq!(board.milestone.number, 9);
        assert_eq!(board.next().expect("a task").number, 21);
        let lowest = read_on(&gh, "main_agent").await.expect("a board");
        assert_eq!(lowest.milestone.number, 4);
        let err = read_on(&gh, "milestone/7-gone")
            .await
            .expect_err("not open");
        assert!(
            matches!(err, Halt::Halted(ref said) if said.contains("#7")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn no_open_milestone_halts_and_names_the_gesture() {
        let gh = FakeGitHub::default();
        let err = read(&gh).await.expect_err("must stop");
        let Halt::Halted(said) = err else {
            panic!("a voluntary stop, not a failure");
        };
        assert!(said.contains(labels::MILESTONE));
        assert!(said.contains("/planner"), "say what to do");
    }

    #[tokio::test]
    async fn an_unreadable_read_propagates_and_never_becomes_an_empty_board() {
        // Failure mode avoided: an empty board reads "milestone done" and
        // triggers a paid /planner.
        let gh = FakeGitHub {
            broken: Some(Halt::Unreadable("expired token".to_string())),
            ..FakeGitHub::default()
        };
        let err = read(&gh).await.expect_err("must fail");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[test]
    fn a_task_due_a_re_study_is_not_picked_and_the_next_one_is() {
        // #219 after its second discovery: a sequential dev loop must not
        // start a code session on it.
        let mut due = task(219);
        due.body = "## Decision\n\nkind: wait\nblocks: AGENT_LOOP_STOP: tasks [...]\nround: \
                    1\nmilestone: #28\ndiscovered: 2"
            .to_string();
        let board = Board {
            milestone: issue(28, &[labels::MILESTONE]),
            tasks: vec![due.clone(), task(220)],
            resuming: None,
        };
        assert_eq!(board.next().map(|t| t.number), Some(220));
        let alone = Board {
            tasks: vec![due.clone()],
            ..board.clone()
        };
        assert_eq!(alone.next().map(|t| t.number), None);
        assert!(
            tasks::startable_tasks(&alone.tasks, &[], None, &|_| true).is_empty(),
            "a lane does not start it either"
        );
        // Re-studied once on this discovery: it runs again.
        due.body.push_str("\nrestudied: 2\nrestudies: 1");
        let studied = Board {
            tasks: vec![due],
            ..board
        };
        assert_eq!(studied.next().map(|t| t.number), Some(219));
    }

    #[test]
    fn find_returns_a_closed_task_so_an_interrupted_round_can_finish() {
        let mut done = task(11);
        done.state = "closed".to_string();
        let board = Board {
            milestone: issue(4, &[labels::MILESTONE]),
            tasks: vec![done],
            resuming: None,
        };
        // The resumed round works on an issue already closed by /code's merge:
        // excluding it would lose /create-test.
        assert_eq!(board.find("11").expect("found").number, 11);
        assert!(board.find("99").is_none());
        assert!(board.find("not a number").is_none());
    }

    #[test]
    fn wanted_gives_the_numbered_task_only_when_it_can_run() {
        let board = Board {
            milestone: issue(4, &[labels::MILESTONE]),
            // #11 may run; #12 has no `harness:ready`, so it may not.
            tasks: vec![task(11), issue(12, &[labels::AGENT])],
            resuming: None,
        };
        assert_eq!(board.wanted(11).map(|task| task.number), Some(11));
        assert!(board.wanted(12).is_none(), "a task that cannot run");
        assert!(board.wanted(99).is_none(), "not on this board");
    }

    #[test]
    fn stuck_tells_you_to_merge_when_everything_is_delivered() {
        let board = Board {
            milestone: issue(4, &[labels::MILESTONE]),
            tasks: vec![issue(11, &[labels::AGENT, labels::WAITING_MERGE])],
            resuming: None,
        };
        let said = board.stuck();
        assert!(said.contains("waiting for you to merge"));
        assert!(!said.contains(labels::READY), "not the gesture here");
    }

    #[test]
    fn stuck_tells_you_what_to_tick_when_nothing_is_ready() {
        let board = Board {
            milestone: issue(4, &[labels::MILESTONE]),
            tasks: vec![issue(11, &[labels::AGENT])],
            resuming: None,
        };
        let said = board.stuck();
        assert!(said.contains(labels::READY));
        assert!(!said.contains("waiting for you to merge"));
    }

    #[tokio::test]
    async fn the_blockers_of_the_open_tasks_alone_are_read_and_the_order_is_kept() {
        let mut done = task(12);
        done.state = "closed".to_string();
        let gh = FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![task(11), done, task(13)])],
            ..FakeGitHub::default()
        };
        let board = read(&gh).await.expect("a board");
        assert_eq!(
            board.tasks.iter().map(|t| t.number).collect::<Vec<_>>(),
            [11, 12, 13]
        );
        assert_eq!(*gh.blockers_asked.borrow(), vec![11, 13]);
    }

    #[tokio::test]
    async fn the_blockers_are_read_for_the_tasks_that_may_run_only() {
        let gh = FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![task(11), task(12), task(13)])],
            ..FakeGitHub::default()
        };
        let board = read_of_those(&gh, issue(4, &[labels::MILESTONE]), &|t| t.number != 12)
            .await
            .expect("a board");
        assert_eq!(board.tasks.len(), 3, "every task is on the board");
        assert_eq!(*gh.blockers_asked.borrow(), vec![11, 13]);
    }

    #[tokio::test]
    async fn the_building_boards_read_what_may_run_until_the_lanes_are_offered() {
        let unit = |number: u64, unit: &str| Issue {
            body: format!("## Scope\n\nx\n\n## Architecture\n\nunit: {unit}\nsystem: media"),
            ..task(number)
        };
        let graph = Graph::parse(
            r#"{"version":1,"milestones":[
              {"number":20,"title":"sessions","systems":["session"],"produces":["session.listScenes"],"consumes":[],"depends_on":[]},
              {"number":19,"title":"media","systems":["media"],"produces":["media.listMedia"],"consumes":["session.listScenes"],"depends_on":[20]}
            ],"ids":{
              "media.listMedia":{"kind":"capability","producer":19,"state":"planned","version":1},
              "session.listScenes":{"kind":"capability","producer":20,"state":"planned","version":1},
              "Session":{"kind":"aggregate","producer":20,"state":"planned","version":1}
            }}"#,
        )
        .expect("a graph");
        let milestones = vec![
            issue(19, &[labels::MILESTONE, labels::TRIGGERED]),
            issue(20, &[labels::MILESTONE, labels::TRIGGERED]),
        ];
        // #31 reads a contract of #20 (mocked by its fixture); #32 reads
        // #20's table, which no fixture stands in for.
        let needing = |number: u64, unit: &str, needs: &str| Issue {
            body: format!(
                "## Scope\n\nx\n\n## Architecture\n\nunit: {unit}\nsystem: media\nneeds: {needs}"
            ),
            ..task(number)
        };
        let fake = || FakeGitHub {
            issues: milestones.clone(),
            subs: vec![
                (20, vec![unit(21, "capability")]),
                (
                    19,
                    vec![
                        needing(31, "micro-ui", "session.listScenes"),
                        needing(32, "capability", "Session"),
                    ],
                ),
            ],
            ..FakeGitHub::default()
        };
        let gh = fake();
        let boards = building(&gh, &milestones, Some(&graph), 5).await;
        let mut read: Vec<u64> = boards.iter().map(|b| b.milestone.number).collect();
        read.sort_unstable();
        assert_eq!(read, [19, 20]);
        let mut asked = gh.blockers_asked.borrow().clone();
        asked.sort_unstable();
        assert_eq!(
            asked,
            [21, 31],
            "#19 waits on #20: #32 reads its table and is not worth a call"
        );
        let gh = fake();
        let one = building(&gh, &milestones, Some(&graph), 1).await;
        assert_eq!(one.len(), 1, "one lane, offered by the first board");
        assert!(
            building(&fake(), &milestones, Some(&graph), 0)
                .await
                .is_empty()
        );
    }
}
