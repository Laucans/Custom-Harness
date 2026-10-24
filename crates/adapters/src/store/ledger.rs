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

use harness_core::domain::{Halt, Outcome, Spend, Tags, breaker, budget, task_number};

/// The header, frozen character for character.
///
/// `fingerprint`, then `unit`, `profile`, `side` and `milestone`, are the
/// columns added since: they sit **last**, so every row written before they
/// existed is still read correctly — it simply has fewer cells, which read
/// as "no fingerprint known", "no unit declared" and "no milestone known"
/// rather than as a match.
pub const HEADER: &str = "when\trun\tround\ttask\tstage\tcost_usd\tturns\t\
                          duration_ms\tin\tout\tsession\tran_on\tcache_read\t\
                          cache_write\toutcome\tfingerprint\tunit\tprofile\tside\t\
                          milestone";

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
    /// The fingerprint of the prompt this stage sent, so a later run can
    /// recognise the identical session.
    pub fingerprint: String,
    /// What the task is — unit, profile, side, milestone — empty when the
    /// run works on no task, or on one that declares nothing.
    pub tags: Tags,
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
            flat(&self.fingerprint),
            flat(&self.tags.unit),
            flat(&self.tags.profile),
            flat(&self.tags.side),
            flat(&self.tags.milestone),
        ];
        debug_assert_eq!(
            values.len(),
            columns().len(),
            "a row must have exactly the header columns"
        );
        values.join("\t")
    }
}

/// What a milestone backfill did to the rows of a ledger.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Backfill {
    /// Rows whose empty `milestone` cell is now filled.
    pub filled: usize,
    /// Rows that already named their milestone, left as written.
    pub kept: usize,
    /// Rows no milestone holds — a roadmap, a pull request, a run on no
    /// task — left as written, their cell still empty.
    pub unresolved: usize,
}

/// The ledger's text with each empty `milestone` cell filled from
/// `milestone_of`, which answers for an issue number (`61`, never `#61`).
///
/// Pure, so the rule is tested without a disk. A row that names its
/// milestone already is never touched, nor is one `milestone_of` cannot
/// place: running it twice gives the same text. The header is the current
/// one, since a filled row has the cell the old header did not name.
#[must_use]
pub fn backfill_text(
    text: &str,
    milestone_of: impl Fn(&str) -> Option<String>,
) -> (String, Backfill) {
    let columns = columns();
    let at = |name: &str| columns.iter().position(|column| *column == name);
    let (Some(task_at), Some(milestone_at)) = (at("task"), at("milestone")) else {
        return (text.to_string(), Backfill::default());
    };
    let mut done = Backfill::default();
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.lines().enumerate() {
        if index == 0 {
            out.push_str(HEADER);
        } else if line.trim().is_empty() {
            out.push_str(line);
        } else {
            let mut cells: Vec<String> = line.split('\t').map(ToString::to_string).collect();
            let task = cells.get(task_at).map_or("", String::as_str);
            if cells.get(milestone_at).is_some_and(|cell| !cell.is_empty()) {
                done.kept += 1;
                out.push_str(line);
            } else if let Some(milestone) = (cells.len() <= columns.len())
                .then(|| milestone_of(task_number(task)))
                .flatten()
            {
                // A row older than the column is padded up to it.
                cells.resize(columns.len(), String::new());
                if let Some(cell) = cells.get_mut(milestone_at) {
                    *cell = flat(&milestone);
                }
                done.filled += 1;
                out.push_str(&cells.join("\t"));
            } else {
                done.unresolved += 1;
                out.push_str(line);
            }
        }
        out.push('\n');
    }
    (out, done)
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

    /// Every row, read back as it was written — the inverse of
    /// [`Row::render`].
    ///
    /// Tolerant of a row written before a column existed: a missing cell is
    /// empty text or an unobserved number, never a shifted column, because
    /// cells are read by the name the header gives them.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] as [`Self::rows`].
    pub fn read(&self) -> Outcome<Vec<Row>> {
        let columns = columns();
        let at = |name: &str| columns.iter().position(|column| *column == name);
        Ok(self
            .rows()?
            .into_iter()
            .map(|cells| {
                let text = |name: &str| {
                    at(name)
                        .and_then(|i| cells.get(i))
                        .cloned()
                        .unwrap_or_default()
                };
                let number = |name: &str| text(name).parse::<u64>().ok();
                let session = text("session");
                Row {
                    when: text("when"),
                    run: text("run"),
                    round: number("round")
                        .and_then(|n| u32::try_from(n).ok())
                        .unwrap_or(0),
                    task: text("task"),
                    stage: text("stage"),
                    spend: Spend {
                        cost_usd: text("cost_usd").parse::<f64>().ok(),
                        turns: number("turns").and_then(|n| u32::try_from(n).ok()),
                        duration_ms: number("duration_ms"),
                        tokens: harness_core::domain::Tokens {
                            input: number("in"),
                            output: number("out"),
                            cache_read: number("cache_read"),
                            cache_write: number("cache_write"),
                        },
                        session: (!session.is_empty()).then_some(session),
                        quota: None,
                    },
                    ran_on: text("ran_on"),
                    outcome: text("outcome"),
                    fingerprint: text("fingerprint"),
                    tags: Tags {
                        unit: text("unit"),
                        profile: text("profile"),
                        side: text("side"),
                        milestone: text("milestone"),
                    },
                }
            })
            .collect())
    }

    /// The rows as the budget reads them: task, run, stage, cost.
    fn charges(&self) -> Outcome<Vec<budget::Charge>> {
        Ok(self
            .read()?
            .into_iter()
            .map(|row| budget::Charge {
                task: row.task,
                run: row.run,
                stage: row.stage,
                cost_usd: row.spend.cost_usd,
            })
            .collect())
    }

    /// What `task` has cost so far, every run counted, a resumed session
    /// counted once ([`budget::totals`]).
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] as [`Self::rows`].
    pub fn spent_on(&self, task: &str) -> Outcome<f64> {
        Ok(budget::totals(&self.charges()?)
            .get(task)
            .copied()
            .unwrap_or(0.0))
    }

    /// The session the last quota-cut row of this task and stage names,
    /// if it is more recent than any row that finished the stage.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] as [`Self::rows`].
    pub fn interrupted(&self, task: &str, stage: &str) -> Outcome<Option<String>> {
        let last = self
            .read()?
            .into_iter()
            .rfind(|row| row.task == task && row.stage == stage);
        Ok(last.and_then(|row| {
            (row.outcome == breaker::QUOTA)
                .then_some(row.spend.session)
                .flatten()
                .filter(|id| !id.is_empty())
        }))
    }

    /// What tasks tagged `unit` and `profile` have cost, as a distribution
    /// of their totals; either empty means "any".
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] as [`Self::rows`].
    pub fn typical(&self, unit: &str, profile: &str) -> Outcome<Option<budget::Typical>> {
        let rows = self.read()?;
        let tagged: Vec<&Row> = rows
            .iter()
            .filter(|row| unit.is_empty() || row.tags.unit == unit)
            .filter(|row| profile.is_empty() || row.tags.profile == profile)
            .collect();
        let charges: Vec<budget::Charge> = tagged
            .iter()
            .map(|row| budget::Charge {
                task: row.task.clone(),
                run: row.run.clone(),
                stage: row.stage.clone(),
                cost_usd: row.spend.cost_usd,
            })
            .collect();
        let totals: Vec<f64> = budget::totals(&charges).into_values().collect();
        Ok(budget::typical(&totals))
    }

    /// How many times this exact session already ended in error.
    ///
    /// The identity is the three cells `task`, `stage` and `fingerprint`;
    /// the rule applied to the outcomes found is
    /// [`breaker::trailing_failures`].
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] if the ledger exists and cannot be read — the
    /// caller is about to pay, and an unreadable history is not an empty one.
    pub fn failures(&self, task: &str, stage: &str, fingerprint: &str) -> Outcome<u32> {
        let columns = columns();
        let at = |name: &str| columns.iter().position(|column| *column == name);
        let (Some(task_at), Some(stage_at), Some(outcome_at), Some(fingerprint_at)) =
            (at("task"), at("stage"), at("outcome"), at("fingerprint"))
        else {
            return Ok(0);
        };
        let outcomes: Vec<String> = self
            .rows()?
            .into_iter()
            .filter(|row| {
                // `.get`, not indexing: a row written before `fingerprint`
                // existed is shorter, and must read as no match rather than
                // panic.
                let cell = |at: usize| row.get(at).map_or("", String::as_str);
                cell(task_at) == task
                    && cell(stage_at) == stage
                    && cell(fingerprint_at) == fingerprint
            })
            .map(|row| row.get(outcome_at).cloned().unwrap_or_default())
            .collect();
        Ok(breaker::trailing_failures(&outcomes))
    }

    /// Fills each row's empty `milestone` cell from `milestone_of`
    /// ([`backfill_text`]), once, for the rows written before the column
    /// existed.
    ///
    /// The budget reads this file, so a row lost would make it lie: the
    /// original is copied to `<ledger>.bak` first, the new text is written
    /// beside it and renamed over it in one step, and the rename is refused
    /// if the ledger grew meanwhile — a run appended a row the new text does
    /// not hold. Nothing to fill writes nothing, not even the copy.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] as [`Self::rows`]; [`Halt::Failed`] if the copy,
    /// the new text or the rename could not be written, or the ledger grew
    /// while it was rewritten.
    pub fn backfill_milestones(
        &self,
        milestone_of: impl Fn(&str) -> Option<String>,
    ) -> Outcome<Backfill> {
        if !self.path.exists() {
            return Ok(Backfill::default());
        }
        let text = std::fs::read_to_string(&self.path).map_err(|e| {
            Halt::Unreadable(format!(
                "ledger unreadable at {} ({e}) — nothing was backfilled",
                self.path.display()
            ))
        })?;
        let (filled, done) = backfill_text(&text, milestone_of);
        if done.filled == 0 {
            return Ok(done);
        }
        let backup = self.beside("bak");
        std::fs::copy(&self.path, &backup).map_err(|e| self.wrote_nothing(&e))?;
        let fresh = self.beside("tmp");
        let written = std::fs::File::create(&fresh).and_then(|mut file| {
            file.write_all(filled.as_bytes())?;
            file.sync_all()
        });
        if let Err(e) = written {
            let _ = std::fs::remove_file(&fresh);
            return Err(self.wrote_nothing(&e));
        }
        let now = std::fs::metadata(&self.path).map(|meta| meta.len()).ok();
        if now != u64::try_from(text.len()).ok() {
            let _ = std::fs::remove_file(&fresh);
            return Err(Halt::Failed(format!(
                "ledger {} grew while it was backfilled — a run is writing to \
                 it; stop it and run the backfill again, nothing was changed",
                self.path.display()
            )));
        }
        std::fs::rename(&fresh, &self.path).map_err(|e| {
            let _ = std::fs::remove_file(&fresh);
            self.wrote_nothing(&e)
        })?;
        Ok(done)
    }

    /// `<ledger>.<suffix>`, in the ledger's own directory — so a rename
    /// from it never crosses a file system.
    fn beside(&self, suffix: &str) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(".");
        name.push(suffix);
        PathBuf::from(name)
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
    use harness_core::domain::Tokens;

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
                // The ledger has no column for it: a reading is state the next
                // run reads, not a row in a history.
                quota: None,
            },
            ran_on: "macbook".to_string(),
            outcome: "ok".to_string(),
            fingerprint: "0123456789abcdef".to_string(),
            tags: Tags {
                unit: "capability".to_string(),
                profile: "mid".to_string(),
                side: "harness:read-side".to_string(),
                milestone: "#12 Billing".to_string(),
            },
        }
    }

    #[test]
    fn a_row_has_exactly_the_columns_of_the_header() {
        assert_eq!(row().render().split('\t').count(), columns().len());
    }

    #[test]
    fn the_header_is_the_frozen_one_and_grew_only_at_the_end() {
        // Frozen: columns and their order are a disk format. A new column is
        // appended, never inserted, so old rows stay readable as-is.
        assert!(HEADER.starts_with(
            "when\trun\tround\ttask\tstage\tcost_usd\tturns\tduration_ms\tin\t\
             out\tsession\tran_on\tcache_read\tcache_write\toutcome"
        ));
        assert_eq!(
            &columns()[15..],
            ["fingerprint", "unit", "profile", "side", "milestone"],
            "the columns added since the Python ledger, in the order they came"
        );
        assert_eq!(columns().len(), 20);
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
        assert_eq!(at("unit"), "capability");
        assert_eq!(at("profile"), "mid");
        assert_eq!(at("side"), "harness:read-side");
        assert_eq!(at("milestone"), "#12 Billing");
    }

    #[test]
    fn a_run_on_no_task_leaves_the_unit_columns_empty_not_invented() {
        let untagged = Row {
            tags: Tags::default(),
            ..row()
        };
        let rendered = untagged.render();
        let cells: Vec<&str> = rendered.split('\t').collect();
        assert_eq!(cells.len(), columns().len());
        assert_eq!(&cells[16..], ["", "", "", ""]);
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

    fn tagged(task: &str, run: &str, stage: &str, cost: f64, outcome: &str, unit: &str) -> Row {
        Row {
            task: task.to_string(),
            run: run.to_string(),
            stage: stage.to_string(),
            spend: Spend {
                cost_usd: Some(cost),
                session: Some(format!("sess-{task}-{stage}")),
                ..Spend::default()
            },
            outcome: outcome.to_string(),
            tags: Tags {
                unit: unit.to_string(),
                profile: "mid".to_string(),
                side: String::new(),
                milestone: String::new(),
            },
            ..row()
        }
    }

    #[test]
    fn what_a_task_spent_counts_a_resumed_session_once_and_every_run() {
        let dir = Dir::new("spent");
        let ledger = dir.ledger();
        ledger
            .append(&tagged("61", "r1", "code", 1.0, "ok", "capability"))
            .expect("a row");
        ledger
            .append(&tagged("61", "r1", "code", 2.5, "ok", "capability"))
            .expect("a row");
        ledger
            .append(&tagged("61", "r2", "create-test", 0.5, "ok", "capability"))
            .expect("a row");
        ledger
            .append(&tagged("62", "r3", "code", 9.0, "ok", "aggregate"))
            .expect("a row");
        let spent = ledger.spent_on("61").expect("read");
        assert!((spent - 3.0).abs() < 1e-9, "{spent}");
        assert!(ledger.spent_on("99").expect("read").abs() < f64::EPSILON);
    }

    #[test]
    fn the_typical_cost_of_a_unit_is_read_from_its_tagged_tasks() {
        let dir = Dir::new("typical");
        let ledger = dir.ledger();
        for (task, cost) in [("1", 2.0), ("2", 4.0), ("3", 6.0)] {
            ledger
                .append(&tagged(task, "r", "code", cost, "ok", "capability"))
                .expect("a row");
        }
        ledger
            .append(&tagged("4", "r", "code", 40.0, "ok", "aggregate"))
            .expect("a row");
        let found = ledger
            .typical("capability", "mid")
            .expect("read")
            .expect("three tasks");
        assert_eq!(found.count, 3);
        assert!((found.median - 4.0).abs() < 1e-9);
        assert!((found.p75 - 6.0).abs() < 1e-9);
        let any = ledger.typical("", "").expect("read").expect("four tasks");
        assert_eq!(any.count, 4);
        assert!(ledger.typical("micro-ui", "").expect("read").is_none());
    }

    #[test]
    fn an_interrupted_stage_names_its_session_until_a_later_row_finishes_it() {
        let dir = Dir::new("interrupted");
        let ledger = dir.ledger();
        ledger
            .append(&tagged(
                "61",
                "r1",
                "code",
                1.0,
                breaker::QUOTA,
                "capability",
            ))
            .expect("a row");
        assert_eq!(
            ledger.interrupted("61", "code").expect("read").as_deref(),
            Some("sess-61-code")
        );
        assert_eq!(ledger.interrupted("61", "create-test").expect("read"), None);
        ledger
            .append(&tagged("61", "r2", "code", 3.0, "ok", "capability"))
            .expect("a row");
        assert_eq!(
            ledger.interrupted("61", "code").expect("read"),
            None,
            "finished since"
        );
        let mut cut_without_id = tagged("61", "r3", "code", 1.0, breaker::QUOTA, "capability");
        cut_without_id.spend.session = None;
        ledger.append(&cut_without_id).expect("a row");
        assert_eq!(
            ledger.interrupted("61", "code").expect("read"),
            None,
            "nothing to resume"
        );
    }

    #[test]
    fn a_row_read_back_is_the_row_that_was_written() {
        let dir = Dir::new("roundtrip");
        let ledger = dir.ledger();
        ledger.append(&row()).expect("a row");
        let back = ledger.read().expect("re-read");
        assert_eq!(back.len(), 1);
        let got = &back[0];
        assert_eq!(got.when, "2026-09-30T12:00:00Z");
        assert_eq!(got.run, "run-1");
        assert_eq!(got.round, 3);
        assert_eq!(got.task, "42");
        assert_eq!(got.stage, "code");
        assert_eq!(got.spend.cost_usd, Some(1.5));
        assert_eq!(got.spend.turns, Some(7));
        assert_eq!(got.spend.duration_ms, Some(45_000));
        assert_eq!(got.spend.tokens.input, Some(100));
        assert_eq!(got.spend.tokens.output, Some(20));
        assert_eq!(got.spend.tokens.cache_read, Some(5));
        assert_eq!(got.spend.tokens.cache_write, Some(6));
        assert_eq!(got.spend.session.as_deref(), Some("sess-42"));
        assert_eq!(got.ran_on, "macbook");
        assert_eq!(got.outcome, "ok");
        assert_eq!(got.fingerprint, "0123456789abcdef");
        assert_eq!(got.tags.unit, "capability");
        assert_eq!(got.tags.profile, "mid");
        assert_eq!(got.tags.side, "harness:read-side");
        assert_eq!(got.tags.milestone, "#12 Billing");
    }

    #[test]
    fn a_row_written_before_the_milestone_column_reads_as_no_milestone() {
        let dir = Dir::new("short-milestone");
        let path = dir.0.join("costs.tsv");
        std::fs::create_dir_all(&dir.0).expect("a directory");
        // The 19 cells of the header before `milestone`.
        let old = format!(
            "{HEADER}\n2026-09-30T12:00:00Z\trun-1\t03\t42\tcode\t1.500000\t\t\t\t\t\tmac\t\t\tok\t\
             0123456789abcdef\tcapability\tmid\tharness:read-side\n"
        );
        std::fs::write(&path, old).expect("a ledger");
        let back = Ledger::new(&path).read().expect("re-read");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].tags.side, "harness:read-side", "no shifted column");
        assert_eq!(back[0].tags.milestone, "", "not declared, never guessed");
    }

    /// A ledger as it stood before the column: an old header, a 15-cell row
    /// of task 42, a 19-cell row of `#43`, one of a pull request, one that
    /// already names its milestone.
    fn legacy() -> String {
        let named = Row {
            task: "44".to_string(),
            ..row()
        };
        format!(
            "when\trun\n\
             2026-09-30T12:00:00Z\trun-1\t03\t42\tcode\t\t\t\t\t\t\tmac\t\t\tSTOP\n\
             2026-09-30T12:00:00Z\trun-1\t01\t#43\twrite\t0.500000\t\t\t\t\t\tmac\t\t\tok\t\
             f\t\t\t\n\
             2026-09-30T12:00:00Z\trun-2\t01\t#105\tfix\t2.000000\t\t\t\t\t\tmac\t\t\tok\n\
             {}\n",
            named.render()
        )
    }

    fn board(number: &str) -> Option<String> {
        match number {
            "42" | "43" | "12" => Some("#12 Billing".to_string()),
            "44" => Some("#99 Elsewhere".to_string()),
            _ => None,
        }
    }

    #[test]
    fn a_backfill_fills_the_empty_cells_from_the_task_with_or_without_its_hash() {
        let (text, done) = backfill_text(&legacy(), board);
        assert_eq!(
            done,
            Backfill {
                filled: 2,
                kept: 1,
                unresolved: 1
            }
        );
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some(HEADER), "the current header");
        let milestone_at = columns().len() - 1;
        let cells = |line: &str| line.split('\t').map(str::to_string).collect::<Vec<_>>();
        let first = cells(lines.next().expect("task 42"));
        assert_eq!(first.len(), columns().len(), "padded up to the column");
        assert_eq!(first[milestone_at], "#12 Billing");
        assert_eq!(first[14], "STOP", "the old cells stay where they were");
        let second = cells(lines.next().expect("task #43"));
        assert_eq!(second[milestone_at], "#12 Billing");
        assert_eq!(second[5], "0.500000");
        let pull = lines.next().expect("the pull request");
        assert_eq!(
            cells(pull).len(),
            15,
            "a row nobody places is left as written"
        );
        let named = cells(lines.next().expect("the named row"));
        assert_eq!(
            named[milestone_at], "#12 Billing",
            "a named row is never rewritten"
        );
        assert_eq!(lines.next(), None, "no row lost, none added");
    }

    #[test]
    fn a_row_as_long_as_the_header_with_an_empty_cell_is_filled() {
        let unnamed = Row {
            tags: Tags::default(),
            ..row()
        };
        let (text, done) = backfill_text(&format!("{HEADER}\n{}\n", unnamed.render()), board);
        assert_eq!(done.filled, 1);
        assert!(text.ends_with("\t#12 Billing\n"), "{text}");
    }

    #[test]
    fn a_row_longer_than_the_header_is_left_alone() {
        let plain = Row {
            tags: Tags::default(),
            ..row()
        };
        let long = format!("{HEADER}\n{}\textra\n", plain.render());
        let (text, done) = backfill_text(&long, board);
        assert_eq!(done.unresolved, 1);
        assert_eq!(text, long);
    }

    #[test]
    fn a_blank_line_is_kept_and_counted_as_nothing() {
        let (text, done) = backfill_text(&format!("{HEADER}\n\n"), board);
        assert_eq!(done, Backfill::default());
        assert_eq!(text, format!("{HEADER}\n\n"));
    }

    #[test]
    fn a_backfill_keeps_a_copy_and_a_second_run_changes_nothing() {
        let dir = Dir::new("backfill");
        std::fs::create_dir_all(&dir.0).expect("a directory");
        let path = dir.0.join("costs.tsv");
        std::fs::write(&path, legacy()).expect("a ledger");
        let ledger = Ledger::new(&path);

        let first = ledger.backfill_milestones(board).expect("backfilled");
        assert_eq!(first.filled, 2);
        let backup = std::fs::read_to_string(dir.0.join("costs.tsv.bak")).expect("a copy");
        assert_eq!(backup, legacy(), "the copy is the ledger as it was");
        assert!(
            !dir.0.join("costs.tsv.tmp").exists(),
            "renamed, not left behind"
        );
        let after = std::fs::read_to_string(&path).expect("re-read");
        let rows = ledger.read().expect("re-read");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].tags.milestone, "#12 Billing");

        let second = ledger.backfill_milestones(board).expect("backfilled");
        assert_eq!(
            second,
            Backfill {
                filled: 0,
                kept: 3,
                unresolved: 1
            }
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("re-read"),
            after,
            "idempotent"
        );
    }

    #[test]
    fn a_backfill_with_nothing_to_fill_writes_nothing() {
        let dir = Dir::new("backfill-none");
        std::fs::create_dir_all(&dir.0).expect("a directory");
        let path = dir.0.join("costs.tsv");
        std::fs::write(&path, legacy()).expect("a ledger");
        let done = Ledger::new(&path)
            .backfill_milestones(|_| None)
            .expect("read");
        assert_eq!(done.filled, 0);
        assert!(!dir.0.join("costs.tsv.bak").exists());
        assert_eq!(std::fs::read_to_string(&path).expect("re-read"), legacy());
        assert_eq!(
            Ledger::new(&dir.0.join("absent.tsv"))
                .backfill_milestones(board)
                .expect("absent"),
            Backfill::default()
        );
    }

    #[test]
    fn a_short_row_reads_back_with_nothing_observed_and_no_unit() {
        let dir = Dir::new("short-read");
        let path = dir.0.join("costs.tsv");
        std::fs::create_dir_all(&dir.0).expect("a directory");
        // The 15 cells of the first header: no fingerprint, no unit.
        let old = format!(
            "{HEADER}\n2026-09-30T12:00:00Z\trun-1\t03\t42\tcode\t\t\t\t\t\t\tmac\t\t\tSTOP\n"
        );
        std::fs::write(&path, old).expect("a ledger");
        let back = Ledger::new(&path).read().expect("re-read");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].spend.cost_usd, None, "unobserved, not zero");
        assert_eq!(back[0].spend.session, None);
        assert_eq!(back[0].outcome, "STOP");
        assert_eq!(back[0].fingerprint, "");
        assert_eq!(back[0].tags, Tags::default());
    }

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("harness-breaker-{tag}"));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }

        fn ledger(&self) -> Ledger {
            Ledger::new(&self.0.join("costs.tsv"))
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn attempt(outcome: &str, fingerprint: &str) -> Row {
        Row {
            outcome: outcome.to_string(),
            fingerprint: fingerprint.to_string(),
            ..row()
        }
    }

    #[test]
    fn identical_failures_are_counted_for_the_session_that_made_them() {
        let dir = Dir::new("counted");
        let ledger = dir.ledger();
        for _ in 0..3 {
            ledger
                .append(&attempt("STOP", "deadbeefdeadbeef"))
                .expect("a row");
        }
        assert_eq!(
            ledger
                .failures("42", "code", "deadbeefdeadbeef")
                .expect("read"),
            3
        );
    }

    #[test]
    fn a_different_prompt_stage_or_task_is_a_different_session() {
        let dir = Dir::new("identity");
        let ledger = dir.ledger();
        for _ in 0..3 {
            ledger
                .append(&attempt("STOP", "deadbeefdeadbeef"))
                .expect("a row");
        }
        // The gesture that resolves the ambiguity rewrites the issue, so the
        // prompt — and the fingerprint — are no longer the ones that failed.
        assert_eq!(
            ledger
                .failures("42", "code", "0000000000000000")
                .expect("read"),
            0
        );
        assert_eq!(
            ledger
                .failures("42", "create-test", "deadbeefdeadbeef")
                .expect("read"),
            0
        );
        assert_eq!(
            ledger
                .failures("43", "code", "deadbeefdeadbeef")
                .expect("read"),
            0
        );
    }

    #[test]
    fn a_row_written_before_the_column_existed_matches_no_session() {
        let dir = Dir::new("legacy");
        let path = dir.0.join("costs.tsv");
        std::fs::create_dir_all(&dir.0).expect("a directory");
        // 15 cells: exactly what the previous header wrote.
        let old = format!(
            "{HEADER}\n2026-09-30T12:00:00Z\trun-1\t03\t42\tcode\t\t\t\t\t\t\tmac\t\t\tSTOP\n"
        );
        std::fs::write(&path, old).expect("a ledger");
        let ledger = Ledger::new(&path);
        assert_eq!(ledger.rows().expect("re-read").len(), 1);
        assert_eq!(
            ledger
                .failures("42", "code", "deadbeefdeadbeef")
                .expect("read"),
            0,
            "an empty cell must never read as a match"
        );
    }

    #[test]
    fn a_ledger_that_does_not_exist_yet_reads_as_no_rows() {
        let path = std::env::temp_dir().join("harness-ledger-absent/nowhere.tsv");
        assert_eq!(
            Ledger::new(&path).rows().expect("absent"),
            [] as [std::vec::Vec<std::string::String>; 0]
        );
    }
}
