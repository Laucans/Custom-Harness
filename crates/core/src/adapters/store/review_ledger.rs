//! The ledger of a PR review: columns distinct from the rounds ledger.
//!
//! **A separate ledger, by design.** A review charges per PR and per pass,
//! never by round or task — mixing the two would make a review cost read as
//! a delivery cost. Same discipline as [`super::ledger`]: frozen header,
//! columns added at row end, `outcome` distinguishes money that bought
//! something from what a cut pass burned.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::domain::{Halt, Outcome, Spend};

/// The header, frozen.
pub const HEADER: &str =
    "when\tpr\tpass\tcost_usd\tturns\tduration_ms\tin\tout\tsession\tran_on\toutcome";

/// The columns, derived from the header.
#[must_use]
pub fn columns() -> Vec<&'static str> {
    HEADER.split('\t').collect()
}

/// What a review pass cost.
#[derive(Debug, Clone)]
pub struct Row {
    /// The instant, in ISO-8601 to the second.
    pub when: String,
    /// The reviewed PR.
    pub pr: String,
    /// `inline` or `brief`.
    pub pass: String,
    /// What the session carrier observed.
    pub spend: Spend,
    /// The machine that ran it.
    pub ran_on: String,
    /// `ok`, or the reason for missing response. Empty = not recorded.
    pub outcome: String,
}

fn seen<T: ToString>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Row {
    /// The row, as it is written.
    #[must_use]
    pub fn render(&self) -> String {
        let cost = self
            .spend
            .cost_usd
            .map_or_else(String::new, |c| format!("{c:.6}"));
        let values = [
            self.when.clone(),
            flat(&self.pr),
            flat(&self.pass),
            cost,
            seen(self.spend.turns),
            seen(self.spend.duration_ms),
            seen(self.spend.tokens.input),
            seen(self.spend.tokens.output),
            flat(self.spend.session.as_deref().unwrap_or_default()),
            flat(&self.ran_on),
            flat(&self.outcome),
        ];
        debug_assert_eq!(values.len(), columns().len());
        values.join("\t")
    }
}

/// The review ledger, on disk.
pub struct ReviewLedger {
    path: PathBuf,
}

impl ReviewLedger {
    /// The ledger at this path. Nothing is created before the first write.
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    /// Add a row. Writes the header first if the file does not exist.
    ///
    /// # Errors
    /// [`Halt::Failed`] if the ledger could not be written.
    pub fn append(&self, row: &Row) -> Outcome<()> {
        let fresh = !self.path.exists();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| self.wrote_nothing(&e))?;
        }
        let mut text = String::new();
        if fresh {
            let _ = writeln!(text, "{HEADER}");
        }
        let _ = writeln!(text, "{}", row.render());
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| self.wrote_nothing(&e))?;
        file.write_all(text.as_bytes())
            .map_err(|e| self.wrote_nothing(&e))
    }

    /// What both passes of a PR cost together, in dollars, formatted to four
    /// decimals — or empty if nothing is recorded yet.
    ///
    /// # Errors
    /// [`Halt::Unreadable`] if the ledger exists but cannot be read.
    pub fn cost_of(&self, pr: &str) -> Outcome<String> {
        if !self.path.exists() {
            return Ok(String::new());
        }
        let text = std::fs::read_to_string(&self.path).map_err(|e| {
            Halt::Unreadable(format!(
                "review ledger unreadable at {} ({e})",
                self.path.display()
            ))
        })?;
        let columns = columns();
        let pr_at = columns.iter().position(|c| *c == "pr").unwrap_or(1);
        let cost_at = columns.iter().position(|c| *c == "cost_usd").unwrap_or(3);
        let mut total = 0.0;
        for line in text.lines().skip(1) {
            let cells: Vec<&str> = line.split('\t').collect();
            if cells.len() > cost_at && cells.get(pr_at) == Some(&pr) {
                total += cells[cost_at].parse::<f64>().unwrap_or(0.0);
            }
        }
        if total == 0.0 {
            return Ok(String::new());
        }
        Ok(format!("{total:.4}"))
    }

    fn wrote_nothing(&self, err: &std::io::Error) -> Halt {
        Halt::Failed(format!(
            "cannot write review ledger {} ({err})",
            self.path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Tokens;

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

    fn row(pr: &str, pass: &str, cost: f64) -> Row {
        Row {
            when: "2026-10-02T15:00:00Z".to_string(),
            pr: pr.to_string(),
            pass: pass.to_string(),
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
        }
    }

    #[test]
    fn a_fresh_ledger_writes_the_header_once() {
        let dir = Dir::new("fresh");
        let path = dir.0.join("costs.tsv");
        let ledger = ReviewLedger::new(&path);
        ledger.append(&row("32", "inline", 0.5)).expect("written");
        ledger.append(&row("32", "brief", 0.2)).expect("written");
        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(text.lines().next(), Some(HEADER));
        assert_eq!(text.lines().count(), 3);
    }

    #[test]
    fn the_cost_of_a_pr_sums_both_passes() {
        let dir = Dir::new("sum");
        let path = dir.0.join("costs.tsv");
        let ledger = ReviewLedger::new(&path);
        ledger.append(&row("32", "inline", 0.5)).expect("written");
        ledger.append(&row("32", "brief", 0.25)).expect("written");
        ledger.append(&row("99", "inline", 9.0)).expect("written");
        assert_eq!(ledger.cost_of("32").expect("read"), "0.7500");
    }

    #[test]
    fn a_pr_with_no_recorded_cost_is_the_empty_string() {
        let dir = Dir::new("absent");
        let ledger = ReviewLedger::new(&dir.0.join("costs.tsv"));
        assert_eq!(ledger.cost_of("32").expect("read"), "");
    }

    #[test]
    fn a_blind_spend_renders_an_empty_cost_not_a_zero() {
        let dir = Dir::new("blind");
        let path = dir.0.join("costs.tsv");
        let ledger = ReviewLedger::new(&path);
        let mut blind = row("32", "inline", 0.0);
        blind.spend = Spend::default();
        blind.outcome = "quota".to_string();
        ledger.append(&blind).expect("written");
        let text = std::fs::read_to_string(&path).expect("read");
        let data = text.lines().nth(1).expect("a row");
        assert_eq!(data.split('\t').nth(3), Some(""));
    }
}
