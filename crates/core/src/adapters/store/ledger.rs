//! The cost ledger: one row per stage, what the run actually cost.
//!
//! **The header is frozen.** Same columns, same order as the Python ledger,
//! and any new column is added **at the end** of the row: older rows have
//! fewer columns and remain readable as-is. This is the only reason `when` is
//! UTC here whereas Python wrote local time — the format does not change, only
//! the value, and a fresh repo has no history to contradict.
//!
//! This module **writes and re-reads** the ledger; it does not format it. The
//! tables a report displays are a different concern.
//!
//! The ledger does not read the clock: `when` arrives in the row. A ledger
//! that read the clock itself would not be testable, and it is the caller who
//! knows what instant it is talking about.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::domain::{Halt, Outcome, Spend};

/// The header, frozen character for character.
pub const HEADER: &str = "when\trun\tround\ttask\tstage\tcost_usd\tturns\t\
                          duration_ms\tin\tout\tsession\tran_on\tcache_read\t\
                          cache_write\toutcome";

/// The columns, derived from the header.
///
/// Single source of truth for the order: extending `HEADER` moves the
/// columns with it, rather than leaving a manually-maintained count drift.
#[must_use]
pub fn columns() -> Vec<&'static str> {
    HEADER.split('\t').collect()
}

/// What a stage cost, ready to be written.
#[derive(Debug, Clone)]
pub struct Row {
    /// The instant, in ISO-8601 to the second. Provided, never read from a clock.
    pub when: String,
    /// The run identifier.
    pub run: String,
    /// The round number.
    pub round: u32,
    /// The billed task.
    pub task: String,
    /// The stage.
    pub stage: String,
    /// What the session carrier observed.
    pub spend: Spend,
    /// The machine that ran it.
    pub ran_on: String,
    /// `ok`, or the reason for missing response. Empty = not recorded.
    pub outcome: String,
}

/// An observed number, or empty.
///
/// Empty and zero do not mean the same thing: `None` means "not observed",
/// and writing it as `0` would read as a free session where nothing was
/// measured.
fn seen<T: ToString>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

/// Free text, with nothing that could break a column.
///
/// Tabs and newlines are what shift an entire row by one column — a task
/// title contains them sooner or later.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Row {
    /// The row, as it is written.
    ///
    /// Assembled in the order of [`columns`]: the row and header cannot drift
    /// from each other.
    #[must_use]
    pub fn render(&self) -> String {
        let cost = self
            .spend
            .cost_usd
            .map_or_else(String::new, |c| format!("{c:.6}"));
        let values = [
            self.when.clone(),
            flat(&self.run),
            format!("{:02}", self.round),
            flat(&self.task),
            flat(&self.stage),
            cost,
            seen(self.spend.turns),
            seen(self.spend.duration_ms),
            seen(self.spend.tokens.input),
            seen(self.spend.tokens.output),
            flat(self.spend.session.as_deref().unwrap_or_default()),
            flat(&self.ran_on),
            seen(self.spend.tokens.cache_read),
            seen(self.spend.tokens.cache_write),
            flat(&self.outcome),
        ];
        debug_assert_eq!(
            values.len(),
            columns().len(),
            "a row must have exactly the header columns"
        );
        values.join("\t")
    }
}

/// The ledger, on disk.
pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
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
    ///
    /// [`Halt::Failed`] if the ledger could not be written. Spending that
    /// cannot be recorded is a failure: a run's budget is read from here,
    /// and a lost row makes it lie.
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

    /// Each data row, split, header excluded.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] if the file exists but cannot be read: an
    /// unreadable ledger is not an empty ledger.
    pub fn rows(&self) -> Outcome<Vec<Vec<String>>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&self.path).map_err(|e| {
            Halt::Unreadable(format!(
                "ledger unreadable at {} ({e}) — what the harness spent is \
                 unknown",
                self.path.display()
            ))
        })?;
        Ok(text
            .lines()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.split('\t').map(ToString::to_string).collect())
            .collect())
    }

    fn wrote_nothing(&self, err: &std::io::Error) -> Halt {
        Halt::Failed(format!(
            "cannot write ledger {} ({err}) — this stage's spending is not \
             recorded anywhere",
            self.path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Tokens;

    fn row() -> Row {
        Row {
            when: "2026-09-30T12:00:00Z".to_string(),
            run: "run-1".to_string(),
            round: 3,
            task: "42".to_string(),
            stage: "code".to_string(),
            spend: Spend {
                cost_usd: Some(1.5),
                turns: Some(7),
                duration_ms: Some(45_000),
                tokens: Tokens {
                    input: Some(100),
                    output: Some(20),
                    cache_read: Some(5),
                    cache_write: Some(6),
                },
                session: Some("sess-42".to_string()),
            },
            ran_on: "macbook".to_string(),
            outcome: "ok".to_string(),
        }
    }

    #[test]
    fn a_row_has_exactly_the_columns_of_the_header() {
        assert_eq!(row().render().split('\t').count(), columns().len());
    }

    #[test]
    fn the_header_is_the_frozen_one() {
        // Frozen: columns and their order are a disk format.
        assert_eq!(
            HEADER,
            "when\trun\tround\ttask\tstage\tcost_usd\tturns\tduration_ms\tin\t\
             out\tsession\tran_on\tcache_read\tcache_write\toutcome"
        );
        assert_eq!(columns().len(), 15);
    }

    #[test]
    fn values_land_in_the_column_the_header_names() {
        let rendered = row().render();
        let cells: Vec<&str> = rendered.split('\t').collect();
        let at = |name: &str| {
            cells[columns()
                .iter()
                .position(|c| *c == name)
                .expect("known column")]
        };
        assert_eq!(at("when"), "2026-09-30T12:00:00Z");
        assert_eq!(at("round"), "03");
        assert_eq!(at("stage"), "code");
        assert_eq!(at("cost_usd"), "1.500000");
        assert_eq!(at("in"), "100");
        assert_eq!(at("out"), "20");
        assert_eq!(at("cache_read"), "5");
        assert_eq!(at("cache_write"), "6");
        assert_eq!(at("session"), "sess-42");
        assert_eq!(at("outcome"), "ok");
    }

    #[test]
    fn an_unobserved_number_is_left_blank_not_written_as_zero() {
        let blind = Row {
            spend: Spend::default(),
            ..row()
        };
        let rendered = blind.render();
        let cells: Vec<&str> = rendered.split('\t').collect();
        let at = |name: &str| {
            cells[columns()
                .iter()
                .position(|c| *c == name)
                .expect("known column")]
        };
        // A zero would read back as a free session; empty means
        // "not measured", which is the case under a blind carrier.
        assert_eq!(at("cost_usd"), "");
        assert_eq!(at("in"), "");
        assert_eq!(at("turns"), "");
    }

    #[test]
    fn a_tab_in_free_text_cannot_shift_the_whole_line() {
        let messy = Row {
            task: "42\twith\ttabs".to_string(),
            ..row()
        };
        assert_eq!(messy.render().split('\t').count(), columns().len());
    }

    #[test]
    fn a_newline_in_free_text_cannot_split_the_row_in_two() {
        let messy = Row {
            task: "42\non two lines".to_string(),
            ..row()
        };
        let rendered = messy.render();
        assert!(!rendered.contains('\n'));
        assert_eq!(rendered.split('\t').count(), columns().len());
    }

    #[test]
    fn writing_then_reading_back_gives_the_row_without_the_header() {
        let dir = std::env::temp_dir().join(format!("harness-ledger-{}", std::process::id()));
        let path = dir.join("costs.tsv");
        let _ = std::fs::remove_dir_all(&dir);
        let ledger = Ledger::new(&path);

        ledger.append(&row()).expect("first row");
        ledger.append(&row()).expect("second row");

        let back = ledger.rows().expect("re-read");
        assert_eq!(back.len(), 2, "header does not count as a row");
        assert_eq!(back[0].len(), columns().len());

        // Header is written only once.
        let raw = std::fs::read_to_string(&path).expect("read");
        assert_eq!(raw.matches(HEADER).count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_ledger_that_does_not_exist_yet_reads_as_no_rows() {
        let path = std::env::temp_dir().join("harness-ledger-absent/nowhere.tsv");
        assert!(Ledger::new(&path).rows().expect("absent").is_empty());
    }
}
