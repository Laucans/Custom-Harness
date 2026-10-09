//! A prompt's dry-run on disk: `.llocal/trials/<id>/`.
//!
//! `harness trial` writes `trial.json` when it starts and again when it ends,
//! and its session's traces (`session.log`, `stream.jsonl`, `prompts.md`)
//! beside it; the view reads them back. Nothing a run reads is here: a trial
//! is the human's question, answered once.

use std::path::Path;

use harness_core::domain::{Halt, Outcome};

/// The folder of every trial, under `.llocal/`.
pub const TRIALS: &str = "trials";

/// The report, in a trial's folder.
pub const REPORT: &str = "trial.json";

pub use harness_core::domain::prompt_book::{TrialReport as Report, TrialState as State};

/// Writes a trial's report, aside then renamed.
///
/// # Errors
///
/// [`Halt::Failed`] if it could not be written.
pub fn write_report(dir: &Path, report: &Report) -> Outcome<()> {
    let failed = |e: &dyn std::fmt::Display| {
        Halt::Failed(format!(
            "couldn't write the trial report in {}: {e}",
            dir.display()
        ))
    };
    std::fs::create_dir_all(dir).map_err(|e| failed(&e))?;
    let text = serde_json::to_string_pretty(report).map_err(|e| failed(&e))?;
    let aside = dir.join(format!("{REPORT}.new"));
    std::fs::write(&aside, text).map_err(|e| failed(&e))?;
    std::fs::rename(&aside, dir.join(REPORT)).map_err(|e| failed(&e))
}

/// A trial's report, `None` when it has none or it does not decode.
#[must_use]
pub fn read_report(dir: &Path) -> Option<Report> {
    let text = std::fs::read_to_string(dir.join(REPORT)).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_round_trips() {
        let dir = std::env::temp_dir().join(format!("harness-trial-{}", std::process::id()));
        let report = Report {
            state: State::Done,
            key: "split/slice".to_string(),
            output: Some("ok".to_string()),
            cost_usd: Some(0.12),
            ..Report::default()
        };
        write_report(&dir, &report).expect("written");
        assert_eq!(read_report(&dir), Some(report));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
