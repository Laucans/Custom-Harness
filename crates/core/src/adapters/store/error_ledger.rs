//! The ledger of what stopped a run, and of what was done about it.
//!
//! **A separate ledger, by design.** [`super::ledger`] answers "what did this
//! cost"; this one answers "why did it stop, and is it still stopping". Mixing
//! them would put rows with no money in a file whose every reader sums
//! columns.
//!
//! Same discipline as the other two: frozen header, columns added at row end.
//!
//! # Why it exists
//!
//! A polling loop reports a failure to its console and forgets it. That is
//! enough while a human reads the console, and useless the moment the loop is
//! meant to recover on its own: nothing can ask "what went wrong last time"
//! without a record. The repair in
//! [`doctor`](crate::domain::doctor) reads this file, and the rows it appends
//! for itself are what keeps a repair from being tried forever.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::domain::{Halt, Outcome};

/// The header, frozen.
pub const HEADER: &str = "when\trun\tworkflow\tkind\treason";

/// The columns, derived from the header.
#[must_use]
pub fn columns() -> Vec<&'static str> {
    HEADER.split('\t').collect()
}

/// The `workflow` a repair writes under, rather than a workflow's name.
pub const DOCTOR: &str = "doctor";

/// What stopped a run, or what a repair did about it.
#[derive(Debug, Clone)]
pub struct Row {
    /// The instant, in ISO-8601 to the second. Provided, never read from a
    /// clock — same reason as the cost ledger.
    pub when: String,
    /// The run identifier.
    pub run: String,
    /// Which workflow stopped (`dev_loop`, `planner`…), or [`DOCTOR`].
    pub workflow: String,
    /// `STOP`, `FAILED`, `QUOTA` as [`Halt::prefix`] writes them — or, for a
    /// repair, what it concluded.
    pub kind: String,
    /// The failure's own words, flattened to one line.
    pub reason: String,
}

/// Free text with nothing that could break a column.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Row {
    /// What this failure is, from the `Halt` that carries it.
    #[must_use]
    pub fn of(when: &str, run: &str, workflow: &str, halt: &Halt) -> Self {
        Self {
            when: when.to_string(),
            run: run.to_string(),
            workflow: workflow.to_string(),
            kind: halt.prefix().to_string(),
            reason: halt.reason().to_string(),
        }
    }

    /// The row, as it is written.
    #[must_use]
    pub fn render(&self) -> String {
        let values = [
            self.when.clone(),
            flat(&self.run),
            flat(&self.workflow),
            flat(&self.kind),
            flat(&self.reason),
        ];
        debug_assert_eq!(
            values.len(),
            columns().len(),
            "a row must have exactly the header columns"
        );
        values.join("\t")
    }
}

/// What the ledger last recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Last {
    /// Which workflow it concerned.
    pub workflow: String,
    /// `STOP`, `FAILED`, `QUOTA`, or a repair's conclusion.
    pub kind: String,
    /// What it said.
    pub reason: String,
}

/// The error ledger, on disk.
pub struct ErrorLedger {
    path: PathBuf,
}

impl ErrorLedger {
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
    /// [`Halt::Failed`] if the ledger could not be written: a repair that
    /// cannot record what it did would be retried at every tick.
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

    /// The most recent row, or `None` when nothing has ever stopped.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] if the file exists but cannot be read: a history
    /// we cannot read is not an empty history, and reading it as one would
    /// repair a failure that is not there.
    pub fn last(&self) -> Outcome<Option<Last>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&self.path).map_err(|e| {
            Halt::Unreadable(format!(
                "error ledger unreadable at {} ({e}) — what last stopped the \
                 harness is unknown",
                self.path.display()
            ))
        })?;
        let at = |cells: &[&str], name: &str| {
            columns()
                .iter()
                .position(|column| *column == name)
                .and_then(|index| cells.get(index))
                .unwrap_or(&"")
                .to_string()
        };
        Ok(text
            .lines()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .last()
            .map(|line| {
                let cells: Vec<&str> = line.split('\t').collect();
                Last {
                    workflow: at(&cells, "workflow"),
                    kind: at(&cells, "kind"),
                    reason: at(&cells, "reason"),
                }
            }))
    }

    fn wrote_nothing(&self, err: &std::io::Error) -> Halt {
        Halt::Failed(format!(
            "cannot write the error ledger {} ({err}) — a repair would be \
             retried at every tick without it",
            self.path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("harness-errors-{tag}"));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }

        fn ledger(&self) -> ErrorLedger {
            ErrorLedger::new(&self.0.join("errors.tsv"))
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn row(kind: &str, reason: &str) -> Row {
        Row {
            when: "2026-10-06T09:00:00Z".to_string(),
            run: "20261006-090000".to_string(),
            workflow: "dev_loop".to_string(),
            kind: kind.to_string(),
            reason: reason.to_string(),
        }
    }

    #[test]
    fn a_halt_becomes_a_row_carrying_its_kind_and_its_own_words() {
        let halt = Halt::Quota("You've hit your session limit".to_string());
        let said = Row::of("2026-10-06T09:00:00Z", "r1", "dev_loop", &halt);
        assert_eq!(said.kind, "QUOTA");
        assert_eq!(said.reason, "You've hit your session limit");
        assert_eq!(said.workflow, "dev_loop");
    }

    #[test]
    fn a_row_has_exactly_the_columns_of_the_header() {
        assert_eq!(row("QUOTA", "out").render().split('\t').count(), 5);
        assert_eq!(columns().len(), 5);
    }

    #[test]
    fn the_header_is_the_frozen_one() {
        assert_eq!(HEADER, "when\trun\tworkflow\tkind\treason");
    }

    #[test]
    fn a_multiline_reason_cannot_split_the_row_in_two() {
        // A Halt's reason is prose from a session: it holds newlines and tabs
        // sooner or later, and one of them would shift every later column.
        let messy = row("FAILED", "line one\nline two\twith a tab");
        let rendered = messy.render();
        assert!(!rendered.contains('\n'));
        assert_eq!(rendered.split('\t').count(), columns().len());
    }

    #[test]
    fn the_last_row_is_what_a_repair_reads() {
        let dir = Dir::new("last");
        let ledger = dir.ledger();
        ledger
            .append(&row("FAILED", "something else"))
            .expect("first");
        ledger
            .append(&row("QUOTA", "session limit"))
            .expect("second");
        let last = ledger.last().expect("read").expect("a row");
        assert_eq!(last.kind, "QUOTA");
        assert_eq!(last.reason, "session limit");
        assert_eq!(last.workflow, "dev_loop");
    }

    #[test]
    fn a_ledger_that_does_not_exist_yet_has_nothing_to_repair() {
        let dir = Dir::new("absent");
        assert_eq!(dir.ledger().last().expect("absent"), None);
    }

    #[test]
    fn the_header_alone_is_not_a_row() {
        let dir = Dir::new("header");
        let path = dir.0.join("errors.tsv");
        std::fs::create_dir_all(&dir.0).expect("a directory");
        std::fs::write(&path, format!("{HEADER}\n")).expect("a ledger");
        assert_eq!(ErrorLedger::new(&path).last().expect("read"), None);
    }
}
