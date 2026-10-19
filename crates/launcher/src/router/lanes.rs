//! The lanes of a parallel watch: one scheduler for every route
//! ([`routing::schedule`]). Each tick, every job the snapshot shows is
//! started in weight order — a free one (the two merges) in the tick, a paid
//! one on a free lane, a `harness` child in a checkout of its own: a task as
//! the dev loop's `--task`, anything else as `harness route <kind> <n>`.
//! Nothing paid runs in this process, so the tick never waits on a session
//! and the lanes are refilled every tick.

use std::path::Path;

use harness_core::domain::Issue;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook};
use harness_workflows::common::branching;
use harness_workflows::common::routing::{self, Pulse, Route, Snapshot};
use harness_workflows::dev_loop::data::tasks;
use harness_workflows::main_agent_merge::data::merge_train;

use crate::cli::WatchArgs;
use crate::dispatch::lanes::Lanes;

use super::Dispatched;
use super::routes::{after, dispatched, lane_command, lane_work};

/// How many jobs the debug line of a tick's schedule names.
const SAID_JOBS: usize = 12;

/// Starts every job the snapshot shows, heaviest first: the free ones in
/// the tick, the paid ones on free lanes — none during a quota rest
/// (`paid` false). One job per issue or PR at a time, one per exclusive
/// group; what is left waits a tick more ([`Pulse::age`]).
#[allow(clippy::too_many_arguments)] // the tick's state, handed as it is
pub(super) async fn run_schedule(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    found: &Snapshot,
    pulse: &mut Pulse,
    paid: bool,
    lanes: &mut Lanes,
    log: &Logbook,
) -> Dispatched {
    let running = lanes.running();
    for board in &found.boards {
        // Said once per tick: what of the board waits on its producers.
        let _ = startable_now(&board.tasks, &running, board.milestone.number, found, log);
    }
    let jobs = routing::schedule(found, &running, &|route| pulse.waited(route));
    log.say(&format!(
        "tick: {:?}",
        jobs.first().map_or(&Route::Nothing, |job| &job.route)
    ));
    log.debug(&format!(
        "schedule: {}",
        jobs.iter()
            .take(SAID_JOBS)
            .map(|job| format!("{:?}={}", job.route, job.weight))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let routing::Plan {
        free,
        paid: to_start,
        mut left,
    } = routing::plan(jobs, &running, &lanes.groups(), lanes.free_slots(), paid);
    let mut failed = None;
    for job in free {
        if lanes.is_closed() {
            break;
        }
        if let Some(halt) = dispatched(args, here, gh, job.route, log).await
            && failed.is_none()
        {
            failed = Some(halt);
        }
    }
    let mut caught_up: Vec<u64> = Vec::new();
    for job in to_start {
        let Some(slot) = lanes.free_slot() else {
            left.push(job.route);
            continue;
        };
        let started = match &job.route {
            Route::DevTask { milestone, task } => {
                start_task(
                    args,
                    here,
                    gh,
                    found,
                    (*milestone, *task),
                    slot,
                    &mut caught_up,
                    lanes,
                    log,
                )
                .await
            }
            route => start_route(args, here, route, job.exclusive, slot, lanes, log),
        };
        if !started {
            left.push(job.route);
        }
    }
    pulse.age(&left);
    // A paid job's success is its lane's (`Lanes::went_through`): only a
    // failed free one is this tick's to report.
    after(args, here, failed, false, log).await
}

/// Starts `route` on lane `slot` as `harness route <kind> <n>`: whether it
/// started.
fn start_route(
    args: &WatchArgs,
    here: &Path,
    route: &Route,
    group: Option<&'static str>,
    slot: usize,
    lanes: &mut Lanes,
    log: &Logbook,
) -> bool {
    let Some((kind, number, base)) = lane_work(route) else {
        return false;
    };
    let Ok(exe) = std::env::current_exe() else {
        log.error("watch: lanes -> cannot find this executable");
        return false;
    };
    let what = clap::ValueEnum::to_possible_value(&kind)
        .map_or_else(|| "route".to_string(), |v| v.get_name().to_string());
    let command = lane_command(args, here, &exe, &what, number, &base, slot);
    match lanes.spawn_in(slot, number, group, command, here) {
        Ok(pid) => {
            log.event(&Event::PreparationTaken {
                lane: u32::try_from(slot).unwrap_or(u32::MAX),
                what,
                issue: number,
                pid,
            });
            true
        }
        Err(e) => {
            log.event(&Event::LaneNotStarted {
                what: Some(what),
                issue: number,
                reason: e.to_string(),
            });
            false
        }
    }
}

/// What a parked task is compared on: its body, its labels and the blockers
/// still holding it ([`tasks::still_blocks`]). Answering the stop — editing
/// the SPEC, a label — moves it; so does a blocker landing: `ForgeDesMondes`'
/// #247 and #248, taken in the second before their `blocked_by` were posted,
/// stopped on "cannot run: blocked" and would have stayed parked after #244
/// and #246 closed; a task parked until a milestone lands moves at its
/// `harness:waiting-merge`, when the code it needs is on the integration
/// branch, not at the close that comes later.
fn issue_version(issue: &Issue) -> String {
    let mut labels = issue.labels.clone();
    labels.sort_unstable();
    let mut open_blockers: Vec<u64> = issue
        .blocked_by
        .iter()
        .filter(|b| tasks::still_blocks(b))
        .map(|b| b.number)
        .collect();
    open_blockers.sort_unstable();
    harness_core::domain::breaker::fingerprint(&format!(
        "{}\n{}\n{open_blockers:?}",
        issue.body,
        labels.join(",")
    ))
}

/// What may start of the milestone's tasks: runnable, not on a lane, and —
/// while the producers of what the milestone reads still build — nothing it
/// needs is a table, a Concept or a Micro-UI one of them owns. The aggregate
/// hold is taken among those only: a task held by its producers holds no
/// aggregate. What is held is said once per tick.
fn startable_now<'a>(
    issues: &'a [Issue],
    running: &[u64],
    milestone: u64,
    found: &Snapshot,
    log: &Logbook,
) -> Vec<&'a Issue> {
    let waiting_on =
        merge_train::producers_landed(found.graph.as_ref(), milestone, &found.milestones);
    let ahead = |task: &Issue| {
        waiting_on.is_empty()
            || tasks::runs_ahead(task, milestone, found.graph.as_ref(), &waiting_on)
    };
    say_held(issues, running, milestone, &waiting_on, &ahead, log);
    tasks::startable_tasks(issues, running, &ahead)
}

/// Says once per tick which runnable tasks of the milestone wait on its
/// producers: those `ahead` refuses among the runnable not on a lane.
fn say_held(
    issues: &[Issue],
    running: &[u64],
    milestone: u64,
    waiting_on: &[u64],
    ahead: &dyn Fn(&Issue) -> bool,
    log: &Logbook,
) {
    if waiting_on.is_empty() {
        return;
    }
    let held: Vec<u64> = tasks::runnable_tasks(issues)
        .into_iter()
        .filter(|task| !running.contains(&task.number) && !ahead(task))
        .map(|task| task.number)
        .collect();
    if held.is_empty() {
        return;
    }
    let list = |numbers: &[u64]| {
        numbers
            .iter()
            .map(|n| format!("#{n}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    log.say(&format!(
        "watch: lanes -> #{milestone}'s tasks reading tables, Concepts or Micro-UIs still building ({}) wait on {} to land — ahead work is what the mock-up stands in for",
        list(&held),
        list(waiting_on)
    ));
}

/// Starts `task` of `milestone` on lane `slot` — a `harness --task <n>`
/// child on the milestone's own branch (`common::branching`), never the
/// fixed `--branch`. A task parked on its issue's very version is not
/// started; the milestone's branch takes the integration branch first,
/// once a tick (`caught_up`). Whether it started.
#[allow(clippy::too_many_arguments)] // the tick's state, handed as it is
async fn start_task(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    found: &Snapshot,
    (milestone, number): (u64, u64),
    slot: usize,
    caught_up: &mut Vec<u64>,
    lanes: &mut Lanes,
    log: &Logbook,
) -> bool {
    let Some(board) = found
        .boards
        .iter()
        .find(|b| b.milestone.number == milestone)
    else {
        return false;
    };
    let Some(task) = board.tasks.iter().find(|t| t.number == number) else {
        return false;
    };
    let version = issue_version(task);
    if lanes.is_parked(number, &version) {
        log.debug(&format!(
            "watch: lanes -> #{number} parked until its issue changes"
        ));
        return false;
    }
    let branch = branching::milestone_branch(milestone, &board.milestone.title);
    if !args.dry_run && !caught_up.contains(&milestone) {
        branching::catch_up(gh, &branch, &args.branch, log).await;
        caught_up.push(milestone);
    }
    let Ok(exe) = std::env::current_exe() else {
        log.error("watch: lanes -> cannot find this executable");
        return false;
    };
    let mut command = tokio::process::Command::new(exe);
    command
        .current_dir(here)
        .arg("--task")
        .arg(number.to_string())
        .arg("--branch")
        .arg(&branch)
        .arg("--use-workspace")
        .arg(format!("lane-{slot}"))
        .arg("--lane")
        .arg("--quiet");
    for (on, flag) in [
        (args.force_reset, "--force-reset"),
        (args.dry_run, "--dry-run"),
        (args.no_human_gate, "--no-human-gate"),
    ] {
        if on {
            command.arg(flag);
        }
    }
    lanes.taken(number, version);
    match lanes.spawn(slot, number, command, here) {
        Ok(pid) => {
            log.event(&Event::LaneTaken {
                lane: u32::try_from(slot).unwrap_or(u32::MAX),
                task: number,
                branch,
                pid,
            });
            true
        }
        Err(e) => {
            log.event(&Event::LaneNotStarted {
                what: None,
                issue: number,
                reason: e.to_string(),
            });
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_workflows::common::labels;
    use std::rc::Rc;

    fn issue(number: u64, labels: &[&str], unit: &str) -> Issue {
        needing(number, labels, unit, "")
    }

    fn needing(number: u64, labels: &[&str], unit: &str, needs: &str) -> Issue {
        Issue {
            number,
            title: format!("task {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: format!(
                "## Scope\n\nx\n\n## Architecture\n\nunit: {unit}\nsystem: media\nneeds: {needs}"
            ),
            ..Issue::default()
        }
    }

    fn graph() -> routing::Graph {
        routing::Graph::parse(
            r#"{"version":1,"milestones":[
              {"number":20,"title":"sessions","systems":["session"],"produces":["session.listScenes"],"consumes":[],"depends_on":[]},
              {"number":19,"title":"media","systems":["media"],"produces":["media.listMedia"],"consumes":["session.listScenes"],"depends_on":[20]}
            ],"ids":{
              "media.listMedia":{"kind":"capability","producer":19,"state":"planned","version":1},
              "session.listScenes":{"kind":"capability","producer":20,"state":"planned","version":1},
              "Session":{"kind":"aggregate","producer":20,"state":"planned","version":1},
              "SceneBoard":{"kind":"micro-ui","producer":20,"state":"planned","version":1}
            }}"#,
        )
        .expect("a graph")
    }

    /// A sink that keeps what the log said, for the lines a tick says once.
    #[derive(Default)]
    struct Capture(std::cell::RefCell<Vec<String>>);

    impl harness_core::traces::Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
        fn keep(&self, _line: &str) {}
    }

    fn snapshot(milestones: Vec<Issue>, graph: Option<routing::Graph>) -> Snapshot {
        Snapshot {
            roadmap: Vec::new(),
            lowest_roadmap_has_milestone: false,
            milestones,
            prs_to_fix: Vec::new(),
            prs_to_review: Vec::new(),
            tasks_to_merge: Vec::new(),
            ready_to_merge: Vec::new(),
            refining: Vec::new(),
            tech_refining: Vec::new(),
            arbitrating: Vec::new(),
            dev_loop_milestones: Vec::new(),
            boards: Vec::new(),
            graph,
            contract_changes: Vec::new(),
            test_debt: Vec::new(),
            rules: Vec::new(),
            deciding: Vec::new(),
            base_checks: None,
        }
    }

    #[test]
    fn a_parked_task_moves_when_a_blocker_closes_as_when_its_body_or_labels_do() {
        let mut task = issue(247, &[labels::AGENT, labels::READY], "aggregate");
        let mut blocker = issue(244, &[labels::AGENT, labels::READY], "aggregate");
        task.blocked_by = vec![blocker.clone()];
        let parked_on = issue_version(&task);
        assert_eq!(
            issue_version(&task),
            parked_on,
            "stable while nothing moves"
        );
        blocker.state = "closed".to_string();
        task.blocked_by = vec![blocker];
        assert_ne!(
            issue_version(&task),
            parked_on,
            "its blocker closed: it moved"
        );
        let unparked = issue_version(&task);
        task.labels.push(labels::NEEDS_DECISION.to_string());
        assert_ne!(issue_version(&task), unparked, "a label moves it");
        task.labels.pop();
        task.body.push_str("\nmore");
        assert_ne!(issue_version(&task), unparked, "its body moves it");
    }

    #[test]
    fn a_task_parked_until_a_milestone_lands_moves_at_its_waiting_merge() {
        // #214 waited for #20 (its stop's blocker): parked on the version
        // with #20 open, it moves when #20's PR lands on the integration
        // branch — the label — not only when #20 closes.
        let mut task = issue(214, &[labels::AGENT, labels::READY], "capability");
        let mut producer = issue(20, &[labels::MILESTONE, labels::TRIGGERED], "");
        task.blocked_by = vec![producer.clone()];
        let parked_on = issue_version(&task);
        producer.labels.push(labels::WAITING_MERGE.to_string());
        task.blocked_by = vec![producer.clone()];
        let landed = issue_version(&task);
        assert_ne!(landed, parked_on, "its producer landed: it moved");
        producer.state = "closed".to_string();
        task.blocked_by = vec![producer];
        assert_eq!(
            issue_version(&task),
            landed,
            "closing a landed producer changes nothing it waits on"
        );
    }

    #[test]
    fn what_starts_now_is_said_once_with_what_waits_on_the_producers() {
        // #60 reads a contract of #20 (mocked); #62 reads #20's table and
        // waits; #63 is already on a lane and is neither started nor said.
        let ui = needing(
            60,
            &[labels::AGENT, labels::READY],
            "micro-ui",
            "session.listScenes",
        );
        let table = needing(62, &[labels::AGENT, labels::READY], "capability", "Session");
        let on_lane = needing(63, &[labels::AGENT, labels::READY], "capability", "Session");
        let milestones = vec![
            issue(19, &[labels::MILESTONE], "-"),
            issue(20, &[labels::MILESTONE], "-"),
        ];
        let issues = [ui, table, on_lane];
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(
            Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        let found = snapshot(milestones.clone(), Some(graph()));
        let started: Vec<u64> = startable_now(&issues, &[63], 19, &found, &log)
            .iter()
            .map(|t| t.number)
            .collect();
        assert_eq!(started, [60]);
        let said = capture.0.borrow().join("\n");
        assert!(
            said.contains("#19's tasks reading tables, Concepts or Micro-UIs still building (#62) wait on #20 to land"),
            "{said}"
        );
        assert!(
            !said.contains("#63"),
            "a task on a lane is not said to wait: {said}"
        );
        // No producer building: everything not on a lane starts, nothing is said.
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(
            Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        let landed = snapshot(milestones, None);
        let started: Vec<u64> = startable_now(&issues, &[], 19, &landed, &log)
            .iter()
            .map(|t| t.number)
            .collect();
        assert_eq!(started, [60, 62, 63]);
        assert!(capture.0.borrow().is_empty(), "{:?}", capture.0.borrow());
    }

    #[test]
    fn ahead_of_its_producers_a_milestone_starts_what_reads_contracts_only() {
        // #60 reads a contract of #20 (mocked); #61 composes a Micro-UI #20
        // builds; #62 reads #20's table — the last two wait for #20.
        let ui = needing(
            60,
            &[labels::AGENT, labels::READY],
            "micro-ui",
            "session.listScenes",
        );
        let screen = needing(
            61,
            &[labels::AGENT, labels::READY],
            "composition",
            "SceneBoard",
        );
        let crate_task = needing(62, &[labels::AGENT, labels::READY], "capability", "Session");
        let milestones = [
            issue(19, &[labels::MILESTONE], "-"),
            issue(20, &[labels::MILESTONE], "-"),
        ];
        let graph = graph();
        let issues = [ui, screen, crate_task];
        let taken = |graph: Option<&routing::Graph>, milestone: u64| {
            let waiting_on = merge_train::producers_landed(graph, milestone, &milestones);
            let ahead = |task: &Issue| {
                waiting_on.is_empty() || tasks::runs_ahead(task, milestone, graph, &waiting_on)
            };
            tasks::startable_tasks(&issues, &[], &ahead)
                .iter()
                .map(|t| t.number)
                .collect::<Vec<_>>()
        };
        assert_eq!(taken(Some(&graph), 19), [60]);
        assert_eq!(
            taken(Some(&graph), 20),
            [60, 61, 62],
            "no producer: everything"
        );
        assert_eq!(taken(None, 19), [60, 61, 62], "no graph: everything");
    }
}
