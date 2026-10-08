//! From what was observed to what is drawn. Pure: `Observed` and the board
//! in, a `Snapshot` out, and `now` is an argument.
//!
//! The one judgement call here is **who is still working**. The harness
//! writes no "run over" line, so a run is live when the watch dispatched its
//! workflow and has not reported back, or when its folder was written to in
//! the last two minutes. `--demo` adds the most recent run regardless, so the
//! plant can be seen moving when nothing runs.

use harness_core::domain::Issue;
use harness_workflows::common::{branching, labels};

use crate::domain::blueprint::{self, Line};
use crate::domain::observe::{Observed, ObservedLine, ObservedRun};
use crate::domain::snapshot::{
    BoardView, Chimney, Employee, Factory, IssueStatus, IssueView, LastRun, LineView,
    MilestoneView, Project, RecentRun, Snapshot, StationState, StationView, Tokens, Version,
    Versions,
};
use crate::domain::traces::{self, Watch, summarize_costs};
use crate::ports::{BoardReading, LedgerRow};

/// A run that wrote in the last two minutes is live whatever the watch says.
const FRESH_SECS: u64 = 120;

/// A dispatched run that has not written for this long is over, whatever the
/// watch failed to say — a session has a thirty-minute deadline, a round a
/// few of them.
const IN_FLIGHT_MAX_SECS: u64 = 3 * 3600;

/// How many stops the picture carries.
const ERRORS_SHOWN: usize = 12;

/// How many ledger rows the picture carries verbatim.
const LEDGER_ROWS_SHOWN: usize = 24;

/// What `snapshot` needs.
pub struct Inputs<'a> {
    /// What the traces said.
    pub observed: &'a Observed,
    /// What GitHub said, if it was asked.
    pub board: Option<&'a BoardReading>,
    /// The project on the sign.
    pub project: &'a Project,
    /// The blueprint.
    pub lines: &'a [Line],
    /// Seconds since the epoch.
    pub now: i64,
    /// Show the most recent run live even if it is over.
    pub demo: bool,
}

/// `#62 Asset folder rule…` — how an issue is written on a badge.
fn named(item: &traces::Named) -> String {
    format!("#{} {}", item.number, item.title)
}

fn round(log: &traces::RunLog) -> Option<String> {
    log.round.map(|(n, total)| format!("{n}/{total}"))
}

/// The latest run's id, to tell it from the rest of the crew.
fn current_run_id(observed: Option<&ObservedLine>) -> String {
    observed
        .and_then(|l| l.latest.as_ref())
        .map(|r| r.run_id.clone())
        .unwrap_or_default()
}

/// Whether a run of `line` is still at work.
fn is_live(watch: &Watch, line: &str, run: &ObservedRun, demo_pick: bool) -> bool {
    // The process's own lock decides when the run has one: no guessing from
    // file ages, and no ghost of a run that stopped a minute ago.
    if let Some(alive) = run.alive {
        return alive || demo_pick;
    }
    let age = run.age_secs.unwrap_or(u64::MAX);
    let dispatched = watch
        .in_flight
        .as_ref()
        .is_some_and(|flight| flight.workflow == line);
    (dispatched && age < IN_FLIGHT_MAX_SECS) || age < FRESH_SECS || demo_pick
}

/// The stages the run has passed: what it logged, plus the two free stations
/// the dev loop never tags.
fn passed(run: &ObservedRun) -> Vec<String> {
    let mut passed = run.log.done.clone();
    passed.extend(run.log.skipped.iter().cloned());
    if run.log.task.is_some() {
        passed.push(blueprint::PICK.to_string());
    }
    if run.log.delivered.is_some() {
        passed.push(blueprint::DELIVER.to_string());
    }
    passed
}

/// Each station's state, and the one the product sits at.
fn stations(
    line: &Line,
    run: Option<&ObservedRun>,
    at_work: bool,
) -> (Vec<StationView>, Option<String>) {
    let passed = run.map(passed).unwrap_or_default();
    let skipped = run.map(|r| r.log.skipped.clone()).unwrap_or_default();
    let last_passed = line
        .stations
        .iter()
        .rposition(|s| s.stage.as_ref().is_some_and(|stage| passed.contains(stage)));
    let current = at_work
        .then(|| {
            line.stations
                .iter()
                .enumerate()
                .filter(|(i, s)| s.stage.is_some() && last_passed.is_none_or(|last| *i > last))
                .map(|(_, s)| s.id.clone())
                .next()
                .or_else(|| line.stations.last().map(|s| s.id.clone()))
        })
        .flatten();
    let views = line
        .stations
        .iter()
        .map(|station| {
            let state = match &station.stage {
                Some(stage) if skipped.contains(stage) => StationState::Skipped,
                Some(stage) if passed.contains(stage) => StationState::Done,
                _ if current.as_deref() == Some(station.id.as_str()) => StationState::Active,
                _ => StationState::Idle,
            };
            StationView {
                id: station.id.clone(),
                label: station.label.clone(),
                kind: station.kind,
                stage: station.stage.clone(),
                model: station.model.clone(),
                purpose: station.purpose.clone(),
                state,
            }
        })
        .collect();
    (views, current)
}

/// The model a live run is on: what its last prompt header says, else what
/// its stream announced, else what the station opens by default.
fn model_of(run: &ObservedRun, line: &Line, station: Option<&str>) -> Option<String> {
    run.turns
        .last()
        .and_then(|turn| traces::model_family(&turn.model))
        .map(ToString::to_string)
        .or_else(|| run.model_seen.clone())
        .or_else(|| {
            line.stations
                .iter()
                .find(|s| Some(s.id.as_str()) == station)
                .and_then(|s| s.model.clone())
        })
}

fn employee(
    line: &Line,
    run: &ObservedRun,
    station: Option<String>,
    at_work: bool,
    subject: Option<&str>,
    ledger: &[LedgerRow],
) -> Employee {
    let stage = line
        .stations
        .iter()
        .find(|s| Some(&s.id) == station.as_ref())
        .and_then(|s| s.stage.clone());
    // The run's own issue before the watch's subject: on a parallel watch,
    // the subject is one lane's, and this run may be on another.
    let who = run
        .log
        .task
        .as_ref()
        .map(|task| format!("#{}", task.number))
        .or_else(|| traces::issue_of_run(&run.run_id).map(|n| format!("#{n}")))
        .or_else(|| subject.map(ToString::to_string))
        .unwrap_or_else(|| run.run_id.clone());
    Employee {
        id: format!("{}/{}", line.id, run.run_id),
        name: agent_name(&line.title, &who),
        workflow: line.id.clone(),
        run_id: run.run_id.clone(),
        model: model_of(run, line, station.as_deref()),
        station,
        stage,
        task: run.log.task.as_ref().map(named),
        milestone: run.log.milestone.as_ref().map(named),
        round: round(&run.log),
        since: run.log.first_at.clone(),
        age_secs: run.age_secs,
        last_line: if run.session_tail.is_empty() {
            run.log.last_line.clone()
        } else {
            run.session_tail.clone()
        },
        active: at_work,
        tokens: tokens_of(ledger, &run.run_id),
    }
}

/// An agent's name: the issue it works first, so the plant reads as "who is
/// on what" — `#64 · Dev loop`; a run on no issue keeps the line first.
fn agent_name(title: &str, who: &str) -> String {
    if who.starts_with('#') {
        format!("{who} · {title}")
    } else {
        format!("{title} · {who}")
    }
}

/// A line's last runs, newest first, from their folder names and the
/// ledger rows they wrote — no run file is read.
fn recent_work(
    line: &Line,
    observed: Option<&ObservedLine>,
    ledger: &[LedgerRow],
    employees: &[Employee],
) -> Vec<RecentRun> {
    let Some(observed) = observed else {
        return Vec::new();
    };
    observed
        .run_ids
        .iter()
        .rev()
        .map(|run_id| {
            let mut rows: Vec<&LedgerRow> =
                ledger.iter().filter(|row| &row.run == run_id).collect();
            rows.sort_by(|a, b| a.when.cmp(&b.when));
            let mut stages: Vec<String> = Vec::new();
            for row in &rows {
                if !stages.contains(&row.stage) {
                    stages.push(row.stage.clone());
                }
            }
            let issue = traces::issue_of_run(run_id)
                .or_else(|| rows.iter().find_map(|row| row.task.parse().ok()));
            let who = issue.map_or_else(|| run_id.clone(), |n| format!("#{n}"));
            RecentRun {
                run_id: run_id.clone(),
                name: agent_name(&line.title, &who),
                issue,
                stages,
                tokens: tokens_of(ledger, run_id),
                active: employees
                    .iter()
                    .any(|e| e.workflow == line.id && &e.run_id == run_id),
            }
        })
        .collect()
}

/// What run `run_id` consumed, from the ledger rows it wrote; `None` when it
/// has written none yet.
fn tokens_of(ledger: &[LedgerRow], run_id: &str) -> Option<Tokens> {
    let mut sum = Tokens::default();
    for row in ledger.iter().filter(|row| row.run == run_id) {
        sum.input = sum.input.saturating_add(row.input.unwrap_or(0));
        sum.output = sum.output.saturating_add(row.output.unwrap_or(0));
        sum.cache_read = sum.cache_read.saturating_add(row.cache_read.unwrap_or(0));
        sum.cache_write = sum.cache_write.saturating_add(row.cache_write.unwrap_or(0));
        sum.stages = sum.stages.saturating_add(1);
    }
    if sum.stages == 0 {
        return None;
    }
    sum.total = sum
        .input
        .saturating_add(sum.output)
        .saturating_add(sum.cache_read)
        .saturating_add(sum.cache_write);
    Some(sum)
}

fn last_run(run: &ObservedRun) -> LastRun {
    LastRun {
        run_id: run.run_id.clone(),
        started_at: run.log.first_at.clone(),
        last_at: run.log.last_at.clone(),
        age_secs: run.age_secs,
        task: run.log.task.as_ref().map(named),
        round: round(&run.log),
        last_line: run.log.last_line.clone(),
        warning: run.log.warning.clone(),
    }
}

/// The line whose latest run wrote most recently — the one `--demo` animates.
fn freshest(observed: &Observed) -> Option<&str> {
    observed
        .lines
        .iter()
        .filter_map(|line| {
            line.latest
                .as_ref()
                .and_then(|run| run.age_secs)
                .map(|age| (age, line.workflow.as_str()))
        })
        .min_by_key(|(age, _)| *age)
        .map(|(_, workflow)| workflow)
}

fn lines_and_employees(inputs: &Inputs<'_>) -> (Vec<LineView>, Vec<Employee>) {
    let watch = &inputs.observed.watch;
    let demo_line = inputs.demo.then(|| freshest(inputs.observed)).flatten();
    let mut lines = Vec::new();
    let mut employees = Vec::new();
    for line in inputs.lines {
        let observed: Option<&ObservedLine> =
            inputs.observed.lines.iter().find(|l| l.workflow == line.id);
        let run = observed.and_then(|l| l.latest.as_ref());
        let demo_pick = demo_line == Some(line.id.as_str());
        let live = run.is_some_and(|run| is_live(watch, &line.id, run, demo_pick));
        let (views, current) = stations(line, run, live);
        // One employee per run at work: a parallel watch runs a line on
        // several lanes, and each lane is somebody standing at a station.
        let subject = watch
            .in_flight
            .as_ref()
            .filter(|flight| flight.workflow == line.id)
            .and_then(|flight| flight.subject.as_deref());
        let mut crew: Vec<&ObservedRun> = observed
            .map(|l| {
                l.recent
                    .iter()
                    .filter(|r| {
                        let is_latest = run.is_some_and(|latest| latest.run_id == r.run_id);
                        is_live(watch, &line.id, r, demo_pick && is_latest)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(run) = run
            && live
            && !crew.iter().any(|r| r.run_id == run.run_id)
        {
            crew.push(run);
        }
        for run in crew {
            let at = if run.run_id == current_run_id(observed) {
                current.clone()
            } else {
                stations(line, Some(run), true).1
            };
            employees.push(employee(
                line,
                run,
                at,
                true,
                subject,
                &inputs.observed.ledger,
            ));
        }
        let recent_work = recent_work(line, observed, &inputs.observed.ledger, &employees);
        lines.push(LineView {
            recent_work,
            id: line.id.clone(),
            title: line.title.clone(),
            trigger: line.trigger.clone(),
            purpose: line.purpose.clone(),
            stations: views,
            runs: observed.map_or(0, |l| l.runs),
            last_run: run.map(last_run),
            active: live,
        });
    }
    (lines, employees)
}

/// Dollars on the stages that open `model` by default.
fn usd_by_model(ledger: &[LedgerRow], lines: &[Line], model: &str) -> f64 {
    ledger
        .iter()
        .filter(|row| blueprint::model_of_stage(lines, &row.stage).as_deref() == Some(model))
        .map(|row| row.cost_usd.unwrap_or(0.0))
        .sum()
}

fn chimneys(inputs: &Inputs<'_>, employees: &[Employee]) -> Vec<Chimney> {
    let mut models = blueprint::models();
    for run in inputs
        .observed
        .lines
        .iter()
        .filter_map(|l| l.latest.as_ref())
    {
        let seen = run
            .turns
            .iter()
            .filter_map(|turn| traces::model_family(&turn.model))
            .map(ToString::to_string)
            .chain(run.model_seen.clone());
        for model in seen {
            if !models.contains(&model) {
                models.push(model);
            }
        }
    }
    models
        .into_iter()
        .map(|model| {
            let runs = employees
                .iter()
                .filter(|e| e.active && e.model.as_deref() == Some(model.as_str()))
                .count();
            Chimney {
                smoking: runs > 0,
                runs: u32::try_from(runs).unwrap_or(u32::MAX),
                usd: usd_by_model(&inputs.observed.ledger, inputs.lines, &model),
                model,
            }
        })
        .collect()
}

/// The watch is believed to be polling: it ticked within three intervals, or
/// it dispatched something that is still plausibly running.
fn watching(watch: &Watch, now: i64) -> bool {
    let interval = i64::try_from(watch.interval.unwrap_or(30)).unwrap_or(30);
    let recent_tick = watch
        .last_tick_at
        .as_deref()
        .and_then(traces::unix)
        .is_some_and(|at| now - at <= interval * 3 + 30);
    let dispatched = watch
        .in_flight
        .as_ref()
        .and_then(|flight| traces::unix(&flight.since))
        .is_some_and(|at| now - at <= i64::try_from(IN_FLIGHT_MAX_SECS).unwrap_or(i64::MAX));
    recent_tick || dispatched
}

fn issue_url(project: &Project, number: u64) -> String {
    if project.url.is_empty() {
        String::new()
    } else {
        format!("{}/issues/{number}", project.url)
    }
}

fn tree_url(project: &Project, branch: &str) -> String {
    if project.url.is_empty() {
        String::new()
    } else {
        format!("{}/tree/{branch}", project.url)
    }
}

/// Where an issue stands, from its labels and blockers.
fn status_of(issue: &Issue) -> IssueStatus {
    if issue.is_closed() {
        IssueStatus::Done
    } else if issue.has(labels::WAITING_MERGE) {
        IssueStatus::Delivered
    } else if issue.has(labels::HUMAN) || issue.has(labels::NEEDS_DECISION) {
        IssueStatus::Human
    } else if issue
        .blocked_by
        .iter()
        .any(|blocker| blocker.is_open() && !blocker.has(labels::WAITING_MERGE))
    {
        IssueStatus::Blocked
    } else if issue.has(labels::READY) {
        IssueStatus::Ready
    } else {
        IssueStatus::Todo
    }
}

fn kind_of(issue: &Issue) -> &'static str {
    if issue.has(labels::ROADMAP) {
        "roadmap"
    } else if issue.has(labels::MILESTONE) {
        "milestone"
    } else if issue.has(labels::HUMAN) {
        "human"
    } else if issue.has(labels::AGENT) {
        "agent"
    } else {
        "issue"
    }
}

fn issue_view(issue: &Issue, project: &Project) -> IssueView {
    IssueView {
        number: issue.number,
        title: issue.title.clone(),
        state: issue.state.clone(),
        labels: issue.labels.clone(),
        url: issue_url(project, issue.number),
        kind: kind_of(issue),
        status: status_of(issue),
    }
}

fn board_view(reading: &BoardReading, project: &Project) -> BoardView {
    let milestones: Vec<MilestoneView> = reading
        .milestones
        .iter()
        .map(|milestone| {
            let branch =
                branching::milestone_branch(milestone.issue.number, &milestone.issue.title);
            let tasks: Vec<IssueView> = milestone
                .tasks
                .iter()
                .map(|task| issue_view(task, project))
                .collect();
            let done = tasks
                .iter()
                .filter(|t| matches!(t.status, IssueStatus::Done | IssueStatus::Delivered))
                .count();
            MilestoneView {
                issue: issue_view(&milestone.issue, project),
                branch_url: tree_url(project, &branch),
                branch,
                done: u32::try_from(done).unwrap_or(u32::MAX),
                total: u32::try_from(tasks.len()).unwrap_or(u32::MAX),
                tasks,
            }
        })
        .collect();
    let needs_human = milestones
        .iter()
        .flat_map(|m| m.tasks.iter())
        .filter(|t| t.status == IssueStatus::Human)
        .cloned()
        .collect();
    BoardView {
        roadmap: reading
            .roadmap
            .iter()
            .map(|issue| issue_view(issue, project))
            .collect(),
        milestones,
        needs_human,
    }
}

fn versions(project: &Project, board: Option<&BoardReading>) -> Versions {
    let milestones = board
        .map(|reading| {
            reading
                .milestones
                .iter()
                .filter(|m| m.issue.is_open())
                .map(|m| {
                    let branch = branching::milestone_branch(m.issue.number, &m.issue.title);
                    Version {
                        url: tree_url(project, &branch),
                        name: branch,
                        label: m.issue.title.clone(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Versions {
        repo_url: project.url.clone(),
        main: Version {
            name: "main".to_string(),
            url: tree_url(project, "main"),
            label: "the current version".to_string(),
        },
        integration: Version {
            name: project.integration_branch.clone(),
            url: tree_url(project, &project.integration_branch),
            label: "what the agents integrated".to_string(),
        },
        milestones,
    }
}

/// Assembles the picture. `at` is left empty: the caller stamps it once it
/// has decided the picture changed.
#[must_use]
pub fn snapshot(inputs: &Inputs<'_>) -> Snapshot {
    let (lines, employees) = lines_and_employees(inputs);
    let watch = &inputs.observed.watch;
    Snapshot {
        at: String::new(),
        project: inputs.project.clone(),
        factory: Factory {
            chimneys: chimneys(inputs, &employees),
            watching: watching(watch, inputs.now),
            last_tick_at: watch.last_tick_at.clone(),
            in_flight: watch.in_flight.clone(),
            saw: watch.saw.clone(),
            idle: employees.iter().all(|e| !e.active),
        },
        employees,
        lines,
        rooms: blueprint::ROOMS.to_vec(),
        board: inputs.board.map(|b| board_view(b, inputs.project)),
        versions: versions(inputs.project, inputs.board),
        costs: summarize_costs(&inputs.observed.ledger, LEDGER_ROWS_SHOWN),
        quota: inputs.observed.quota.clone(),
        errors: inputs
            .observed
            .errors
            .iter()
            .rev()
            .take(ERRORS_SHOWN)
            .cloned()
            .collect(),
        recent: watch.recent.clone(),
        demo: inputs.demo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::observe::ObservedRun;
    use crate::domain::traces::{InFlight, parse_run_log};
    use crate::ports::Milestone;

    fn project() -> Project {
        Project {
            name: "dnd_helper".to_string(),
            slug: "Laucans/dnd_helper".to_string(),
            url: "https://github.com/Laucans/dnd_helper".to_string(),
            integration_branch: "main_agent".to_string(),
        }
    }

    fn dev_run(log: &str, age: u64) -> ObservedRun {
        ObservedRun {
            run_id: "20261006-202608".to_string(),
            log: parse_run_log(log, &[]),
            turns: vec![],
            age_secs: Some(age),
            model_seen: None,
            session_tail: String::new(),
            alive: None,
        }
    }

    fn observed_with(workflow: &str, run: ObservedRun, watch: Watch) -> Observed {
        Observed {
            watch,
            lines: vec![ObservedLine {
                workflow: workflow.to_string(),
                runs: 1,
                latest: Some(run),
                recent: Vec::new(),
                run_ids: Vec::new(),
            }],
            ledger: vec![],
            errors: vec![],
            quota: None,
        }
    }

    fn in_flight(workflow: &str, since: &str) -> Watch {
        Watch {
            in_flight: Some(InFlight {
                route: String::new(),
                workflow: workflow.to_string(),
                subject: Some("milestone 17".to_string()),
                since: since.to_string(),
            }),
            last_tick_at: Some(since.to_string()),
            interval: Some(30),
            ..Watch::default()
        }
    }

    const MID_RUN: &str = "\
[2026-10-06T20:26:10Z] pipeline: technical-refinement(opus/high) -> code(sonnet/high) -> create-test(sonnet/high)
[2026-10-06T20:26:16Z] --- round 1/3 ---
[2026-10-06T20:26:21Z] task #62: Asset folder rule [auto]
[2026-10-06T20:28:32Z] [technical-refinement] AGENT_LOOP_OK: sections written
";

    fn assemble(observed: &Observed, demo: bool) -> Snapshot {
        let lines = blueprint::lines();
        snapshot(&Inputs {
            observed,
            board: None,
            project: &project(),
            lines: &lines,
            now: traces::unix("2026-10-06T20:30:00Z").expect("clock"),
            demo,
        })
    }

    #[test]
    fn a_dispatched_run_puts_its_employee_at_the_stage_after_the_last_one_done() {
        let observed = observed_with(
            "agent-loop",
            dev_run(MID_RUN, 40),
            in_flight("agent-loop", "2026-10-06T20:26:00Z"),
        );
        let snap = assemble(&observed, false);
        assert_eq!(snap.employees.len(), 1);
        let worker = &snap.employees[0];
        assert_eq!(worker.station.as_deref(), Some("code"));
        assert_eq!(worker.stage.as_deref(), Some("code"));
        assert_eq!(worker.model.as_deref(), Some("sonnet"));
        assert_eq!(worker.task.as_deref(), Some("#62 Asset folder rule"));
        assert_eq!(worker.name, "#62 · Dev loop");
        assert!(worker.active);

        let dev = snap
            .lines
            .iter()
            .find(|l| l.id == "agent-loop")
            .expect("line");
        let state = |id: &str| dev.stations.iter().find(|s| s.id == id).expect(id).state;
        assert_eq!(state("pick"), StationState::Done);
        assert_eq!(state("technical-refinement"), StationState::Done);
        assert_eq!(state("code"), StationState::Active);
        assert_eq!(state("create-test"), StationState::Idle);
        assert_eq!(state("code.pre"), StationState::Idle);
        assert!(!snap.factory.idle);
        assert!(snap.factory.watching);
    }

    #[test]
    fn the_sonnet_chimney_smokes_when_the_worker_is_on_code() {
        let observed = observed_with(
            "agent-loop",
            dev_run(MID_RUN, 40),
            in_flight("agent-loop", "2026-10-06T20:26:00Z"),
        );
        let snap = assemble(&observed, false);
        let smoke = |model: &str| {
            snap.factory
                .chimneys
                .iter()
                .find(|c| c.model == model)
                .expect(model)
                .smoking
        };
        assert!(smoke("sonnet"));
        assert!(!smoke("opus"));
    }

    #[test]
    fn a_stale_run_with_no_dispatch_is_nobody_unless_demo_says_so() {
        let observed = observed_with("agent-loop", dev_run(MID_RUN, 5_000), Watch::default());
        let quiet = assemble(&observed, false);
        assert_eq!(
            quiet.employees,
            [] as [crate::domain::snapshot::Employee; 0]
        );
        assert!(quiet.factory.idle);
        assert!(!quiet.factory.watching);
        assert!(quiet.factory.chimneys.iter().all(|c| !c.smoking));

        let demo = assemble(&observed, true);
        assert_eq!(demo.employees.len(), 1);
        assert!(demo.demo);
    }

    #[test]
    fn a_fresh_write_is_live_even_when_the_watch_said_nothing() {
        let observed = observed_with("agent-loop", dev_run(MID_RUN, 30), Watch::default());
        assert_eq!(assemble(&observed, false).employees.len(), 1);
    }

    #[test]
    fn a_dispatch_the_watch_forgot_to_close_dies_after_three_hours() {
        let observed = observed_with(
            "agent-loop",
            dev_run(MID_RUN, 4 * 3600),
            in_flight("agent-loop", "2026-10-06T10:00:00Z"),
        );
        assert_eq!(
            assemble(&observed, false).employees,
            [] as [crate::domain::snapshot::Employee; 0]
        );
    }

    #[test]
    fn a_skipped_stage_reads_as_skipped_and_the_product_moves_past_it() {
        let text = "[2026-10-06T20:26:16Z] --- round 1/3 ---\n\
                    [2026-10-06T20:26:21Z] task #64: fvtt pack adapter [auto]\n\
                    [2026-10-06T20:26:22Z] issue #64 already carries harness:tech-written — skipping the technical refinement\n";
        let known = vec!["technical-refinement".to_string()];
        let run = ObservedRun {
            log: parse_run_log(text, &known),
            ..dev_run(text, 10)
        };
        let snap = assemble(&observed_with("agent-loop", run, Watch::default()), false);
        let dev = snap
            .lines
            .iter()
            .find(|l| l.id == "agent-loop")
            .expect("line");
        let state = |id: &str| dev.stations.iter().find(|s| s.id == id).expect(id).state;
        assert_eq!(state("technical-refinement"), StationState::Skipped);
        assert_eq!(state("code"), StationState::Active);
    }

    fn issue(number: u64, title: &str, state: &str, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: title.to_string(),
            state: state.to_string(),
            labels: labels.iter().map(ToString::to_string).collect(),
            body: String::new(),
            blocked_by: vec![],
        }
    }

    #[test]
    fn the_board_sorts_tasks_by_what_their_labels_say() {
        let mut blocked = issue(66, "proof", "open", &[labels::AGENT]);
        blocked.blocked_by = vec![issue(65, "export", "open", &[labels::AGENT])];
        let reading = BoardReading {
            slug: "Laucans/dnd_helper".to_string(),
            roadmap: vec![issue(12, "v1", "open", &[labels::ROADMAP, labels::READY])],
            milestones: vec![Milestone {
                issue: issue(17, "Dernier kilomètre", "open", &[labels::MILESTONE]),
                tasks: vec![
                    issue(61, "actor", "open", &[labels::AGENT, labels::WAITING_MERGE]),
                    issue(62, "scene", "closed", &[labels::AGENT]),
                    issue(65, "export", "open", &[labels::AGENT, labels::READY]),
                    blocked,
                    issue(67, "import in Foundry", "open", &[labels::HUMAN]),
                ],
            }],
        };
        let observed = Observed::default();
        let lines = blueprint::lines();
        let snap = snapshot(&Inputs {
            observed: &observed,
            board: Some(&reading),
            project: &project(),
            lines: &lines,
            now: 0,
            demo: false,
        });
        let board = snap.board.expect("board");
        assert_eq!(board.roadmap[0].kind, "roadmap");
        let milestone = &board.milestones[0];
        assert_eq!(milestone.branch, "milestone/17-dernier-kilom-tre");
        assert_eq!(
            milestone.branch_url,
            "https://github.com/Laucans/dnd_helper/tree/milestone/17-dernier-kilom-tre"
        );
        assert_eq!((milestone.done, milestone.total), (2, 5));
        let status = |n: u64| {
            milestone
                .tasks
                .iter()
                .find(|t| t.number == n)
                .expect("task")
                .status
        };
        assert_eq!(status(61), IssueStatus::Delivered);
        assert_eq!(status(62), IssueStatus::Done);
        assert_eq!(status(65), IssueStatus::Ready);
        assert_eq!(status(66), IssueStatus::Blocked);
        assert_eq!(status(67), IssueStatus::Human);
        assert_eq!(board.needs_human.len(), 1);
        assert_eq!(snap.versions.milestones.len(), 1);
        assert_eq!(snap.versions.integration.name, "main_agent");
        assert_eq!(
            snap.versions.main.url,
            "https://github.com/Laucans/dnd_helper/tree/main"
        );
    }

    #[test]
    fn without_a_repository_url_the_links_stay_empty_rather_than_broken() {
        let project = Project {
            url: String::new(),
            ..project()
        };
        let observed = Observed::default();
        let lines = blueprint::lines();
        let snap = snapshot(&Inputs {
            observed: &observed,
            board: None,
            project: &project,
            lines: &lines,
            now: 0,
            demo: false,
        });
        assert_eq!(snap.versions.main.url, "");
        assert!(snap.board.is_none());
        assert_eq!(snap.lines.len(), lines.len());
        assert_eq!(snap.rooms.len(), 6);
    }

    fn row(run: &str, input: u64, output: u64, cache_read: u64) -> LedgerRow {
        LedgerRow {
            when: String::new(),
            run: run.to_string(),
            round: String::new(),
            task: String::new(),
            stage: "code".to_string(),
            cost_usd: None,
            turns: None,
            duration_ms: None,
            input: Some(input),
            output: Some(output),
            cache_read: Some(cache_read),
            cache_write: None,
            outcome: "ok".to_string(),
        }
    }

    #[test]
    fn a_dead_run_is_no_one_at_work_however_fresh_and_a_live_one_is() {
        let watch = Watch::default();
        let mut run = dev_run(MID_RUN, 5);
        run.alive = Some(false);
        assert!(!is_live(&watch, "agent-loop", &run, false));
        run.alive = Some(true);
        run.age_secs = Some(10_000);
        assert!(is_live(&watch, "agent-loop", &run, false));
    }

    #[test]
    fn a_run_s_tokens_are_the_sum_of_its_own_ledger_rows_only() {
        let ledger = vec![
            row("20261008-152126-64", 10, 20, 300),
            row("20261008-152126-64", 1, 2, 30),
            row("20261008-152126-65", 1_000, 1_000, 1_000),
        ];
        let tokens = tokens_of(&ledger, "20261008-152126-64").expect("two rows");
        assert_eq!(tokens.input, 11);
        assert_eq!(tokens.output, 22);
        assert_eq!(tokens.cache_read, 330);
        assert_eq!(tokens.total, 363);
        assert_eq!(tokens.stages, 2);
        assert!(tokens_of(&ledger, "20261008-000000").is_none());
    }

    #[test]
    fn every_fresh_run_of_a_line_is_an_employee_of_its_own() {
        // Two lanes on the dev loop: two workers, each at their own station,
        // and the line's stations read from the latest run.
        let latest = dev_run(MID_RUN, 12);
        let mut other = dev_run("[2026-10-06T20:26:21Z] task #63: Other [auto]\n", 40);
        other.run_id = "20261006-202000".to_string();
        let mut observed = observed_with(
            "agent-loop",
            latest.clone(),
            in_flight("agent-loop", "2026-10-06T20:26:00Z"),
        );
        observed.lines[0].recent = vec![other, latest];
        let snap = assemble(&observed, false);
        assert_eq!(snap.employees.len(), 2);
        let ids: Vec<&str> = snap.employees.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"agent-loop/20261006-202000"));
        assert!(ids.contains(&"agent-loop/20261006-202608"));
        assert!(snap.employees.iter().all(|e| e.active));
    }
}
