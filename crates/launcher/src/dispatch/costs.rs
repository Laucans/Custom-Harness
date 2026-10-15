//! `harness costs`: what the harness spent, by what it was spent on.
//!
//! Reads the one ledger (`.llocal/logs/agent-loop/costs.tsv`) and groups its
//! rows by a kind of unit, a profile, a stage, a side or a task — the
//! question behind every optimisation of the loop's context: *what does one
//! Capability cost, and did the last change move it?* Mounts nothing, runs
//! no session, writes nothing: the ledger is read as the loop wrote it.
//!
//! The grouping is pure ([`report`]) and tested on rows; only [`run`]
//! touches the disk.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use harness_adapters::store::ledger::{Ledger, Row};
use harness_core::domain::Outcome;
use harness_core::domain::workspace::Workspace;

use crate::cli::{CostsArgs, GroupBy};

/// One line of the report: the rows of one group, summed.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Line {
    /// How many stage calls the group holds.
    pub calls: usize,
    /// The dollars, over the rows that observed a cost.
    pub cost_usd: f64,
    /// The model turns, over the rows that observed them.
    pub turns: u64,
    /// Input tokens, cache excluded.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Tokens read from the cache.
    pub cache_read: u64,
    /// How many rows ended in something other than `ok`.
    pub failed: usize,
}

impl Line {
    fn add(&mut self, row: &Row) {
        self.calls += 1;
        self.cost_usd += row.spend.cost_usd.unwrap_or(0.0);
        self.turns += u64::from(row.spend.turns.unwrap_or(0));
        self.input += row.spend.tokens.input.unwrap_or(0);
        self.output += row.spend.tokens.output.unwrap_or(0);
        self.cache_read += row.spend.tokens.cache_read.unwrap_or(0);
        if row.outcome != "ok" {
            self.failed += 1;
        }
    }

    /// Dollars per call, the figure a split reads to size a slice.
    #[must_use]
    pub fn cost_per_call(&self) -> f64 {
        // `u32` is more calls than any ledger holds, and converts exactly.
        match u32::try_from(self.calls) {
            Ok(0) | Err(_) => 0.0,
            Ok(calls) => self.cost_usd / f64::from(calls),
        }
    }
}

/// The key a row is grouped under. Empty cells read as `(none)`, so a row
/// from before the column existed is counted, visibly, rather than dropped.
fn key_of(row: &Row, by: GroupBy) -> String {
    let cell = match by {
        GroupBy::Unit => row.tags.unit.as_str(),
        GroupBy::Profile => row.tags.profile.as_str(),
        GroupBy::Side => row.tags.side.as_str(),
        GroupBy::Stage => row.stage.as_str(),
        GroupBy::Task => row.task.as_str(),
    };
    if cell.is_empty() {
        "(none)".to_string()
    } else {
        cell.to_string()
    }
}

/// The rows, grouped and summed. Rows whose `when` sorts before `since`
/// are left out; an empty `since` keeps everything.
#[must_use]
pub fn report(rows: &[Row], by: GroupBy, since: &str) -> BTreeMap<String, Line> {
    let mut lines: BTreeMap<String, Line> = BTreeMap::new();
    for row in rows {
        if !since.is_empty() && row.when.as_str() < since {
            continue;
        }
        lines.entry(key_of(row, by)).or_default().add(row);
    }
    lines
}

/// The report as a table, one line per group, a total last.
#[must_use]
pub fn render(lines: &BTreeMap<String, Line>, by: GroupBy) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<28} {:>6} {:>10} {:>9} {:>7} {:>11} {:>10} {:>10} {:>6}",
        by.name(),
        "calls",
        "usd",
        "usd/call",
        "turns",
        "in",
        "out",
        "cache_read",
        "failed"
    );
    let mut total = Line::default();
    for (key, line) in lines {
        let _ = writeln!(
            out,
            "{key:<28} {:>6} {:>10.2} {:>9.2} {:>7} {:>11} {:>10} {:>10} {:>6}",
            line.calls,
            line.cost_usd,
            line.cost_per_call(),
            line.turns,
            line.input,
            line.output,
            line.cache_read,
            line.failed
        );
        total.calls += line.calls;
        total.cost_usd += line.cost_usd;
        total.turns += line.turns;
        total.input += line.input;
        total.output += line.output;
        total.cache_read += line.cache_read;
        total.failed += line.failed;
    }
    let _ = writeln!(
        out,
        "{:<28} {:>6} {:>10.2} {:>9.2} {:>7} {:>11} {:>10} {:>10} {:>6}",
        "total",
        total.calls,
        total.cost_usd,
        total.cost_per_call(),
        total.turns,
        total.input,
        total.output,
        total.cache_read,
        total.failed
    );
    out
}

/// Reads the ledger of the checkout at `here` and prints the report.
///
/// # Errors
///
/// [`harness_core::domain::Halt::Unreadable`] when the ledger exists and
/// cannot be read. A ledger that does not exist yet is an empty report.
pub fn run(args: &CostsArgs, here: &Path) -> Outcome<String> {
    let rows = Ledger::new(&Workspace::new(here).ledger()).read()?;
    Ok(render(&report(&rows, args.by, &args.since), args.by))
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::{Spend, Tags, Tokens};

    fn row(when: &str, task: &str, stage: &str, unit: &str, cost: f64, outcome: &str) -> Row {
        Row {
            when: when.to_string(),
            run: "r".to_string(),
            round: 1,
            task: task.to_string(),
            stage: stage.to_string(),
            spend: Spend {
                cost_usd: Some(cost),
                turns: Some(10),
                tokens: Tokens {
                    input: Some(1000),
                    output: Some(200),
                    cache_read: Some(50_000),
                    cache_write: None,
                },
                ..Spend::default()
            },
            ran_on: "h".to_string(),
            outcome: outcome.to_string(),
            fingerprint: String::new(),
            tags: Tags {
                unit: unit.to_string(),
                profile: "mid".to_string(),
                side: "harness:read-side".to_string(),
            },
        }
    }

    fn rows() -> Vec<Row> {
        vec![
            row(
                "2026-10-01T10:00:00Z",
                "61",
                "code",
                "capability",
                2.0,
                "ok",
            ),
            row(
                "2026-10-02T10:00:00Z",
                "62",
                "code",
                "capability",
                4.0,
                "ok",
            ),
            row(
                "2026-10-03T10:00:00Z",
                "63",
                "code",
                "aggregate",
                9.0,
                "STOP",
            ),
            row(
                "2026-10-03T11:00:00Z",
                "63",
                "create-test",
                "aggregate",
                1.0,
                "ok",
            ),
        ]
    }

    #[test]
    fn rows_are_summed_under_their_unit() {
        let lines = report(&rows(), GroupBy::Unit, "");
        let capability = &lines["capability"];
        assert_eq!(capability.calls, 2);
        assert!((capability.cost_usd - 6.0).abs() < 1e-9);
        assert!((capability.cost_per_call() - 3.0).abs() < 1e-9);
        assert_eq!(capability.turns, 20);
        assert_eq!(capability.input, 2000);
        assert_eq!(capability.output, 400);
        assert_eq!(capability.cache_read, 100_000);
        assert_eq!(capability.failed, 0);
        let aggregate = &lines["aggregate"];
        assert_eq!(aggregate.calls, 2);
        assert_eq!(aggregate.failed, 1, "a STOP is not an ok");
    }

    #[test]
    fn every_grouping_reads_the_cell_it_names() {
        let all = rows();
        assert_eq!(
            report(&all, GroupBy::Stage, "")
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["code", "create-test"]
        );
        assert_eq!(
            report(&all, GroupBy::Task, "")
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["61", "62", "63"]
        );
        assert_eq!(
            report(&all, GroupBy::Profile, "")
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["mid"]
        );
        assert_eq!(
            report(&all, GroupBy::Side, "")
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["harness:read-side"]
        );
    }

    #[test]
    fn since_keeps_the_rows_from_that_instant_on() {
        let lines = report(&rows(), GroupBy::Task, "2026-10-02");
        assert_eq!(lines.keys().cloned().collect::<Vec<_>>(), ["62", "63"]);
        assert_eq!(lines["63"].calls, 2);
    }

    #[test]
    fn a_row_at_the_very_instant_of_since_is_kept() {
        // `--since 2026-10-02T10:00:00Z` means that second included: a row
        // stamped exactly then is the first one wanted, not the last one dropped.
        let lines = report(&rows(), GroupBy::Task, "2026-10-02T10:00:00Z");
        assert!(lines.contains_key("62"), "{:?}", lines.keys());
        assert!(!lines.contains_key("61"));
    }

    #[test]
    fn the_total_line_sums_every_column() {
        let lines = report(&rows(), GroupBy::Unit, "");
        let text = render(&lines, GroupBy::Unit);
        let last = text.lines().last().expect("a total");
        let cells: Vec<&str> = last.split_whitespace().collect();
        // total | calls | usd | usd/call | turns | in | out | cache_read | failed
        assert_eq!(cells[1], "4", "{last}");
        assert_eq!(cells[2], "16.00", "{last}");
        assert_eq!(cells[3], "4.00", "{last}");
        assert_eq!(cells[4], "40", "turns: {last}");
        assert_eq!(cells[5], "4000", "input: {last}");
        assert_eq!(cells[6], "800", "output: {last}");
        assert_eq!(cells[7], "200000", "cache read: {last}");
        assert_eq!(cells[8], "1", "failed: {last}");
    }

    #[test]
    fn a_row_without_the_column_is_counted_under_none_not_dropped() {
        let mut old = row("2026-10-01T10:00:00Z", "1", "code", "", 1.0, "ok");
        old.tags = Tags::default();
        let lines = report(&[old], GroupBy::Unit, "");
        assert_eq!(lines["(none)"].calls, 1);
    }

    #[test]
    fn an_empty_group_costs_nothing_per_call() {
        assert!(Line::default().cost_per_call().abs() < f64::EPSILON);
    }

    #[test]
    fn the_table_names_the_grouping_and_ends_with_the_total() {
        let lines = report(&rows(), GroupBy::Unit, "");
        let text = render(&lines, GroupBy::Unit);
        let first = text.lines().next().expect("a header");
        assert!(first.starts_with("unit"), "{first}");
        assert!(text.contains("capability"));
        assert!(text.contains("aggregate"));
        let last = text.lines().last().expect("a total");
        assert!(last.starts_with("total"), "{last}");
        assert!(last.contains("16.00"), "2 + 4 + 9 + 1: {last}");
        assert!(last.contains(" 4 "), "four calls: {last}");
    }

    #[test]
    fn an_absent_ledger_is_an_empty_report() {
        let dir = std::env::temp_dir().join(format!("harness-costs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        let text = run(
            &CostsArgs {
                by: GroupBy::Unit,
                since: String::new(),
            },
            &dir,
        )
        .expect("readable");
        assert_eq!(text.lines().count(), 2, "a header and a total: {text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
