//! The repair the harness performs on itself, and the record it reads it from.
//!
//! Two halves, as everywhere in this crate: the **rule** is pure and lives in
//! [`harness_core::domain::doctor`]; here is the gesture — a `git` in a real
//! checkout, and a row appended to a real ledger.
//!
//! # Why the watch calls it
//!
//! A quota that runs out mid-session leaves files no one committed, and the
//! next run's clean-tree gate refuses on them. The gate is right to refuse —
//! it cannot tell the harness's own leftovers from a human's work in progress.
//! What was missing is anyone able to answer it: the loop reported the failure
//! to its console and tried again, identically, for six hours.
//!
//! So the loop now records why it stopped, and asks this module on the next
//! tick. `harness doctor` is the same code a human can run by hand.
//!
//! # Two entry points, and why only one guards
//!
//! [`treat`] is what the watch calls, between two ticks: no session is running
//! then, by construction — the loop is single-threaded and the workflow that
//! failed has already returned.
//!
//! [`treat_by_hand`] is the CLI's, and it refuses while a session is alive.
//! The repair discards uncommitted files, and a session writes uncommitted
//! files for tens of minutes before committing them: run by hand at the wrong
//! moment, the repair deletes the work of the run it was meant to help. That
//! is not a hypothetical — it happened once, to the author of this module.

use std::path::{Path, PathBuf};
use std::time::Duration;

use harness_core::adapters::shell::disk::RealDisk;
use harness_core::adapters::shell::git::GitCli;
use harness_core::adapters::shell::process;
use harness_core::adapters::store::error_ledger::{self, ErrorLedger};
use harness_core::domain::doctor::{Repair, is_failure, repair_for};
use harness_core::domain::workspace::Workspace;
use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::git::Repo;
use harness_core::traces::Logbook;
use harness_workflows::dev_loop::run as dev_loop;

use crate::adapters::spending;

/// Records why a run stopped, so a later tick can act on it.
///
/// Failing to record is **not** propagated: the run already failed, and losing
/// the note must not replace one failure with another. It is said out loud
/// instead, because a silent loss here disables the repair.
pub fn record(here: &Path, run_id: &str, workflow: &str, halt: &Halt) {
    let ledger = ErrorLedger::new(&Workspace::new(here).error_ledger());
    let row = error_ledger::Row::of(&spending::now(), run_id, workflow, halt);
    if let Err(broke) = ledger.append(&row) {
        eprintln!(
            "watch: the failure could not be recorded: {}",
            broke.reason()
        );
    }
}

/// The binary a session runs under, as a process line would show it.
///
/// Matched loosely on purpose: a false positive makes the doctor refuse, which
/// is the harmless direction. A false negative is what deletes a live
/// session's work.
const SESSION_LINE: &str = "claude -p";

/// Whether a paid session is alive right now.
///
/// A process check rather than a lock file: a lock left behind by a watch that
/// was killed would refuse every repair from then on, which is the deadlock
/// this whole module exists to remove. A process cannot go stale.
///
/// `pgrep` that does not answer counts as "a session is running": not knowing
/// and deciding to delete is exactly the mistake worth preventing here.
async fn session_is_running() -> bool {
    let found = process::run(
        "pgrep",
        &["-f".to_string(), SESSION_LINE.to_string()],
        Path::new("."),
    )
    .await;
    // `pgrep` exits 1 when nothing matches: that is the answer, not a failure.
    // A `pgrep` that could not run at all leaves us unable to know, and the
    // safe reading of "unknown" here is "yes".
    found.map_or(true, |ran| ran.ok())
}

/// [`treat`], for a human at a terminal: refuses while a session is alive.
///
/// # Errors
///
/// [`Halt::Halted`] while a session is running — a correct refusal, naming
/// what to wait for. Otherwise whatever [`treat`] propagates.
pub async fn treat_by_hand(here: &Path, dry_run: bool, log: &Logbook) -> Outcome<Repair> {
    if let Some(refused) = refusal(dry_run, session_is_running().await) {
        return Err(refused);
    }
    treat(here, dry_run, log).await
}

/// Whether to refuse, given whether this is a dry run and whether a session is
/// alive.
///
/// Separated from the probe so the rule is testable without a process table:
/// what matters is that a live session refuses and a dry run never does.
fn refusal(dry_run: bool, session_is_running: bool) -> Option<Halt> {
    // A dry run changes nothing, and "what would you do" is the question a
    // human asks precisely while something is in flight.
    if dry_run || !session_is_running {
        return None;
    }
    Some(Halt::Halted(format!(
        "a paid session is running ({SESSION_LINE}) — repairing now would \
         discard what it has written but not yet committed. Wait for it to \
         finish, or run with --dry-run to see what would be repaired"
    )))
}

/// Reads what last stopped the harness and repairs what it knows how to.
///
/// Returns the repair carried out — [`Repair::Nothing`] when the last row is
/// not a failure this treats, or when there is no history at all.
///
/// **No guard on a running session**: see the module doc. Call
/// [`treat_by_hand`] from anywhere that is not the watch's own loop.
///
/// # Errors
///
/// An unreadable ledger, or a `git` that refuses in a workspace. A repair that
/// half-happened must not be reported as done.
pub async fn treat(here: &Path, dry_run: bool, log: &Logbook) -> Outcome<Repair> {
    let source = Workspace::new(here);
    let ledger = ErrorLedger::new(&source.error_ledger());
    let Some(last) = ledger.last()? else {
        log.debug("doctor: nothing has stopped the harness yet");
        return Ok(Repair::Nothing);
    };
    if !is_failure(&last.kind) {
        // The last row is a repair's own trace. Reading it as a fresh failure
        // is how a loop repairs the same thing at every tick.
        log.debug(&format!(
            "doctor: last row is {} — already treated",
            last.kind
        ));
        return Ok(Repair::Nothing);
    }
    let repair = repair_for(&last.kind, &last.reason);
    if repair == Repair::Nothing {
        log.say(&format!(
            "doctor: {} on {} is not something to repair — {}",
            last.kind, last.workflow, last.reason
        ));
        return Ok(Repair::Nothing);
    }
    log.say(&format!(
        "doctor: {} on {} — {}",
        last.kind, last.workflow, last.reason
    ));
    if dry_run {
        log.say(&format!(
            "doctor: would {} (dry run, nothing done)",
            repair.outcome()
        ));
        return Ok(repair);
    }
    match repair {
        Repair::Nothing => Ok(Repair::Nothing),
        Repair::CleanWorkspace => {
            clean_workspaces(&source, log).await?;
            record_repair(&source, repair, log);
            Ok(repair)
        }
        Repair::InstallDependencies => {
            install_dependencies(&source, log).await?;
            record_repair(&source, repair, log);
            Ok(repair)
        }
    }
}

/// How long an install may run before it counts as hung.
///
/// Twenty minutes: `npm install` on a cold cache, over a slow link, is slow but
/// not broken, and a kill in the middle leaves a half-populated `node_modules`
/// that the gate will then accept.
const INSTALL_TIMEOUT: Duration = Duration::from_mins(20);

/// Installs what a clone cannot carry, in every mounted workspace that lacks it.
///
/// The commands are not read back out of the failure's prose: they are derived
/// from the manifests the checkout carries, by the **same** function the gate
/// uses (`dev_loop::run::installed`). The gate and the repair can then not
/// disagree about what a given clone needs.
///
/// Run through `sh -c` because a remedy is a shell line, `&&` included
/// (`python -m venv .venv && pip install -r requirements.txt`).
async fn install_dependencies(source: &Workspace, log: &Logbook) -> Outcome<()> {
    let mounted = workspaces_under(&source.workspaces());
    if mounted.is_empty() {
        log.say("doctor: no workspace mounted — nothing to install");
        return Ok(());
    }
    let disk = RealDisk;
    for root in mounted {
        for (needs, remedy) in dev_loop::installed(&root, &disk) {
            if root.join(&needs).exists() {
                continue;
            }
            log.say(&format!(
                "doctor: {needs} is missing in {} — running `{remedy}`",
                source.rel(&root)
            ));
            let ran = process::run_for(
                "sh",
                &["-c".to_string(), remedy.clone()],
                &root,
                INSTALL_TIMEOUT,
            )
            .await?;
            if !ran.ok() {
                return Err(Halt::Failed(format!(
                    "`{remedy}` failed in {}: {}",
                    source.rel(&root),
                    ran.why()
                )));
            }
            // The exit code is not the proof. An installer can exit zero having
            // installed nothing — a lockfile for another platform, a registry
            // that answered an empty tree — and reporting that as repaired
            // sends the loop back into the same gate at every tick.
            if !root.join(&needs).exists() {
                return Err(Halt::Failed(format!(
                    "`{remedy}` succeeded in {} and {needs} is still not there \
                     — a human has to look at this one",
                    source.rel(&root)
                )));
            }
            log.say(&format!("doctor: {needs} installed"));
        }
    }
    Ok(())
}

/// Appends the repair's own trace, so the next tick does not redo it.
fn record_repair(source: &Workspace, repair: Repair, log: &Logbook) {
    let row = error_ledger::Row {
        when: spending::now(),
        run: spending::run_id(),
        workflow: error_ledger::DOCTOR.to_string(),
        kind: repair.outcome().to_string(),
        reason: "the loop can take its next tick".to_string(),
    };
    if let Err(broke) = ErrorLedger::new(&source.error_ledger()).append(&row) {
        // Said, not swallowed: without this row the repair runs again at every
        // tick, which is the failure mode the ledger exists to prevent.
        log.warn(&format!(
            "doctor: the repair was done but could not be recorded ({}) — it \
             will be attempted again",
            broke.reason()
        ));
    }
}

/// Puts every mounted workspace back on its own `HEAD`, discarding what an
/// interrupted session left uncommitted.
///
/// `HEAD`, not `origin/<branch>`: the point is to remove the **uncommitted**
/// residue the gate refuses on, and nothing more. What a session committed and
/// pushed stays; the next run resets to `origin` by itself anyway, so doing it
/// here would only destroy more on a guess.
///
/// Every workspace rather than the one that failed: the run id does not say
/// which folder it mounted, and a clean tree is the correct state for all of
/// them. A read-only checkout has nothing to discard, so the gesture is a
/// no-op there.
async fn clean_workspaces(source: &Workspace, log: &Logbook) -> Outcome<()> {
    let mounted = workspaces_under(&source.workspaces());
    if mounted.is_empty() {
        log.say("doctor: no workspace mounted — nothing to clean");
        return Ok(());
    }
    for root in mounted {
        let git = GitCli::new(&root);
        let dirty = git.dirty_files().await?;
        if dirty.is_empty() {
            continue;
        }
        // Said before it is discarded: this is the only trace left of what an
        // interrupted session had written.
        log.say(&format!(
            "doctor: discarding {} uncommitted change(s) in {}",
            dirty.len(),
            source.rel(&root)
        ));
        for line in &dirty {
            log.debug(line);
        }
        let back = git.reset_hard("HEAD").await?;
        if !back.ok() {
            return Err(Halt::Failed(format!(
                "cannot reset {}: {}",
                source.rel(&root),
                back.why()
            )));
        }
        git.clean().await?;
    }
    Ok(())
}

/// The git checkouts directly under `dir`.
///
/// A folder without `.git` is not a workspace this harness made, and is left
/// alone — the same rule provisioning applies before it writes anywhere.
fn workspaces_under(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(".git").exists())
        .collect();
    // Ordered, so a log reads the same way twice.
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("harness-doctor-{tag}"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a test directory");
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_folder_without_git_is_not_a_workspace_to_touch() {
        let dir = Dir::new("not-git");
        std::fs::create_dir_all(dir.0.join("by-hand")).expect("a folder");
        assert!(workspaces_under(&dir.0).is_empty());
    }

    #[test]
    fn a_checkout_is_found_and_a_missing_folder_is_not_an_error() {
        let dir = Dir::new("found");
        std::fs::create_dir_all(dir.0.join("clone/.git")).expect("a clone");
        assert_eq!(workspaces_under(&dir.0), vec![dir.0.join("clone")]);
        assert!(workspaces_under(&dir.0.join("nowhere")).is_empty());
    }

    #[tokio::test]
    async fn a_dry_run_by_hand_is_allowed_even_while_a_session_runs() {
        // It changes nothing, and "what would you do" is the question a human
        // asks precisely while something is in flight.
        let dir = Dir::new("by-hand-dry");
        assert_eq!(
            treat_by_hand(&dir.0, true, &Logbook::null())
                .await
                .expect("a verdict"),
            Repair::Nothing
        );
    }

    #[test]
    fn a_live_session_refuses_and_names_what_to_do_instead() {
        // The mistake this prevents: the repair discarded twelve files a live
        // session had written and not yet committed.
        let err = refusal(false, true).expect("must refuse");
        assert!(matches!(err, Halt::Halted(_)), "{err}");
        assert!(err.reason().contains("--dry-run"), "{}", err.reason());
        assert!(err.reason().contains("not yet committed"));
    }

    #[test]
    fn nothing_running_or_a_dry_run_never_refuses() {
        assert!(refusal(false, false).is_none());
        assert!(refusal(true, true).is_none(), "a dry run changes nothing");
        assert!(refusal(true, false).is_none());
    }

    #[tokio::test]
    async fn the_probe_sees_a_process_carrying_the_session_marker() {
        // A loop, not `sleep`: `sh` exec-replaces a single simple command and
        // the marker would vanish from the process table with it — which is
        // exactly how the first version of this test passed while proving
        // nothing.
        let mut alive = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(format!("while :; do sleep 0.2; done # {SESSION_LINE}"))
            .spawn()
            .expect("a stand-in session");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let seen = session_is_running().await;
        let _ = alive.kill().await;
        assert!(seen, "the probe must see a live session");
    }

    #[tokio::test]
    async fn with_no_history_there_is_nothing_to_repair() {
        let dir = Dir::new("empty");
        let said = treat(&dir.0, false, &Logbook::null())
            .await
            .expect("a verdict");
        assert_eq!(said, Repair::Nothing);
    }

    #[tokio::test]
    async fn a_quota_is_diagnosed_and_a_dry_run_changes_nothing() {
        let dir = Dir::new("quota");
        let source = Workspace::new(&dir.0);
        ErrorLedger::new(&source.error_ledger())
            .append(&error_ledger::Row::of(
                "2026-10-06T09:00:00Z",
                "r1",
                "dev_loop",
                &Halt::Quota("session limit".to_string()),
            ))
            .expect("a row");
        assert_eq!(
            treat(&dir.0, true, &Logbook::null())
                .await
                .expect("a verdict"),
            Repair::CleanWorkspace
        );
        // A dry run must not have written its own trace, or the real repair
        // would then read as already done.
        let last = ErrorLedger::new(&source.error_ledger())
            .last()
            .expect("read")
            .expect("a row");
        assert_eq!(last.kind, "QUOTA");
    }

    #[tokio::test]
    async fn a_repair_records_itself_so_the_next_tick_does_not_redo_it() {
        let dir = Dir::new("recorded");
        let source = Workspace::new(&dir.0);
        ErrorLedger::new(&source.error_ledger())
            .append(&error_ledger::Row::of(
                "2026-10-06T09:00:00Z",
                "r1",
                "dev_loop",
                &Halt::Quota("session limit".to_string()),
            ))
            .expect("a row");
        // No workspace mounted: the gesture is a no-op, the bookkeeping is not.
        assert_eq!(
            treat(&dir.0, false, &Logbook::null())
                .await
                .expect("a verdict"),
            Repair::CleanWorkspace
        );
        let last = ErrorLedger::new(&source.error_ledger())
            .last()
            .expect("read")
            .expect("a row");
        assert_eq!(last.kind, "CLEANED");
        assert_eq!(last.workflow, error_ledger::DOCTOR);
        // And a second pass finds nothing left to do.
        assert_eq!(
            treat(&dir.0, false, &Logbook::null())
                .await
                .expect("a verdict"),
            Repair::Nothing
        );
    }

    #[tokio::test]
    async fn a_workspace_missing_its_dependencies_is_diagnosed_as_an_install() {
        // The deadlock: the clone was deleted, the harness re-cloned it by
        // itself, and a clone carries no `node_modules` — so every later tick
        // stopped on the same gate.
        let dir = Dir::new("deps");
        let source = Workspace::new(&dir.0);
        let said = format!(
            "the workspace has no node_modules — {} . Install them in /w: npm install",
            harness_core::domain::doctor::MISSING_DEPENDENCIES
        );
        ErrorLedger::new(&source.error_ledger())
            .append(&error_ledger::Row::of(
                "2026-10-06T09:00:00Z",
                "r1",
                "dev_loop",
                &Halt::Halted(said),
            ))
            .expect("a row");
        assert_eq!(
            treat(&dir.0, true, &Logbook::null())
                .await
                .expect("a verdict"),
            Repair::InstallDependencies
        );
        // No workspace mounted: nothing to install, and the bookkeeping still
        // says the stop was treated, so the next tick does not redo it.
        assert_eq!(
            treat(&dir.0, false, &Logbook::null())
                .await
                .expect("a verdict"),
            Repair::InstallDependencies
        );
        let last = ErrorLedger::new(&source.error_ledger())
            .last()
            .expect("read")
            .expect("a row");
        assert_eq!(last.kind, "INSTALLED");
        assert_eq!(
            treat(&dir.0, false, &Logbook::null())
                .await
                .expect("a verdict"),
            Repair::Nothing
        );
    }

    #[tokio::test]
    async fn a_voluntary_stop_is_recorded_but_not_repaired() {
        let dir = Dir::new("stop");
        let source = Workspace::new(&dir.0);
        ErrorLedger::new(&source.error_ledger())
            .append(&error_ledger::Row::of(
                "2026-10-06T09:00:00Z",
                "r1",
                "dev_loop",
                &Halt::Halted("the SPEC is ambiguous".to_string()),
            ))
            .expect("a row");
        assert_eq!(
            treat(&dir.0, false, &Logbook::null())
                .await
                .expect("a verdict"),
            Repair::Nothing
        );
        // Untouched: a STOP stays readable as the last thing that happened.
        let last = ErrorLedger::new(&source.error_ledger())
            .last()
            .expect("read")
            .expect("a row");
        assert_eq!(last.kind, "STOP");
    }

    #[test]
    fn recording_a_failure_creates_the_ledger_and_keeps_the_reason() {
        let dir = Dir::new("record");
        record(
            &dir.0,
            "20261006-090000",
            "dev_loop",
            &Halt::Quota("You've hit your session limit".to_string()),
        );
        let last = ErrorLedger::new(&Workspace::new(&dir.0).error_ledger())
            .last()
            .expect("read")
            .expect("a row");
        assert_eq!(last.kind, "QUOTA");
        assert_eq!(last.reason, "You've hit your session limit");
    }
}
