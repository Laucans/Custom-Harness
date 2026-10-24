//! `harness backfill-milestones`: the cost ledger's `milestone` column,
//! filled once on the rows written before it existed.
//!
//! A run writes the milestone of what it spends on; the rows older than the
//! column name none, and the view could only rebuild it from the milestones
//! the board still lists. This reads the board's milestones once, open and
//! closed (`hierarchy::milestone_index`, through the GitHub port), and hands
//! them to the ledger, which rewrites itself safely
//! (`Ledger::backfill_milestones`: a copy first, an atomic rename, nothing
//! touched that already names its milestone).

use std::path::{Path, PathBuf};

use harness_adapters::shell::github::GhCli;
use harness_adapters::store::ledger::{Backfill, Ledger};
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome, Slug};
use harness_core::ports::shell::github::GitHub;
use harness_workflows::common::hierarchy;

use crate::cli::BackfillArgs;

/// Backfills the ledger the arguments name against the target's board, and
/// says what it did.
///
/// # Errors
/// A target that is no repository, then whatever [`run`] reports.
pub async fn by_hand(args: &BackfillArgs, here: &Path) -> Outcome<String> {
    let slug = Slug::parse(&args.target_repo_url).ok_or_else(|| {
        Halt::Failed(format!(
            "{:?} is not a repository whose milestones can be read: set TARGET_REPO_URL",
            args.target_repo_url
        ))
    })?;
    run(&GhCli::for_slug(&slug), &ledger_path(args, here)).await
}

/// Reads every milestone of `gh`, then fills the ledger at `path`.
///
/// The board is read whole before the ledger is opened: a board that cannot
/// be read leaves the ledger as it was.
///
/// # Errors
/// A failed board read; the ledger's own refusal
/// ([`Ledger::backfill_milestones`]).
pub async fn run(gh: &dyn GitHub, path: &Path) -> Outcome<String> {
    let index = hierarchy::milestone_index(gh).await?;
    let done = Ledger::new(path).backfill_milestones(|number| index.get(number).cloned())?;
    Ok(report(path, done, index.len()))
}

/// The ledger the arguments name, or this checkout's own.
fn ledger_path(args: &BackfillArgs, here: &Path) -> PathBuf {
    if args.ledger.is_empty() {
        Workspace::new(here).ledger()
    } else {
        PathBuf::from(&args.ledger)
    }
}

/// What the backfill did, in one line.
fn report(path: &Path, done: Backfill, placed: usize) -> String {
    format!(
        "{}: {} row(s) filled, {} already named, {} on no milestone — {placed} issue(s) placed \
         under a milestone{}",
        path.display(),
        done.filled,
        done.kept,
        done.unresolved,
        if done.filled > 0 {
            " (the ledger as it was is beside it, .bak)"
        } else {
            ""
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::NoGitHub;

    fn args(ledger: &str, target: &str) -> BackfillArgs {
        BackfillArgs {
            ledger: ledger.to_string(),
            target_repo_url: target.to_string(),
        }
    }

    #[test]
    fn the_ledger_is_the_one_named_or_the_checkouts_own() {
        let here = Path::new("/repo");
        assert_eq!(
            ledger_path(&args("/tmp/copy.tsv", ""), here),
            PathBuf::from("/tmp/copy.tsv")
        );
        assert_eq!(
            ledger_path(&args("", ""), here),
            Workspace::new(here).ledger()
        );
    }

    #[test]
    fn the_report_says_each_count_and_where_the_copy_is() {
        let path = Path::new("/l/costs.tsv");
        let said = report(
            path,
            Backfill {
                filled: 3,
                kept: 2,
                unresolved: 1,
            },
            40,
        );
        assert_eq!(
            said,
            "/l/costs.tsv: 3 row(s) filled, 2 already named, 1 on no milestone — 40 issue(s) \
             placed under a milestone (the ledger as it was is beside it, .bak)"
        );
        assert!(!report(path, Backfill::default(), 0).contains(".bak"));
    }

    #[tokio::test]
    async fn no_target_repository_is_refused_before_anything_is_read() {
        let err = by_hand(&args("/nowhere.tsv", ""), Path::new("/repo"))
            .await
            .expect_err("no repository");
        assert!(err.reason().contains("TARGET_REPO_URL"));
    }

    #[tokio::test]
    async fn a_board_that_cannot_be_read_leaves_the_ledger_as_it_was() {
        let dir = std::env::temp_dir().join(format!("harness-backfill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        let path = dir.join("costs.tsv");
        let text = "when\trun\n2026-10-01T00:00:00Z\tr\t01\t42\tcode\n";
        std::fs::write(&path, text).expect("a ledger");
        assert!(run(&NoGitHub, &path).await.is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("re-read"), text);
        assert!(!dir.join("costs.tsv.bak").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
