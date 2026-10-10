//! What a pull request cost — the implementation of [`ReviewCosts`], read
//! from the rounds ledger.
//!
//! **One ledger for every workflow.** Every paid session lands in
//! `costs.tsv` through the `Spending` port, a review's passes included,
//! with the pull request's number in the `task` column. A review's cost is
//! therefore a *read* of that ledger — the rows of that number — and no
//! second file: an earlier shape wrote reviews to a ledger of their own
//! that nothing filled, so every posted comment said nothing of its cost.
//!
//! Issues and pull requests share one numbering on GitHub, so a `task` that
//! names a pull request names nothing else: the sum is what the harness
//! spent on that pull request, its review passes and any repair.

use std::path::Path;

use harness_core::domain::Outcome;
use harness_core::ports::store::review::ReviewCosts;

use super::ledger::{Ledger, columns};

/// A pull request's spending, read from the rounds ledger.
pub struct ReviewLedger {
    ledger: Ledger,
}

impl ReviewLedger {
    /// Over the rounds ledger at this path — the one every workflow writes.
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self {
            ledger: Ledger::new(path),
        }
    }
}

impl ReviewCosts for ReviewLedger {
    /// Every row of this pull request, summed, in dollars to four decimals.
    fn cost_of(&self, pr: &str) -> Outcome<String> {
        let columns = columns();
        let task_at = columns.iter().position(|c| *c == "task").unwrap_or(3);
        let cost_at = columns.iter().position(|c| *c == "cost_usd").unwrap_or(5);
        let mut total = 0.0;
        for cells in self.ledger.rows()? {
            if cells.get(task_at).map(String::as_str) == Some(pr)
                && let Some(cost) = cells.get(cost_at)
            {
                total += cost.parse::<f64>().unwrap_or(0.0);
            }
        }
        if total == 0.0 {
            return Ok(String::new());
        }
        Ok(format!("{total:.4}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::{Spend, Tokens};
    use std::path::PathBuf;

    use super::super::ledger::Row;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "harness-review-ledger-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn row(task: &str, stage: &str, cost: f64) -> Row {
        Row {
            when: "2026-10-02T15:00:00Z".to_string(),
            run: "20261002-150000".to_string(),
            round: 1,
            task: task.to_string(),
            stage: stage.to_string(),
            spend: Spend {
                cost_usd: Some(cost),
                turns: Some(3),
                tokens: Tokens {
                    input: Some(100),
                    output: Some(50),
                    ..Tokens::default()
                },
                ..Spend::default()
            },
            ran_on: "test-host".to_string(),
            outcome: "ok".to_string(),
            fingerprint: String::new(),
        }
    }

    #[test]
    fn the_cost_of_a_pr_sums_its_rows_and_nobody_else_s() {
        let dir = Dir::new("sum");
        let path = dir.0.join("costs.tsv");
        let ledger = Ledger::new(&path);
        ledger.append(&row("32", "inline", 0.5)).expect("written");
        ledger.append(&row("32", "brief", 0.25)).expect("written");
        ledger.append(&row("99", "inline", 9.0)).expect("written");
        ledger.append(&row("7", "code", 4.0)).expect("written");
        assert_eq!(
            ReviewLedger::new(&path).cost_of("32").expect("read"),
            "0.7500"
        );
    }

    #[test]
    fn a_pr_with_no_recorded_cost_is_the_empty_string() {
        let dir = Dir::new("absent");
        let ledger = ReviewLedger::new(&dir.0.join("costs.tsv"));
        assert_eq!(ledger.cost_of("32").expect("read"), "");
    }

    #[test]
    fn a_blind_spend_reads_as_nothing_recorded_not_as_free() {
        let dir = Dir::new("blind");
        let path = dir.0.join("costs.tsv");
        let mut blind = row("32", "inline", 0.0);
        blind.spend = Spend::default();
        blind.outcome = "quota".to_string();
        Ledger::new(&path).append(&blind).expect("written");
        assert_eq!(ReviewLedger::new(&path).cost_of("32").expect("read"), "");
    }
}
