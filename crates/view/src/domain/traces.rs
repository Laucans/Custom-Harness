//! What the trace files say, read as values.
//!
//! Pure: every function takes text and returns a struct, so each rendering
//! decision tests against a string. The formats are the ones the launcher
//! writes — `watch.log` (one line per tick), a run's `run.log`, the headers of
//! its `prompts.md`, the first event of its `stream.jsonl` — and the cost
//! ledger as the port hands it over.
//!
//! The view is a **reader of what the harness leaves behind**: nothing here
//! is a contract the harness promised, so every parser answers "unknown"
//! rather than failing when a line does not match.

use serde::Serialize;

use crate::domain::journal::{self, Journal};
use crate::ports::LedgerRow;

/// A journal line split at its clock: `[2026-10-07T12:50:44Z] rest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamped<'a> {
    /// The clock, as written.
    pub at: &'a str,
    /// Everything after it.
    pub rest: &'a str,
}

/// A clock as the sinks write it: `2026-10-07T12:50:44Z`, twenty characters.
fn looks_like_clock(text: &str) -> bool {
    text.len() == 20
        && text.ends_with('Z')
        && text.as_bytes().get(10) == Some(&b'T')
        && text.chars().filter(char::is_ascii_digit).count() == 14
}

/// Splits a journal line at its clock, or `None` when it carries none — a
/// continuation line of a multi-line warning, or a `[stage]` tag, which also
/// opens with a bracket.
#[must_use]
pub fn stamped(line: &str) -> Option<Stamped<'_>> {
    let inner = line.strip_prefix('[')?;
    let end = inner.find(']')?;
    let at = inner.get(..end)?;
    if !looks_like_clock(at) {
        return None;
    }
    let rest = inner.get(end + 1..)?;
    Some(Stamped {
        at,
        rest: rest.strip_prefix(' ').unwrap_or(rest),
    })
}

/// The clock as seconds since the epoch, or `None` if it does not parse.
#[must_use]
pub fn unix(at: &str) -> Option<i64> {
    at.parse::<jiff::Timestamp>()
        .ok()
        .map(jiff::Timestamp::as_second)
}

/// A run id as the launcher names a run folder: `20261006-202608` — eight
/// digits, a dash, six digits — optionally followed by a dash and the issue a
/// lane ran (`20261006-202608-17`). What tells a run folder from a ledger or a
/// loose file in the same directory, and what a URL segment must look like
/// before the server opens anything under it.
#[must_use]
pub fn is_run_id(name: &str) -> bool {
    let (clock, issue) = match name.get(15..) {
        Some("") | None => (name, None),
        Some(rest) => (name.get(..15).unwrap_or_default(), rest.strip_prefix('-')),
    };
    let clock_ok = clock.len() == 15
        && clock.as_bytes().get(8) == Some(&b'-')
        && clock
            .chars()
            .enumerate()
            .all(|(i, c)| i == 8 || c.is_ascii_digit());
    let issue_ok = issue.is_none_or(|digits| {
        !digits.is_empty() && digits.len() <= 10 && digits.chars().all(|c| c.is_ascii_digit())
    });
    clock_ok && issue_ok && !(name.len() > 15 && issue.is_none())
}

/// The log folder a route's dispatch writes into, or `None` for a route the
/// view does not know.
#[must_use]
pub fn workflow_of_route(route: &str) -> Option<&'static str> {
    let name = route
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or_default();
    match name {
        "DevLoop" => Some("agent-loop"),
        "Refinement" | "TechRefinement" => Some("refinement"),
        "Split" => Some("split"),
        "Planner" => Some("planner"),
        "PrReview" => Some("pr-review"),
        "PrFix" => Some("pr-fix"),
        "MergeMainAgent" => Some("main-agent-merge"),
        "MergeIntoMilestone" => Some("milestone-merge"),
        _ => None,
    }
}

/// How long after its trigger a run may start and still be the trigger's. A
/// run opens its log first thing — before it mounts anything — so a few
/// seconds are the norm; wider, and a trigger whose run never began would be
/// handed the next trigger's.
const RUN_STARTS_WITHIN_SECS: i64 = 60;

/// The run a trigger started: of `runs` (folder names), the first one that
/// began at most a few seconds before `at` and at most
/// [`RUN_STARTS_WITHIN_SECS`] after it — for `issue` when the run names one.
#[must_use]
pub fn run_of(runs: &[String], issue: Option<u64>, at: &str) -> Option<String> {
    let at = unix(at)?;
    let clock_of = |run: &str| -> Option<i64> {
        let c = run.get(..15)?;
        unix(&format!(
            "{}-{}-{}T{}:{}:{}Z",
            c.get(..4)?,
            c.get(4..6)?,
            c.get(6..8)?,
            c.get(9..11)?,
            c.get(11..13)?,
            c.get(13..15)?
        ))
    };
    let issue_of = |run: &str| -> Option<u64> { run.get(16..)?.parse().ok() };
    runs.iter()
        .filter(|run| is_run_id(run))
        .filter(|run| match (issue, issue_of(run)) {
            (Some(wanted), Some(named)) => wanted == named,
            (Some(_), None) | (None, _) => true,
        })
        .filter_map(|run| clock_of(run).map(|clock| (clock, run)))
        .filter(|(clock, _)| *clock >= at - 5 && *clock <= at + RUN_STARTS_WITHIN_SECS)
        .min_by_key(|(clock, _)| *clock)
        .map(|(_, run)| run.clone())
}

/// What a route is about — `milestone 17`, `issue 66`, `pr 71` — read from
/// its first field.
#[must_use]
pub fn route_subject(route: &str) -> Option<String> {
    let (_, inner) = route.split_once('{')?;
    let (key, value) = inner.trim_end_matches('}').trim().split_once(':')?;
    let value = value.split(',').next()?.trim().trim_matches('"');
    Some(format!("{} {value}", key.trim()))
}

/// A route the watch dispatched and has not reported back on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InFlight {
    /// The route, as `watch.log` printed it.
    pub route: String,
    /// The log folder its run writes into.
    pub workflow: String,
    /// `milestone 17`, `issue 66`, … when the route names one.
    pub subject: Option<String>,
    /// When it was dispatched.
    pub since: String,
}

/// What `watch.log` says about the polling loop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Watch {
    /// When the current watch announced itself.
    pub started_at: Option<String>,
    /// Its `--interval`, in seconds, when the announcement said.
    pub interval: Option<u64>,
    /// The last tick, dispatched or not.
    pub last_tick_at: Option<String>,
    /// A dispatch still running, as far as the journal knows.
    pub in_flight: Option<InFlight>,
    /// The last `saw:` line — the snapshot the router decided on.
    pub saw: Option<String>,
    /// The last lines, raw, newest last.
    pub recent: Vec<String>,
    /// A soft stop was asked: the watch waits for its lanes, starts nothing.
    pub draining: bool,
    /// The watch said it stopped, after its last start.
    pub stopped: bool,
    /// What the loop triggered, and its tick counts.
    pub journal: Journal,
}

/// Reads `watch.log`, keeping the last `keep` non-empty lines.
#[must_use]
pub fn parse_watch(text: &str, keep: usize) -> Watch {
    let mut watch = Watch::default();
    for line in text.lines() {
        let Some(Stamped { at, rest }) = stamped(line) else {
            continue;
        };
        if let Some(route) = rest.strip_prefix("tick: ") {
            watch.last_tick_at = Some(at.to_string());
            watch.in_flight = (route != "Nothing").then(|| InFlight {
                route: route.to_string(),
                workflow: workflow_of_route(route).unwrap_or("unknown").to_string(),
                subject: route_subject(route),
                since: at.to_string(),
            });
        } else if rest.starts_with("quiet:") {
            // A quiet tick is a tick: the board did not move, the loop polled.
            watch.last_tick_at = Some(at.to_string());
        } else if let Some(saw) = rest.strip_prefix("saw: ") {
            watch.saw = Some(saw.to_string());
        } else if let Some(every) = rest.strip_prefix("watch: every ") {
            watch.started_at = Some(at.to_string());
            watch.interval = every.split('s').next().and_then(|n| n.parse().ok());
            watch.in_flight = None;
            watch.draining = false;
            watch.stopped = false;
        } else if rest.starts_with("watch: draining") {
            watch.draining = true;
        } else if rest.starts_with("watch: stopped") {
            watch.draining = false;
            watch.stopped = true;
            watch.in_flight = None;
        } else if rest.contains("watch: ") && rest.contains(" -> ") {
            // The dispatched workflow reported back — or the doctor did,
            // which only runs once the tick is over.
            watch.in_flight = None;
        }
    }
    watch.journal = journal::read(text);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    watch.recent = lines
        .iter()
        .skip(lines.len().saturating_sub(keep))
        .map(ToString::to_string)
        .collect();
    watch
}

/// One entry of the `pipeline:` line a dev loop announces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Planned {
    /// The stage name.
    pub stage: String,
    /// `opus`, `sonnet`, or `local` for a free stage.
    pub model: String,
    /// `high`, `medium`, … or empty for a free stage.
    pub effort: String,
}

/// An issue the run named: `#62: Asset folder rule…`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Named {
    /// The issue number.
    pub number: u64,
    /// Its title, as logged.
    pub title: String,
}

/// What one run's `run.log` says about where the run is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunLog {
    /// The announced sequence, when the run announced one.
    pub planned: Vec<Planned>,
    /// `(n, total)` of the round in progress.
    pub round: Option<(u32, u32)>,
    /// The task of the current round.
    pub task: Option<Named>,
    /// The milestone the run works.
    pub milestone: Option<Named>,
    /// Stages that reported in the current round, in order.
    pub done: Vec<String>,
    /// Stages the current round skipped, in order.
    pub skipped: Vec<String>,
    /// The clock of the first line.
    pub first_at: Option<String>,
    /// The clock of the last line.
    pub last_at: Option<String>,
    /// The last line, clock removed.
    pub last_line: String,
    /// The delivery line, when the round delivered.
    pub delivered: Option<String>,
    /// The last warning, when there was one.
    pub warning: Option<String>,
    /// The branch the run announced it works on.
    pub branch: Option<String>,
}

/// `technical-refinement(opus/high) -> code(sonnet/high) -> deliver(local)`.
fn parse_pipeline(text: &str) -> Vec<Planned> {
    text.split(" -> ")
        .filter_map(|entry| {
            let (stage, spec) = entry.trim().split_once('(')?;
            let spec = spec.trim_end_matches(')');
            let (model, effort) = spec.split_once('/').unwrap_or((spec, ""));
            Some(Planned {
                stage: stage.to_string(),
                model: model.to_string(),
                effort: effort.to_string(),
            })
        })
        .collect()
}

/// `62: Asset folder rule and Scene document`.
fn parse_named(text: &str) -> Option<Named> {
    let (number, title) = text.split_once(": ")?;
    Some(Named {
        number: number.trim().parse().ok()?,
        title: title.trim().to_string(),
    })
}

/// `[code] …` → `code`. A stage tag is lowercase words joined by dashes;
/// anything else between brackets is not one.
fn stage_tag(rest: &str) -> Option<&str> {
    let inner = rest.strip_prefix('[')?;
    let end = inner.find(']')?;
    let tag = inner.get(..end)?;
    let is_tag = !tag.is_empty()
        && tag
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    is_tag.then_some(tag)
}

/// Whether `text` names `stage`, dashes read as spaces: "skipping the
/// technical refinement" names `technical-refinement`.
fn mentions(text: &str, stage: &str) -> bool {
    text.to_lowercase()
        .replace('-', " ")
        .contains(&stage.to_lowercase().replace('-', " "))
}

fn push_unique(list: &mut Vec<String>, item: &str) {
    if !list.iter().any(|known| known == item) {
        list.push(item.to_string());
    }
}

/// Reads a `run.log`. `known` are the stage names the line expects, so a
/// skip line can be attributed even when the run announced no pipeline.
#[must_use]
pub fn parse_run_log(text: &str, known: &[String]) -> RunLog {
    let mut log = RunLog::default();
    for line in text.lines() {
        let Some(Stamped { at, rest }) = stamped(line) else {
            continue;
        };
        if log.first_at.is_none() {
            log.first_at = Some(at.to_string());
        }
        log.last_at = Some(at.to_string());
        log.last_line = rest.to_string();
        if let Some(pipeline) = rest.strip_prefix("pipeline: ") {
            log.planned = parse_pipeline(pipeline);
        } else if let Some(round) = rest.strip_prefix("--- round ") {
            if let Some((n, total)) = round.trim_end_matches(" ---").split_once('/')
                && let (Ok(n), Ok(total)) = (n.parse(), total.parse())
            {
                log.round = Some((n, total));
                log.done.clear();
                log.skipped.clear();
            }
        } else if let Some(task) = rest.strip_prefix("task #") {
            log.task = parse_named(task.trim_end_matches(" [auto]"));
        } else if let Some(milestone) = rest.strip_prefix("milestone #") {
            log.milestone = parse_named(milestone.split(" — ").next().unwrap_or(milestone));
        } else if let Some(warning) = rest
            .strip_prefix("error: ")
            .or_else(|| rest.strip_prefix("warning: "))
        {
            log.warning = Some(warning.to_string());
        } else if let Some(run) = rest.strip_prefix("run ") {
            if let Some(branch) = run.split("branch ").nth(1) {
                log.branch = Some(branch.split(',').next().unwrap_or(branch).to_string());
            }
        } else if rest.contains(" delivered by PR ") {
            log.delivered = Some(rest.to_string());
        } else if let Some(tag) = stage_tag(rest) {
            // A session that only opened has not reported: it is still at work.
            if harness_core::traces::stage_opening(rest).is_none() {
                push_unique(&mut log.done, tag);
            }
        } else if rest.contains("skipp") {
            let planned: Vec<String> = log.planned.iter().map(|p| p.stage.clone()).collect();
            for stage in planned.iter().chain(known.iter()) {
                if mentions(rest, stage) {
                    push_unique(&mut log.skipped, stage);
                }
            }
        }
    }
    log
}

/// One header of `prompts.md`: `## turn 1 — /tech-analyst (opus/high)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Turn {
    /// The turn number within the stage.
    pub number: u32,
    /// The first words of the prompt — a slash command, or `(prose)`.
    pub lead: String,
    /// The model the turn opened.
    pub model: String,
    /// The effort it asked for.
    pub effort: String,
}

/// Reads the turn headers out of a `prompts.md`.
#[must_use]
pub fn parse_prompt_headers(text: &str) -> Vec<Turn> {
    text.lines()
        .filter_map(|line| {
            let header = line.strip_prefix("## turn ")?;
            let (turn, rest) = header.split_once(" — ")?;
            let (lead, spec) = rest.rsplit_once(" (")?;
            let (model, effort) = spec.trim_end_matches(')').split_once('/')?;
            Some(Turn {
                number: turn.trim().parse().ok()?,
                lead: lead.trim().to_string(),
                model: model.to_string(),
                effort: effort.to_string(),
            })
        })
        .collect()
}

/// `claude-opus-5-5` → `opus`. The family name is what a chimney is named
/// after; the version is not a different chimney.
#[must_use]
pub fn model_family(full: &str) -> Option<&'static str> {
    let lower = full.to_lowercase();
    ["opus", "sonnet", "haiku"]
        .into_iter()
        .find(|family| lower.contains(family))
}

/// The model the stream's first event announces, by family.
#[must_use]
pub fn model_from_stream_head(head: &str) -> Option<String> {
    let (_, after) = head.split_once("\"model\":\"")?;
    let (model, _) = after.split_once('"')?;
    model_family(model).map(ToString::to_string)
}

/// A sum over one key of the ledger.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Bucket {
    /// The stage, the day, the task, or the outcome.
    pub key: String,
    /// Dollars, as the carrier estimated them.
    pub usd: f64,
    /// Rows summed.
    pub count: u32,
}

/// Token totals over the ledger.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Tokens {
    /// Input tokens, cache excluded.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Read from cache.
    pub cache_read: u64,
    /// Written to cache.
    pub cache_write: u64,
}

/// What the cost ledger adds up to.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Costs {
    /// Every row's `cost_usd`, summed.
    pub total_usd: f64,
    /// Rows, that is, paid sessions.
    pub sessions: u32,
    /// By stage, most expensive first.
    pub by_stage: Vec<Bucket>,
    /// By UTC day, oldest first.
    pub by_day: Vec<Bucket>,
    /// By task, most expensive first, the twelve heaviest.
    pub by_task: Vec<Bucket>,
    /// By outcome: `ok`, `STOP`, `FAILED`, `QUOTA`.
    pub by_outcome: Vec<Bucket>,
    /// By side of the architecture the task is on — `data-layer` (built on
    /// the strongest model), `other`, or `unknown` for a task the board no
    /// longer lists. Filled by [`attribute_sides`].
    pub by_side: Vec<Bucket>,
    /// Token totals.
    pub tokens: Tokens,
    /// The newest rows, newest first.
    pub last: Vec<LedgerRow>,
}

fn bump(buckets: &mut Vec<Bucket>, key: &str, usd: f64) {
    match buckets.iter_mut().find(|bucket| bucket.key == key) {
        Some(bucket) => {
            bucket.usd += usd;
            bucket.count += 1;
        }
        None => buckets.push(Bucket {
            key: key.to_string(),
            usd,
            count: 1,
        }),
    }
}

/// Adds the ledger up. `keep_last` newest rows are carried along verbatim.
#[must_use]
pub fn summarize_costs(rows: &[LedgerRow], keep_last: usize) -> Costs {
    let mut costs = Costs::default();
    for row in rows {
        let usd = row.cost_usd.unwrap_or(0.0);
        costs.total_usd += usd;
        costs.sessions += 1;
        bump(&mut costs.by_stage, &row.stage, usd);
        bump(&mut costs.by_day, row.when.get(..10).unwrap_or(""), usd);
        // One workflow writes `65`, another `#65`: the same task, one bucket.
        let task = row.task.trim_start_matches('#');
        bump(
            &mut costs.by_task,
            if task.is_empty() { "—" } else { task },
            usd,
        );
        let outcome = if row.outcome.is_empty() {
            "?"
        } else {
            &row.outcome
        };
        bump(&mut costs.by_outcome, outcome, usd);
        costs.tokens.input += row.input.unwrap_or(0);
        costs.tokens.output += row.output.unwrap_or(0);
        costs.tokens.cache_read += row.cache_read.unwrap_or(0);
        costs.tokens.cache_write += row.cache_write.unwrap_or(0);
    }
    costs.by_stage.sort_by(|a, b| b.usd.total_cmp(&a.usd));
    costs.by_task.sort_by(|a, b| b.usd.total_cmp(&a.usd));
    costs.by_task.truncate(12);
    costs.by_day.sort_by(|a, b| a.key.cmp(&b.key));
    costs.last = rows.iter().rev().take(keep_last).cloned().collect();
    costs
}

/// The issue a lane's run worked, from its id: `20261008-152126-64` → 64.
/// `None` for a bare clock — a run in the watch's own process.
#[must_use]
pub fn issue_of_run(run_id: &str) -> Option<u64> {
    if !is_run_id(run_id) {
        return None;
    }
    run_id.get(16..)?.parse().ok()
}

/// One stage of a run, with the session it opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StageLog {
    /// The stage, as the run named it (`code`), or `session <n>` when no
    /// trace names it.
    pub stage: String,
    /// That session's lines of `session.log`, the tail when it is long.
    pub text: String,
}

/// A run's `session.log`, cut into one entry per session and each named
/// after its stage.
///
/// Every session opens with a `── turn 1 …` header, in the order they
/// opened. The names come from the journal's `[stage] session opens` lines,
/// in that same order; a run older than those lines falls back on
/// `ledger_stages` — the stages its ledger rows name, oldest first.
#[must_use]
pub fn stage_logs(
    run_log: &str,
    session_log: &str,
    ledger_stages: &[String],
    tail_bytes: usize,
) -> Vec<StageLog> {
    let opened: Vec<&str> = run_log
        .lines()
        .filter_map(|line| stamped(line).map(|s| s.rest))
        .filter_map(harness_core::traces::stage_opening)
        .collect();
    let mut sessions: Vec<Vec<&str>> = Vec::new();
    for line in session_log.lines() {
        if line.starts_with("── turn 1 ") || sessions.is_empty() {
            sessions.push(Vec::new());
        }
        if let Some(current) = sessions.last_mut() {
            current.push(line);
        }
    }
    sessions
        .into_iter()
        .enumerate()
        .map(|(n, lines)| {
            let stage = opened
                .get(n)
                .map(ToString::to_string)
                .or_else(|| {
                    (opened.is_empty())
                        .then(|| ledger_stages.get(n).cloned())
                        .flatten()
                })
                .unwrap_or_else(|| format!("session {}", n.saturating_add(1)));
            StageLog {
                stage,
                text: tail_of(&lines.join("\n"), tail_bytes),
            }
        })
        .collect()
}

/// Fills `by_side` from the ledger: `is_data_layer(task)` says which side a
/// task is on, `None` when the board does not know it.
pub fn attribute_sides(
    costs: &mut Costs,
    rows: &[LedgerRow],
    is_data_layer: impl Fn(&str) -> Option<bool>,
) {
    costs.by_side.clear();
    for row in rows {
        let key = match is_data_layer(&row.task) {
            Some(true) => "data-layer",
            Some(false) => "other",
            None => "unknown",
        };
        bump(&mut costs.by_side, key, row.cost_usd.unwrap_or(0.0));
    }
}

/// The last `max` bytes of `text`, cut on a line boundary.
fn tail_of(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len().saturating_sub(max);
    while !text.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    let cut = text.get(start..).unwrap_or_default();
    cut.split_once('\n')
        .map_or(cut, |(_, rest)| rest)
        .to_string()
}

#[cfg(test)]
mod tests {
    fn ledger_row(task: &str, usd: f64) -> LedgerRow {
        LedgerRow {
            when: String::new(),
            run: String::new(),
            round: String::new(),
            task: task.to_string(),
            stage: "code".to_string(),
            cost_usd: Some(usd),
            turns: None,
            duration_ms: None,
            input: None,
            output: None,
            cache_read: None,
            cache_write: None,
            outcome: "ok".to_string(),
        }
    }

    #[test]
    fn spending_is_attributed_to_the_side_of_the_task() {
        let rows = vec![
            ledger_row("64", 2.0),
            ledger_row("64", 1.0),
            ledger_row("65", 0.5),
            ledger_row("9", 0.25),
        ];
        let mut costs = Costs::default();
        attribute_sides(&mut costs, &rows, |task| match task {
            "64" => Some(true),
            "65" => Some(false),
            _ => None,
        });
        let usd = |key: &str| costs.by_side.iter().find(|b| b.key == key).map(|b| b.usd);
        assert_eq!(usd("data-layer"), Some(3.0));
        assert_eq!(usd("other"), Some(0.5));
        assert_eq!(usd("unknown"), Some(0.25));
    }

    #[test]
    fn a_lane_run_names_its_issue_and_a_bare_run_none() {
        assert_eq!(issue_of_run("20261008-152126-64"), Some(64));
        assert_eq!(issue_of_run("20261008-152126"), None);
        assert_eq!(issue_of_run("costs.tsv"), None);
    }

    #[test]
    fn each_session_is_named_by_the_stage_that_opened_it() {
        let run_log = "[2026-10-08T15:21:28Z] run 1\n\
                       [2026-10-08T15:21:29Z] [technical-refinement] session opens\n\
                       [2026-10-08T15:27:06Z] [code] session opens\n";
        let session_log = "── turn 1 /tech-analyst ──\nplan\n── turn 1 /code ──\nbuild\n── turn 2 /code ──\nfix\n";
        let stages = stage_logs(run_log, session_log, &[], 10_000);
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].stage, "technical-refinement");
        assert!(stages[0].text.contains("plan"));
        assert_eq!(stages[1].stage, "code");
        assert!(stages[1].text.contains("fix"));
    }

    #[test]
    fn an_older_run_falls_back_on_its_ledger_then_on_a_number() {
        let session_log = "── turn 1 a ──\nx\n── turn 1 b ──\ny\n── turn 1 c ──\nz\n";
        let stages = stage_logs(
            "",
            session_log,
            &["technical-refinement".to_string(), "code".to_string()],
            10_000,
        );
        let names: Vec<&str> = stages.iter().map(|s| s.stage.as_str()).collect();
        assert_eq!(names, ["technical-refinement", "code", "session 3"]);
    }

    use super::*;
    use proptest::prelude::*;

    const WATCH: &str = "\
[2026-10-07T05:53:00Z] watch: every 30s on Laucans/dnd_helper — journal .llocal/logs/agent-loop/watch.log
[2026-10-07T05:55:04Z] saw: 1 roadmap, 6 milestone(s), 0 refining, dev_loop=Some(17)
[2026-10-07T05:55:04Z] tick: DevLoop { milestone: 17 }
[2026-10-07T05:59:42Z] watch: dev_loop -> Continue
[2026-10-07T06:00:51Z] tick: Nothing
";

    #[test]
    fn a_stamped_line_splits_at_its_clock() {
        let line = stamped("[2026-10-07T12:50:44Z] tick: Nothing").expect("stamped");
        assert_eq!(line.at, "2026-10-07T12:50:44Z");
        assert_eq!(line.rest, "tick: Nothing");
    }

    #[test]
    fn a_stage_tag_is_not_a_clock() {
        assert!(stamped("[code] AGENT_LOOP_OK: shipped").is_none());
        assert!(stamped("  - #61 PNJ to dnd5e: delivered").is_none());
        assert!(stamped("").is_none());
    }

    #[test]
    fn the_clock_reads_as_unix_seconds() {
        assert_eq!(unix("1970-01-01T00:01:00Z"), Some(60));
        assert_eq!(unix("not a clock"), None);
    }

    #[test]
    fn a_run_id_is_a_compact_clock_and_nothing_else() {
        assert!(is_run_id("20261006-202608"));
        assert!(!is_run_id("costs.tsv"));
        assert!(!is_run_id("flow-20261007-055756.jsonl"));
        assert!(!is_run_id("../20261006-202608"));
        assert!(!is_run_id("2026100-6202608"));
        assert!(is_run_id("20261006-202608-17"));
        assert!(!is_run_id("20261006-202608-"));
        assert!(!is_run_id("20261006-202608x17"));
        assert!(!is_run_id("20261006-202608-../x"));
    }

    #[test]
    fn a_dispatched_route_is_in_flight_until_the_watch_reports_back() {
        let dispatched: String = WATCH.lines().take(3).collect::<Vec<_>>().join("\n");
        let watch = parse_watch(&dispatched, 10);
        let flight = watch.in_flight.expect("in flight");
        assert_eq!(flight.workflow, "agent-loop");
        assert_eq!(flight.subject.as_deref(), Some("milestone 17"));
        assert_eq!(flight.since, "2026-10-07T05:55:04Z");
        assert_eq!(watch.interval, Some(30));
        assert_eq!(watch.started_at.as_deref(), Some("2026-10-07T05:53:00Z"));

        let reported: String = WATCH.lines().take(4).collect::<Vec<_>>().join("\n");
        assert!(parse_watch(&reported, 10).in_flight.is_none());
    }

    #[test]
    fn a_trigger_finds_the_run_it_started() {
        let runs: Vec<String> = [
            "20261009-003827-15",
            "20261009-004034-15",
            "20261009-004035-16",
            "20261009-010000",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(
            run_of(&runs, Some(15), "2026-10-09T00:40:34Z").as_deref(),
            Some("20261009-004034-15")
        );
        assert_eq!(
            run_of(&runs, Some(16), "2026-10-09T00:40:34Z").as_deref(),
            Some("20261009-004035-16")
        );
        assert_eq!(
            run_of(&runs, None, "2026-10-09T00:59:30Z").as_deref(),
            Some("20261009-010000"),
            "an inline run, named by its clock alone"
        );
        assert_eq!(run_of(&runs, Some(15), "2026-10-09T02:00:00Z"), None);
        assert_eq!(run_of(&runs, Some(15), "not a clock"), None);
    }

    #[test]
    fn a_quiet_tick_is_still_a_tick() {
        let text = "\
[2026-10-09T00:52:32Z] tick: DevLoop { milestone: 3 }
[2026-10-09T00:54:42Z] quiet: nothing moved on the board — no snapshot read
";
        let watch = parse_watch(text, 5);
        assert_eq!(watch.last_tick_at.as_deref(), Some("2026-10-09T00:54:42Z"));
    }

    #[test]
    fn a_soft_stop_drains_then_stops_until_the_next_start() {
        let text = "\
[2026-10-08T22:00:00Z] watch: every 30s on Laucans/dnd_helper2 — journal x
[2026-10-08T22:00:01Z] tick: DevLoop { milestone: 3 }
[2026-10-08T22:01:00Z] watch: draining — soft stop asked, no new task; waiting for 1 lane(s) (#15)
";
        let draining = parse_watch(text, 5);
        assert!(draining.draining);
        assert!(!draining.stopped);

        let stopped =
            format!("{text}[2026-10-08T22:09:00Z] watch: stopped — soft stop, every lane done\n");
        let stopped = parse_watch(&stopped, 5);
        assert!(!stopped.draining);
        assert!(stopped.stopped);
        assert!(stopped.in_flight.is_none());

        let again = format!(
            "{text}[2026-10-08T22:09:00Z] watch: stopped — soft stop, every lane done\n\
             [2026-10-08T22:10:00Z] watch: every 30s on Laucans/dnd_helper2 — journal x\n"
        );
        let again = parse_watch(&again, 5);
        assert!(!again.draining && !again.stopped);
    }

    #[test]
    fn a_quiet_tick_clears_the_flight_and_keeps_the_last_lines() {
        let watch = parse_watch(WATCH, 2);
        assert!(watch.in_flight.is_none());
        assert_eq!(watch.last_tick_at.as_deref(), Some("2026-10-07T06:00:51Z"));
        assert_eq!(
            watch.saw.as_deref(),
            Some("1 roadmap, 6 milestone(s), 0 refining, dev_loop=Some(17)")
        );
        assert_eq!(watch.recent.len(), 2);
        assert!(watch.recent[1].ends_with("tick: Nothing"));
    }

    #[test]
    fn the_doctor_reporting_also_ends_the_flight() {
        let text = "[2026-10-07T05:55:04Z] tick: DevLoop { milestone: 17 }\n\
                    [2026-10-07T05:59:42Z] doctor: QUOTA on dev_loop — session limit\n\
                    [2026-10-07T05:59:42Z] watch: doctor -> CLEANED\n";
        assert!(parse_watch(text, 5).in_flight.is_none());
    }

    #[test]
    fn every_route_maps_to_its_log_folder() {
        assert_eq!(
            workflow_of_route("DevLoop { milestone: 17 }"),
            Some("agent-loop")
        );
        assert_eq!(
            workflow_of_route("PrReview { pr: \"71\", base: \"x\" }"),
            Some("pr-review")
        );
        assert_eq!(workflow_of_route("Nothing"), None);
        assert_eq!(
            route_subject("PrReview { pr: \"71\", base: \"x\" }").as_deref(),
            Some("pr 71")
        );
        assert_eq!(route_subject("Nothing"), None);
    }

    const RUN: &str = "\
[2026-10-06T20:26:10Z] run 20261006-202608 — branch milestone/17-dernier, 3 round(s)
[2026-10-06T20:26:10Z] pipeline: technical-refinement(opus/high) -> code(sonnet/high) -> create-test(sonnet/high)
[2026-10-06T20:26:16Z] milestone #17: Dernier kilomètre : un PNJ manuel jouable dans Foundry — 6 open task(s)
[2026-10-06T20:26:16Z] --- round 1/3 ---
[2026-10-06T20:26:21Z] task #62: Asset folder rule and Scene document for a place's assets [auto]
[2026-10-06T20:28:32Z] [technical-refinement] AGENT_LOOP_OK: #62 now has its sections
[2026-10-06T20:31:35Z] [code] AGENT_LOOP_OK: #62 shipped via PR #70
";

    #[test]
    fn a_run_log_says_where_the_round_is() {
        let log = parse_run_log(RUN, &[]);
        assert_eq!(log.planned.len(), 3);
        assert_eq!(log.planned[0].model, "opus");
        assert_eq!(log.round, Some((1, 3)));
        assert_eq!(log.branch.as_deref(), Some("milestone/17-dernier"));
        let task = log.task.expect("task");
        assert_eq!(task.number, 62);
        assert_eq!(
            task.title,
            "Asset folder rule and Scene document for a place's assets"
        );
        let milestone = log.milestone.expect("milestone");
        assert_eq!(milestone.number, 17);
        assert_eq!(
            milestone.title,
            "Dernier kilomètre : un PNJ manuel jouable dans Foundry"
        );
        assert_eq!(log.done, ["technical-refinement", "code"]);
        assert_eq!(log.first_at.as_deref(), Some("2026-10-06T20:26:10Z"));
        assert_eq!(log.last_at.as_deref(), Some("2026-10-06T20:31:35Z"));
        assert!(log.last_line.starts_with("[code] AGENT_LOOP_OK"));
    }

    #[test]
    fn an_opened_session_has_not_reported_yet() {
        let text = "[2026-10-08T21:23:20Z] pipeline: technical-refinement(opus/high) -> code(sonnet/high) -> create-test(sonnet/high)\n\
                    [2026-10-08T21:31:21Z] [technical-refinement] AGENT_LOOP_OK: written\n\
                    [2026-10-08T21:31:23Z] [code] session opens\n";
        assert_eq!(parse_run_log(text, &[]).done, ["technical-refinement"]);
    }

    #[test]
    fn a_new_round_forgets_the_stages_of_the_previous_one() {
        let text = format!(
            "{RUN}[2026-10-06T20:33:46Z] #62 delivered by PR #70, merged on milestone/17 — marked harness:waiting-merge\n\
             [2026-10-06T20:33:48Z] --- round 2/3 ---\n\
             [2026-10-06T20:33:53Z] task #63: Adventure document\n"
        );
        let log = parse_run_log(&text, &[]);
        assert_eq!(log.round, Some((2, 3)));
        assert_eq!(log.done, [] as [String; 0]);
        assert_eq!(log.task.expect("task").number, 63);
        assert!(log.delivered.expect("delivered").contains("PR #70"));
    }

    #[test]
    fn a_skipped_stage_is_attributed_with_or_without_a_pipeline() {
        let known = vec![
            "technical-refinement".to_string(),
            "create-test".to_string(),
        ];
        let text = "[2026-10-06T20:26:21Z] issue #64 already carries harness:tech-written — skipping the technical refinement\n\
                    [2026-10-06T20:26:22Z] /create-test skipped — not in --stages (code)\n\
                    [2026-10-06T20:26:23Z] warning: ?? todo.md\n";
        let log = parse_run_log(text, &known);
        assert_eq!(log.skipped, ["technical-refinement", "create-test"]);
        assert_eq!(log.warning.as_deref(), Some("?? todo.md"));
    }

    #[test]
    fn prompt_headers_name_the_model_each_turn_opened() {
        let text = "## turn 1 — /tech-analyst (opus/high)\n```\nbody\n```\n## turn 2 — (prose) (sonnet/medium)\n";
        let turns = parse_prompt_headers(text);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].lead, "/tech-analyst");
        assert_eq!(turns[0].model, "opus");
        assert_eq!(turns[1].lead, "(prose)");
        assert_eq!(turns[1].effort, "medium");
    }

    #[test]
    fn the_stream_head_names_the_model_family() {
        let head =
            r#"{"type":"system","subtype":"init","cwd":"/x","model":"claude-opus-5-5","tools":[]}"#;
        assert_eq!(model_from_stream_head(head).as_deref(), Some("opus"));
        assert_eq!(model_family("claude-sonnet-5"), Some("sonnet"));
        assert_eq!(model_family("gpt"), None);
    }

    fn row(when: &str, task: &str, stage: &str, usd: f64, outcome: &str) -> LedgerRow {
        LedgerRow {
            when: when.to_string(),
            run: "r".to_string(),
            round: "01".to_string(),
            task: task.to_string(),
            stage: stage.to_string(),
            cost_usd: Some(usd),
            turns: Some(3),
            duration_ms: Some(1000),
            input: Some(10),
            output: Some(20),
            cache_read: Some(30),
            cache_write: Some(40),
            outcome: outcome.to_string(),
        }
    }

    #[test]
    fn the_ledger_adds_up_by_stage_day_task_and_outcome() {
        let rows = vec![
            row("2026-10-06T15:01:07Z", "50", "code", 0.8, "STOP"),
            row("2026-10-06T15:21:14Z", "#50", "code", 2.0, "ok"),
            row("2026-10-07T09:00:00Z", "62", "create-test", 0.5, "ok"),
        ];
        let costs = summarize_costs(&rows, 2);
        assert!((costs.total_usd - 3.3).abs() < 1e-9);
        assert_eq!(costs.sessions, 3);
        assert_eq!(costs.by_stage[0].key, "code");
        assert_eq!(costs.by_stage[0].count, 2);
        assert_eq!(
            costs
                .by_day
                .iter()
                .map(|b| b.key.as_str())
                .collect::<Vec<_>>(),
            ["2026-10-06", "2026-10-07"]
        );
        assert_eq!(costs.by_task[0].key, "50", "`50` and `#50` are one task");
        assert_eq!(costs.by_task[0].count, 2);
        assert_eq!(
            costs
                .by_outcome
                .iter()
                .find(|b| b.key == "ok")
                .map(|b| b.count),
            Some(2)
        );
        assert_eq!(costs.tokens.cache_write, 120);
        assert_eq!(costs.last.len(), 2);
        assert_eq!(costs.last[0].task, "62");
    }

    proptest! {
        #[test]
        fn any_clock_and_rest_round_trip_through_stamped(
            secs in 0_i64..4_000_000_000,
            rest in "[^\\r\\n]{0,80}",
        ) {
            let at = jiff::Timestamp::from_second(secs)
                .expect("in range")
                .strftime("%Y-%m-%dT%H:%M:%SZ")
                .to_string();
            let line = format!("[{at}] {rest}");
            let split = stamped(&line).expect("a stamped line");
            prop_assert_eq!(split.at, at.as_str());
            prop_assert_eq!(split.rest, rest.as_str());
            prop_assert_eq!(unix(split.at), Some(secs));
        }
    }
}
