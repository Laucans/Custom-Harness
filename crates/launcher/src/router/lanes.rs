//! The lanes of a parallel watch: every free lane is given a runnable task
//! of the current milestone, then a milestone to split or an issue waiting on
//! its business refinement — each a `harness` child in a checkout of its
//! own. Nothing here runs in this process.

use std::path::Path;

use harness_core::domain::{Halt, Issue};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook};
use harness_workflows::common::routing::{self, Snapshot};
use harness_workflows::common::{branching, labels};
use harness_workflows::dev_loop::data::tasks;
use harness_workflows::main_agent_merge::data::merge_train;

use crate::cli::WatchArgs;
use crate::dispatch::lanes::Lanes;

use super::routes::refinement_branch;
use super::with_split_blockers;

/// Gives every free lane work, whatever the route: the runnable tasks of the
/// current milestone first, then milestones to split and issues waiting on
/// their business refinement in the lanes left
/// ([`routing::lane_work`]).
pub(super) async fn fill_lanes(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    found: &Snapshot,
    log: &Logbook,
    lanes: &mut Lanes,
) -> Option<(&'static str, Halt)> {
    let work = routing::lane_work(found);
    // The critical path's milestone first: the ahead work only takes the
    // lanes it leaves free.
    for milestone in &work.tasks_of {
        if lanes.free_slot().is_none() {
            break;
        }
        if let Some(halt) = run_lanes(args, here, gh, *milestone, found, lanes, log).await {
            return Some(("dev_loop", halt));
        }
    }
    if work.preparation
        && lanes.free_slot().is_some()
        && let Some(halt) = run_router_lanes(args, here, gh, found, lanes, log).await
    {
        return Some(("router_lanes", halt));
    }
    None
}

/// Fills the free lanes with the router's own work, splits first: every
/// milestone ready to split, then every issue waiting on its **business**
/// refinement, that no lane is on — each a `harness split <n>` or `harness
/// refine <n>` child in a checkout of its own (`router-lane-<k>`), up to
/// `--parallel` at once. Two splits cut two different milestones, two
/// refinements write two different bodies, and a milestone is never split
/// while its refinement is pending: nothing here waits on anything else.
pub(super) async fn run_router_lanes(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    found: &Snapshot,
    lanes: &mut Lanes,
    log: &Logbook,
) -> Option<Halt> {
    let listed = async {
        let milestones =
            with_split_blockers(gh, gh.issues_labelled(labels::MILESTONE, "open").await?).await?;
        let refining = gh.issues_labelled(labels::REFINEMENT, "open").await?;
        Ok::<_, Halt>((milestones, refining))
    };
    let (milestones, refining) = match listed.await {
        Ok(found) => found,
        Err(halt) => {
            log.warn(&format!(
                "watch: router lanes -> cannot list the work: {}",
                halt.reason()
            ));
            return Some(halt);
        }
    };
    let running = lanes.running();
    let splits = routing::splits(&milestones, found.graph.as_ref())
        .into_iter()
        .filter(|number| !running.contains(number));
    let mut refining: Vec<&Issue> = refining
        .iter()
        .filter(|issue| issue.is_open() && !running.contains(&issue.number))
        .collect();
    refining.sort_unstable_by_key(|issue| issue.number);
    let mut refines: Vec<(u64, String)> = Vec::with_capacity(refining.len());
    for issue in refining {
        let branch = refinement_branch(gh, &args.branch, issue).await;
        refines.push((issue.number, branch));
    }
    let work = splits
        .map(|number| ("split", number, args.branch.clone()))
        .chain(
            refines
                .into_iter()
                .map(|(number, branch)| ("refine", number, branch)),
        );
    let mut started = 0usize;
    for (what, number, branch) in work {
        let Some(slot) = lanes.free_slot() else {
            log.say("watch: router lanes -> every lane is busy");
            break;
        };
        let Some(command) = router_lane_command(args, here, what, number, &branch, slot) else {
            log.error("watch: router lanes -> cannot find this executable");
            return None;
        };
        match lanes.spawn(slot, number, command, here) {
            Ok(pid) => {
                started += 1;
                log.event(&Event::PreparationTaken {
                    lane: u32::try_from(slot).unwrap_or(u32::MAX),
                    what: what.to_string(),
                    issue: number,
                    pid,
                });
            }
            Err(e) => log.event(&Event::LaneNotStarted {
                what: Some(what.to_string()),
                issue: number,
                reason: e.to_string(),
            }),
        }
    }
    if started == 0 && lanes.free_slot().is_some() {
        log.say("watch: router lanes -> every split and refinement is already on a lane");
    }
    None
}

/// `harness <what> <number>` on lane `slot`, in its own checkout.
fn router_lane_command(
    args: &WatchArgs,
    here: &Path,
    what: &str,
    number: u64,
    branch: &str,
    slot: usize,
) -> Option<tokio::process::Command> {
    let mut command = tokio::process::Command::new(std::env::current_exe().ok()?);
    command
        .current_dir(here)
        .arg(what)
        .arg(number.to_string())
        .arg("--use-workspace")
        .arg(format!("router-lane-{slot}"))
        .arg("--target-repo-url")
        .arg(&args.target_repo_url)
        .arg("--branch")
        .arg(branch)
        .arg("--permission-mode")
        .arg(&args.permission_mode);
    if args.dry_run {
        command.arg("--dry-run");
    }
    Some(command)
}

/// What a parked task is compared on: its body, its labels and the blockers
/// still open on it. Answering the stop — editing the SPEC, a label — moves
/// it; so does a blocker closing: `ForgeDesMondes`' #247 and #248, taken in
/// the second before their `blocked_by` were posted, stopped on "cannot run:
/// blocked" and would have stayed parked after #244 and #246 closed.
fn issue_version(issue: &Issue) -> String {
    let mut labels = issue.labels.clone();
    labels.sort_unstable();
    let mut open_blockers: Vec<u64> = issue
        .blocked_by
        .iter()
        .filter(|b| b.is_open())
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

/// Runs the dev loop pointed at this milestone's own branch, derived from
/// its number and title (`common::branching`) — never the fixed
/// `--branch`/`INTEGRATION_BRANCH` a direct `harness` invocation would use.
/// Gives every runnable task of the milestone that no lane is on to a free
/// lane — a `harness --task <n>` child in its own workspace — up to
/// `--parallel` at once. Nothing runs in this process.
pub(super) async fn run_lanes(
    args: &WatchArgs,
    here: &Path,
    gh: &dyn GitHub,
    milestone: u64,
    found: &Snapshot,
    lanes: &mut Lanes,
    log: &Logbook,
) -> Option<Halt> {
    let Some(board) = found
        .boards
        .iter()
        .find(|b| b.milestone.number == milestone)
    else {
        log.warn(&format!(
            "watch: lanes -> no board read for milestone #{milestone} this tick"
        ));
        return None;
    };
    let branch = branching::milestone_branch(milestone, &board.milestone.title);
    let running = lanes.running();
    let mut parked = Vec::new();
    let mut candidates: Vec<(u64, String)> = Vec::new();
    // The runnable tasks not on a lane, those changing an aggregate another
    // holds left for a later tick — and, while the producers of what this
    // milestone reads still build, the read side only: a contract is mocked
    // until built, a table is not.
    let startable = startable_now(&board.tasks, &running, milestone, found, log);
    for task in startable {
        let version = issue_version(task);
        if lanes.is_parked(task.number, &version) {
            parked.push(task.number);
        } else {
            candidates.push((task.number, version));
        }
    }
    if candidates.is_empty() {
        let list = |tasks: &[u64]| {
            tasks
                .iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        log.say(&format!(
            "watch: lanes -> every runnable task is already on a lane ({}){}",
            list(&running),
            if parked.is_empty() {
                String::new()
            } else {
                format!(" or parked until its issue changes ({})", list(&parked))
            }
        ));
        return None;
    }
    if !args.dry_run {
        branching::catch_up(gh, &branch, &args.branch, log).await;
    }
    for (number, version) in candidates {
        let Some(slot) = lanes.free_slot() else {
            log.say("watch: lanes -> every lane is busy");
            break;
        };
        let mut command = tokio::process::Command::new(match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                log.error(&format!("watch: lanes -> cannot find this executable: {e}"));
                return None;
            }
        });
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
        if args.force_reset {
            command.arg("--force-reset");
        }
        if args.dry_run {
            command.arg("--dry-run");
        }
        if args.no_human_gate {
            command.arg("--no-human-gate");
        }
        lanes.taken(number, version);
        match lanes.spawn(slot, number, command, here) {
            Ok(pid) => log.event(&Event::LaneTaken {
                lane: u32::try_from(slot).unwrap_or(u32::MAX),
                task: number,
                branch: branch.clone(),
                pid,
            }),
            Err(e) => log.event(&Event::LaneNotStarted {
                what: None,
                issue: number,
                reason: e.to_string(),
            }),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
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
            pr_to_fix: None,
            pr_to_review: None,
            task_to_merge: None,
            ready_to_merge: None,
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
