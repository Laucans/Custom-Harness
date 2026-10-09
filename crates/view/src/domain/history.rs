//! The control room over a chosen period: what was spent, what stopped, what
//! the watch triggered, and its journal — between two instants the human
//! picked, rather than "lately".
//!
//! Every trace is stamped with the same clock form (`2026-10-08T21:31:21Z`,
//! UTC), so a period is two such stamps and "in it" is a comparison: the
//! form sorts as it reads.

use serde::Serialize;

use crate::domain::journal::{self, Journal};
use crate::domain::traces::{self, Costs, Stamped, attribute_sides, stamped, summarize_costs};
use crate::ports::{ErrorRow, LedgerRow};

/// How many of the period's paid sessions are listed one by one.
const SESSIONS_LISTED: usize = 100;

/// How many of the period's stops are listed.
const STOPS_LISTED: usize = 200;

/// How many journal lines a period shows, its last ones.
const LOG_LINES: usize = 600;

/// Two instants, either open: from the start of the traces, until now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Range {
    /// The first instant, inclusive.
    pub from: Option<String>,
    /// The last instant, inclusive.
    pub to: Option<String>,
}

impl Range {
    /// A period from what the page sent.
    ///
    /// # Errors
    ///
    /// An instant that is not a trace clock (`2026-10-08T21:31:21Z`), or a
    /// period that ends before it starts.
    pub fn parse(from: Option<&str>, to: Option<&str>) -> Result<Self, String> {
        let clock = |at: Option<&str>| -> Result<Option<String>, String> {
            match at.map(str::trim).filter(|at| !at.is_empty()) {
                None => Ok(None),
                Some(at) if at.len() == 20 && traces::unix(at).is_some() => {
                    Ok(Some(at.to_string()))
                }
                Some(at) => Err(format!(
                    "{at:?} is not an instant like 2026-10-08T21:31:21Z"
                )),
            }
        };
        let range = Self {
            from: clock(from)?,
            to: clock(to)?,
        };
        if let (Some(from), Some(to)) = (&range.from, &range.to)
            && from > to
        {
            return Err(format!("the period ends ({to}) before it starts ({from})"));
        }
        Ok(range)
    }

    /// Whether `at` falls in the period.
    #[must_use]
    pub fn holds(&self, at: &str) -> bool {
        self.from.as_deref().is_none_or(|from| at >= from)
            && self.to.as_deref().is_none_or(|to| at <= to)
    }

    /// Whether something that began at `start` and ended at `end` (`None`:
    /// still going) ran at some point of the period.
    #[must_use]
    pub fn overlaps(&self, start: &str, end: Option<&str>) -> bool {
        self.to.as_deref().is_none_or(|to| start <= to)
            && match (self.from.as_deref(), end) {
                (Some(from), Some(end)) => end >= from,
                _ => true,
            }
    }
}

/// The control room's figures over one period.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct History {
    /// The period.
    pub range: Range,
    /// What its paid sessions cost, summed the way the Spending screen sums.
    pub costs: Costs,
    /// Its stops, newest first.
    pub errors: Vec<ErrorRow>,
    /// What the watch triggered during it, and its ticks.
    pub journal: Journal,
    /// The journal lines of the period, oldest first — its last ones.
    pub logs: Vec<String>,
    /// More lines fell in the period than are shown.
    pub logs_cut: bool,
}

/// Everything the control room shows, over `range`.
#[must_use]
pub fn history(
    range: Range,
    ledger: &[LedgerRow],
    errors: &[ErrorRow],
    watch_log: &str,
    is_data_layer: impl Fn(&str) -> Option<bool>,
) -> History {
    let rows: Vec<LedgerRow> = ledger
        .iter()
        .filter(|row| range.holds(&row.when))
        .cloned()
        .collect();
    let mut costs = summarize_costs(&rows, SESSIONS_LISTED);
    attribute_sides(&mut costs, &rows, is_data_layer);
    let errors = errors
        .iter()
        .rev()
        .filter(|row| range.holds(&row.when))
        .take(STOPS_LISTED)
        .cloned()
        .collect();
    let lines: Vec<&str> = watch_log
        .lines()
        .filter(|line| stamped(line).is_some_and(|Stamped { at, .. }| range.holds(at)))
        .collect();
    let logs_cut = lines.len() > LOG_LINES;
    let logs = lines
        .iter()
        .skip(lines.len().saturating_sub(LOG_LINES))
        .map(ToString::to_string)
        .collect();
    History {
        journal: journal::read_within(watch_log, Some(&range)),
        range,
        costs,
        errors,
        logs,
        logs_cut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn row(when: &str, task: &str, usd: f64) -> LedgerRow {
        LedgerRow {
            when: when.to_string(),
            run: "r".to_string(),
            round: "01".to_string(),
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

    fn stop(when: &str) -> ErrorRow {
        ErrorRow {
            when: when.to_string(),
            run: "r".to_string(),
            workflow: "watch".to_string(),
            kind: "STOP".to_string(),
            reason: "x".to_string(),
        }
    }

    #[test]
    fn a_period_is_two_trace_clocks_in_order() {
        assert!(Range::parse(Some("2026-10-08T20:00:00Z"), Some("2026-10-08T21:00:00Z")).is_ok());
        assert!(
            Range::parse(None, Some("")).is_ok(),
            "both ends may be open"
        );
        assert!(Range::parse(Some("yesterday"), None).is_err());
        assert!(Range::parse(Some("2026-10-08T20:00:00.000Z"), None).is_err());
        assert!(Range::parse(Some("2026-10-08T21:00:00Z"), Some("2026-10-08T20:00:00Z")).is_err());
    }

    #[test]
    fn a_period_holds_its_ends_and_overlaps_what_crosses_them() {
        let range = Range::parse(Some("2026-10-08T20:00:00Z"), Some("2026-10-08T21:00:00Z"))
            .expect("range");
        assert!(range.holds("2026-10-08T20:00:00Z") && range.holds("2026-10-08T21:00:00Z"));
        assert!(!range.holds("2026-10-08T21:00:01Z"));
        assert!(range.overlaps("2026-10-08T19:00:00Z", Some("2026-10-08T20:30:00Z")));
        assert!(range.overlaps("2026-10-08T19:00:00Z", None));
        assert!(!range.overlaps("2026-10-08T19:00:00Z", Some("2026-10-08T19:59:59Z")));
        assert!(!range.overlaps("2026-10-08T21:00:01Z", None));
    }

    #[test]
    fn the_figures_are_those_of_the_period_only() {
        let range = Range::parse(Some("2026-10-08T20:00:00Z"), Some("2026-10-08T21:00:00Z"))
            .expect("range");
        let ledger = [
            row("2026-10-08T19:59:59Z", "14", 4.0),
            row("2026-10-08T20:10:00Z", "15", 1.5),
            row("2026-10-08T20:50:00Z", "15", 2.0),
            row("2026-10-08T21:30:00Z", "16", 8.0),
        ];
        let errors = [stop("2026-10-08T20:45:00Z"), stop("2026-10-08T22:00:00Z")];
        let log = "\
[2026-10-08T19:30:00Z] quiet: nothing moved
[2026-10-08T20:30:00Z] quiet: nothing moved
[2026-10-08T20:31:00Z] quiet: nothing moved
";
        let history = history(range, &ledger, &errors, log, |_| None);
        assert!((history.costs.total_usd - 3.5).abs() < 1e-9);
        assert_eq!(history.costs.sessions, 2);
        assert_eq!(history.errors.len(), 1);
        assert_eq!(history.logs.len(), 2);
        assert!(!history.logs_cut);
        assert_eq!((history.journal.ticks, history.journal.empty_ticks), (2, 2));
    }

    proptest! {
        #[test]
        fn any_two_instants_parse_without_panicking(a in "\\PC{0,30}", b in "\\PC{0,30}") {
            let _ = Range::parse(Some(&a), Some(&b));
        }
    }
}
