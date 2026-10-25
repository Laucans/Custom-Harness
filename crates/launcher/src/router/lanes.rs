//! The lanes of a parallel watch: one scheduler for every route
//! ([`routing::schedule`]). Each tick, every job the snapshot shows is
//! started in weight order — a free one (the two merges) in the tick, a paid
//! one on a free lane, a `harness` child in a checkout of its own: a task as
//! the dev loop's `--task`, anything else as `harness route <kind> <n>`.
//! Nothing paid runs in this process, so the tick never waits on a session
//! and the lanes are refilled every tick.

use std::path::Path;

use harness_core::domain::Issue;
use harness_core::domain::workspace::Workspace;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook};
use harness_workflows::common::branching;
use harness_workflows::common::catch_up::Busy;
use harness_workflows::common::routing::{self, Pulse, Route, Snapshot};
use harness_workflows::dev_loop::data::tasks;
use harness_workflows::main_agent_merge::data::merge_train;

use crate::cli::WatchArgs;
use crate::dispatch::lanes::{Lanes, prune_target};

use super::Dispatched;
use super::floor::GIB;
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
        let _ = startable_now(board, &running, found, log);
    }
    say_rules_held(found, log);
    let pulls = routing::pulls(found, &running);
    say_unpulled(&pulls, pulse, log);
    let pulled = routing::pulled(found, &running);
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
    let busy = Busy::of(&running, &found.boards);
    for job in free {
        if lanes.is_closed() {
            break;
        }
        if let Some(halt) = dispatched(args, here, gh, job.route, pulse, &busy, log).await
            && failed.is_none()
        {
            failed = Some(halt);
        }
    }
    for job in to_start {
        let Some(slot) = lanes.free_slot() else {
            left.push(job.route);
            continue;
        };
        if !args.dry_run {
            prune_slot(
                args,
                here,
                slot,
                &harness_adapters::shell::disk::RealDisk,
                log,
            );
        }
        let started = match &job.route {
            Route::DevTask { milestone, task } => {
                start_task(args, here, found, (*milestone, *task), slot, lanes, log)
            }
            route => start_route(args, here, route, job.exclusive, slot, lanes, log),
        };
        if !started {
            left.push(job.route);
        } else if let Route::Split { milestone } = job.route
            && let Some(event) = pulled_event(pulled.as_ref(), milestone)
        {
            // Said when the split starts, not each tick it waits.
            log.event(&event);
        }
    }
    pulse.age(&left);
    // A paid job's success is its lane's (`Lanes::went_through`): only a
    // failed free one is this tick's to report.
    after(args, here, failed, false, log).await
}

/// The event of a split pulled past the cap, when `milestone` is the one
/// this tick pulled (`routing::pulled`: the first splittable pull the cap
/// holds back).
fn pulled_event(pulled: Option<&routing::Pull>, milestone: u64) -> Option<Event> {
    pulled
        .filter(|pull| pull.producer == milestone)
        .map(|pull| Event::SplitPulled {
            milestone: pull.producer,
            by_milestone: pull.milestone,
            by_task: pull.task,
            need: pull.need.clone(),
            why: pull.why.said().to_string(),
            through: pull.through,
        })
}

/// Says the producers a starved or held milestone would pull but may not
/// split, nor pull through ([`routing::stuck_pulls`]): once per change.
fn say_unpulled(pulls: &[routing::Pull], pulse: &mut Pulse, log: &Logbook) {
    let said = unpulled_lines(pulls);
    for line in routing::unpulled_news(pulse, said) {
        log.say(&line);
    }
}

/// What [`say_unpulled`] says of `pulls`, one line per stuck pull.
fn unpulled_lines(pulls: &[routing::Pull]) -> Vec<String> {
    routing::stuck_pulls(pulls)
        .into_iter()
        .map(|pull| {
            format!(
                "watch: #{} would be split past the cap for #{} of #{} ({}, {}), but its own \
                 producers are not split and none can be yet",
                pull.producer,
                pull.task,
                pull.milestone,
                pull.need,
                pull.why.said()
            )
        })
        .collect()
}

/// Says once per tick the tasks a human holds (no `harness:ready`) that
/// hold many pending rules ([`routing::RULES_SAID`] or more): what never
/// leaves them stuck in silence.
fn say_rules_held(found: &Snapshot, log: &Logbook) {
    for (task, unblocked) in routing::rule_unblockers(found) {
        if unblocked.touched < routing::RULES_SAID {
            break;
        }
        let held = found
            .boards
            .iter()
            .flat_map(|board| board.tasks.iter())
            .any(|issue| issue.number == task && !tasks::is_ready(issue));
        if held {
            log.say(&format!(
                "watch: #{task} has no {}, holds {} rules",
                harness_workflows::common::labels::READY,
                unblocked.touched
            ));
        }
    }
}

/// Prunes the build folders of lane `slot`'s two checkouts — its tasks'
/// (`lane-<k>`, under `AGENTIC_WORKSPACES_DIR`) and its routes'
/// (`router-lane-<k>`) — before it takes its next job: the slot is free, so
/// nothing builds in either ([`prune_target`]).
pub(super) fn prune_slot(
    args: &WatchArgs,
    here: &Path,
    slot: usize,
    disk: &dyn Disk,
    log: &Logbook,
) {
    let cap = args.lane_target_cap_gb.saturating_mul(GIB);
    let source = Workspace::new(here);
    let checkouts = [
        source
            .workspaces_in(&args.workspaces_dir)
            .join(format!("lane-{slot}")),
        source.workspaces().join(format!("router-lane-{slot}")),
    ];
    for checkout in checkouts {
        let target = checkout.join("target");
        if let Some(pruned) = prune_target(disk, &target, cap) {
            log.event(&Event::LaneTargetPruned {
                lane: u32::try_from(slot).unwrap_or(u32::MAX),
                target: source.rel(&target),
                before: pruned.before,
                after: pruned.after,
                what: pruned.what,
            });
        }
    }
}

/// How many pending rules `task` unblocks, when it is enough to say
/// ([`routing::RULES_SAID`]).
fn rules_held(found: &Snapshot, task: u64) -> Option<u32> {
    routing::rule_unblockers(found)
        .into_iter()
        .find(|(number, _)| *number == task)
        .map(|(_, unblocked)| unblocked.touched)
        .filter(|touched| *touched >= routing::RULES_SAID)
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

/// What may start of the milestone's tasks — [`routing::startable`].
///
/// Runnable, not on a lane, not held by its own pull request's cycle, and —
/// while a milestone before it in plan order has not landed — nothing it
/// needs is a table, a Concept or a Micro-UI (one a screen does not lay
/// out) one of them owns. The aggregate hold is taken among those only: a
/// task held by its producers holds no aggregate. What is held, and by
/// which producer, is said once per tick.
fn startable_now<'a>(
    board: &'a harness_workflows::dev_loop::data::board::Board,
    running: &[u64],
    found: &Snapshot,
    log: &Logbook,
) -> Vec<&'a Issue> {
    let milestone = board.milestone.number;
    let waiting_on =
        merge_train::unlanded_before(found.graph.as_ref(), milestone, &found.milestones);
    say_held(
        &board.tasks,
        running,
        milestone,
        found.graph.as_ref(),
        &waiting_on,
        log,
    );
    routing::startable(found, board, running)
}

/// Says once per tick which runnable tasks of the milestone wait on a
/// producer still building, and on which: those [`tasks::held_by`] names
/// among the runnable not on a lane.
fn say_held(
    issues: &[Issue],
    running: &[u64],
    milestone: u64,
    graph: Option<&routing::Graph>,
    waiting_on: &[u64],
    log: &Logbook,
) {
    if waiting_on.is_empty() {
        return;
    }
    let list = |numbers: &[u64]| {
        numbers
            .iter()
            .map(|n| format!("#{n}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let held: Vec<String> = tasks::runnable_tasks(issues, graph)
        .into_iter()
        .filter(|task| !running.contains(&task.number))
        .filter_map(|task| {
            let on = tasks::held_by(task, milestone, graph, waiting_on);
            (!on.is_empty()).then(|| format!("#{} on {}", task.number, list(&on)))
        })
        .collect();
    if held.is_empty() {
        return;
    }
    log.say(&format!(
        "watch: lanes -> #{milestone}'s tasks reading tables, Concepts or Micro-UIs still building wait for them to land ({}) — ahead work is what the mock-up stands in for",
        held.join("; ")
    ));
}

/// Starts `task` of `milestone` on lane `slot` — a `harness --task <n>`
/// child on the milestone's own branch (`common::branching`), never the
/// fixed `--branch`. A task parked on its issue's very version is not
/// started. The branch holds the integration branch's last green head: the
/// router holds the tasks of a branch behind it, or one that could not take
/// it, until a catch-up brought it up (`routing::Snapshot::catch_up`).
/// Whether it started.
fn start_task(
    args: &WatchArgs,
    here: &Path,
    found: &Snapshot,
    (milestone, number): (u64, u64),
    slot: usize,
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
        // Said, not debugged: a parked top task is why lanes sit idle.
        let holds = rules_held(found, number)
            .map_or_else(String::new, |rules| format!(", holds {rules} rules"));
        log.say(&format!(
            "watch: lanes -> #{number} parked until its issue changes or an hour passes{holds}"
        ));
        return false;
    }
    let branch = branching::milestone_branch(milestone, &board.milestone.title);
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
    use crate::dispatch::lanes::tests::Builds;
    use clap::Parser as _;

    #[test]
    fn a_free_lane_s_two_checkouts_are_pruned_and_said_before_its_next_job() {
        let watch = |cap: &str| match crate::cli::Cli::try_parse_from([
            "harness",
            "watch",
            "--parallel",
            "3",
            "--workspaces-dir",
            "../ws",
            "--lane-target-cap-gb",
            cap,
        ]) {
            Ok(crate::cli::Cli {
                command: Some(crate::cli::Command::Watch(args)),
                ..
            }) => args,
            other => panic!("a watch command line: {other:?}"),
        };
        let here = Path::new("/f/factory");
        let disk = Builds::with(&[
            // The task checkout's target: over twice 1 GB.
            (
                "/f/factory/../ws/lane-1/target/debug/deps/a-0123456789abcdef",
                3 * GIB,
                Some(1),
            ),
            // The route checkout's: over 1 GB, under 2.
            (
                "/f/factory/.llocal/agentic_workspaces/router-lane-1/target/debug/incremental/x",
                GIB + 1,
                Some(1),
            ),
            // Another lane's, far over: not this slot's.
            ("/f/factory/../ws/lane-2/target/x", 9 * GIB, Some(1)),
        ]);
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(
            Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        prune_slot(&watch("1"), here, 1, &disk, &log);
        assert_eq!(
            *disk.removed.borrow(),
            [
                std::path::PathBuf::from("/f/factory/../ws/lane-1/target/debug"),
                std::path::PathBuf::from(
                    "/f/factory/.llocal/agentic_workspaces/router-lane-1/target/debug/incremental"
                ),
            ]
        );
        let said = capture.0.borrow().join("\n");
        assert!(
            said.contains(
                "lane 1: ../ws/lane-1/target pruned 3.0 GB -> 0.0 GB (the debug profile, above twice the cap)"
            ),
            "{said}"
        );
        assert!(
            said.contains(
                "lane 1: .llocal/agentic_workspaces/router-lane-1/target pruned 1.0 GB -> 0.0 GB \
                 (incremental caches)"
            ),
            "{said}"
        );
        // A cap of 0 prunes nothing.
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(
            Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        prune_slot(&watch("0"), here, 2, &disk, &log);
        assert_eq!(disk.removed.borrow().len(), 2);
        assert!(capture.0.borrow().is_empty());
    }
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
            task_prs: Vec::new(),
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
            database_down: false,
            review_backlog_full: false,
            catch_up: Vec::new(),
            catch_up_held: Vec::new(),
            prs_waiting: Vec::new(),
            rules_on_pr: Vec::new(),
            settled: Vec::new(),
            tasks_merged: Vec::new(),
            contracts_landed: Vec::new(),
            debt_landings: Vec::new(),
            rule_waits: harness_workflows::common::rule_waits::Waits::default(),
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
        let board = harness_workflows::dev_loop::data::board::Board {
            milestone: issue(19, &[labels::MILESTONE], "-"),
            tasks: vec![ui, table, on_lane],
            resuming: None,
        };
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(
            Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        let found = snapshot(milestones.clone(), Some(graph()));
        let started: Vec<u64> = startable_now(&board, &[63], &found, &log)
            .iter()
            .map(|t| t.number)
            .collect();
        assert_eq!(started, [60]);
        let said = capture.0.borrow().join("\n");
        assert!(
            said.contains("#19's tasks reading tables, Concepts or Micro-UIs still building wait for them to land (#62 on #20)"),
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
        let started: Vec<u64> = startable_now(&board, &[], &landed, &log)
            .iter()
            .map(|t| t.number)
            .collect();
        assert_eq!(started, [60, 62, 63]);
        assert!(capture.0.borrow().is_empty(), "{:?}", capture.0.borrow());
        // A task its own PR's cycle holds is neither started nor said held.
        let owned = Snapshot {
            task_prs: vec![harness_core::domain::Pr {
                num: "7".to_string(),
                title: "the micro-ui (#60)".to_string(),
                state: "OPEN".to_string(),
                labels: vec![labels::TO_REVIEW.to_string()],
                ..harness_core::domain::Pr::default()
            }],
            ..snapshot(Vec::new(), None)
        };
        let started: Vec<u64> = startable_now(&board, &[], &owned, &log)
            .iter()
            .map(|t| t.number)
            .collect();
        assert_eq!(started, [62, 63], "#60's PR is under review");
    }

    #[test]
    fn ahead_of_its_producers_a_milestone_starts_what_the_mock_up_stands_in_for() {
        // #60 reads a contract of #20 (mocked); #61 composes a Micro-UI #20
        // builds, against the mock-up's fragment; #62 reads #20's table and
        // alone waits for #20.
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
            let waiting_on = merge_train::unlanded_before(graph, milestone, &milestones);
            let ahead = |task: &Issue| {
                waiting_on.is_empty() || tasks::runs_ahead(task, milestone, graph, &waiting_on)
            };
            tasks::startable_tasks(&issues, &[], graph, &ahead)
                .iter()
                .map(|t| t.number)
                .collect::<Vec<_>>()
        };
        assert_eq!(taken(Some(&graph), 19), [60, 61]);
        assert_eq!(
            taken(Some(&graph), 20),
            [60, 61, 62],
            "no producer: everything"
        );
        assert_eq!(taken(None, 19), [60, 61, 62], "no graph: everything");
    }

    #[test]
    fn a_split_pulled_past_the_cap_is_told_for_the_pulled_one_only() {
        let pull = |producer: u64, splittable: bool| routing::Pull {
            producer,
            milestone: 28,
            task: 219,
            need: "Brief".to_string(),
            why: routing::Starving::Starved,
            splittable,
            through: None,
        };
        let pulled = pull(24, true);
        assert_eq!(
            pulled_event(Some(&pulled), 24),
            Some(Event::SplitPulled {
                milestone: 24,
                by_milestone: 28,
                by_task: 219,
                need: "Brief".to_string(),
                why: "starved".to_string(),
                through: None,
            })
        );
        assert_eq!(pulled_event(Some(&pulled), 27), None, "one pulled a tick");
        assert_eq!(
            pulled_event(None, 24),
            None,
            "a split under the cap is no pull"
        );
        // A stuck pull is said with why it pulls; one pulled through is not stuck.
        let stuck = routing::Pull {
            why: routing::Starving::HeldAtLanding,
            ..pull(18, false)
        };
        let lines = unpulled_lines(&[stuck.clone(), pull(24, true)]);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].starts_with(
                "watch: #18 would be split past the cap for #219 of #28 (Brief, held at landing)"
            ),
            "{}",
            lines[0]
        );
        let deeper = routing::Pull {
            through: Some(18),
            ..pull(10, true)
        };
        assert!(
            unpulled_lines(&[stuck, deeper]).is_empty(),
            "pulled through: not stuck"
        );
    }

    #[test]
    fn a_task_a_human_holds_that_holds_many_rules_is_said() {
        use harness_workflows::common::rule_waits::Waits;
        let task = |number: u64, labels: &[&str]| Issue {
            body: "## Architecture\n\nunit: data-capability\nsystem: campaign\nproduces: c.make"
                .to_string(),
            ..issue(number, labels, "data-capability")
        };
        let waits = |count: usize| {
            Waits(
                (0..count)
                    .map(|i| (format!("R{i}"), vec!["data-capability:c.make".to_string()]))
                    .collect(),
            )
        };
        let found = |labels: &[&str], count: usize| Snapshot {
            boards: vec![harness_workflows::dev_loop::data::board::Board {
                milestone: issue(18, &[labels::MILESTONE], "-"),
                tasks: vec![task(157, labels)],
                resuming: None,
            }],
            rule_waits: waits(count),
            ..snapshot(Vec::new(), None)
        };
        let said = |snapshot: &Snapshot| {
            let capture = Rc::new(Capture::default());
            let log = Logbook::new(
                Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
                harness_core::traces::Verbosity::Normal,
            );
            say_rules_held(snapshot, &log);
            capture.0.borrow().join("\n")
        };
        assert!(
            said(&found(&[labels::AGENT], 17))
                .contains("#157 has no harness:ready, holds 17 rules")
        );
        assert!(
            said(&found(&[labels::AGENT, labels::READY], 17)).is_empty(),
            "ready: the lanes take it"
        );
        assert!(
            said(&found(&[labels::AGENT], 4)).is_empty(),
            "under the bar"
        );
        assert_eq!(
            rules_held(&found(&[labels::AGENT, labels::READY], 17), 157),
            Some(17)
        );
        assert_eq!(
            rules_held(&found(&[labels::AGENT, labels::READY], 4), 157),
            None
        );
        assert_eq!(
            rules_held(&found(&[labels::AGENT, labels::READY], 17), 158),
            None
        );
    }
}
