//! What to do about a run that stopped: the rule, not the gesture.
//!
//! Pure. Given what the error ledger last recorded, this says which repair is
//! called for; carrying it out needs a checkout and a `git`, and lives with
//! the launcher.
//!
//! # Two rules, deliberately
//!
//! An exhausted quota is the only *failure* that is **certainly not the
//! harness's fault and certainly leaves a mess**: the window closes
//! mid-sentence, so the session had no chance to commit what it had written,
//! and the uncommitted files it leaves make the next run's clean-tree gate
//! refuse — at every tick, until a human intervenes. That deadlock is what
//! [`Repair::CleanWorkspace`] repairs.
//!
//! A workspace with no dependencies installed is the second, and it is a
//! `STOP` rather than a failure: the gate is *right* to refuse, and the thing
//! to do about it is not to overrule it but to carry out the very command it
//! names. It is the deadlock a deleted clone leaves behind — the harness
//! re-clones by itself, and a clone carries no `node_modules`, so every later
//! tick stops on the same sentence. See [`Repair::InstallDependencies`].
//!
//! What is still left alone:
//!
//! - every other `STOP` is a correct outcome. The session chose to stop and
//!   wrote nothing it did not mean to; repairing it would mean overwriting a
//!   refusal.
//! - `FAILED` is a stage that rendered nothing usable, and **we do not know
//!   what state it left**. Cleaning on a guess would destroy the evidence of
//!   the only failure class worth a human's attention.
//!
//! Adding a rule here means being able to say the same two things about it:
//! what it is certain of, and what it would destroy if wrong.

use crate::domain::breaker;

/// A repair the harness can carry out on itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repair {
    /// Nothing known to do — the failure is not one this knows how to treat.
    Nothing,
    /// Put the workspace back on its branch, discarding what an interrupted
    /// session left uncommitted.
    ///
    /// What is discarded is the harness's own unfinished attempt, in a clone
    /// it owns and re-mounts every run; the next session redoes it from the
    /// issue.
    CleanWorkspace,
    /// Install what a clone cannot carry — `node_modules`, a venv — by running
    /// the very command the gate named.
    ///
    /// Nothing is destroyed: these paths are gitignored by construction (that
    /// is *why* a clone lacks them), and the command is the project's own
    /// install command, derived from the manifests the checkout carries.
    InstallDependencies,
}

/// The words a missing-dependencies refusal carries, so a repair can recognise
/// one.
///
/// A marker, not a sentence to parse: the gate writes it into its own prose
/// (`dev_loop::checks::preflight`), this module matches on it, and both sides
/// have a test holding them together. The repair then re-derives *what* to
/// install from the checkout rather than reading it back out of the message —
/// prose is for the human, the manifests are the fact.
pub const MISSING_DEPENDENCIES: &str = "nothing a clone carries";

impl Repair {
    /// What the ledger should record when this repair is carried out.
    #[must_use]
    pub const fn outcome(self) -> &'static str {
        match self {
            Self::Nothing => "NOTHING",
            Self::CleanWorkspace => "CLEANED",
            Self::InstallDependencies => "INSTALLED",
        }
    }
}

/// The repair that this stop calls for.
///
/// `kind` is the ledger's own word — what [`Halt::prefix`](crate::domain::Halt)
/// wrote; `reason` is the row's own words, which is where a `STOP` says *which*
/// stop it is.
#[must_use]
pub fn repair_for(kind: &str, reason: &str) -> Repair {
    if kind == breaker::QUOTA {
        return Repair::CleanWorkspace;
    }
    if kind == breaker::STOP && reason.contains(MISSING_DEPENDENCIES) {
        return Repair::InstallDependencies;
    }
    Repair::Nothing
}

/// Whether this ledger row is a failure at all, rather than a repair's own
/// trace.
///
/// A repair appends its conclusion to the same ledger, and reading that back
/// as a fresh failure is how a loop repairs the same thing forever.
#[must_use]
pub fn is_failure(kind: &str) -> bool {
    breaker::is_error(kind) || kind == breaker::QUOTA
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Halt;

    /// The kind of a halt, as the ledger writes it.
    fn kind(halt: &Halt) -> &'static str {
        halt.prefix()
    }

    #[test]
    fn an_exhausted_quota_calls_for_a_clean_workspace() {
        // The deadlock this exists for: the window closed mid-write, the files
        // stayed uncommitted, and every later tick refused on them.
        assert_eq!(
            repair_for(kind(&Halt::Quota(String::new())), "session limit"),
            Repair::CleanWorkspace
        );
    }

    #[test]
    fn a_workspace_without_its_dependencies_calls_for_the_install() {
        // The deadlock a deleted clone leaves: the harness re-clones by
        // itself, a clone carries no `node_modules`, and every later tick
        // stops on the same sentence.
        let said = format!(
            "the workspace has no node_modules — {MISSING_DEPENDENCIES}. \
             Install them in /w: npm install"
        );
        assert_eq!(
            repair_for(kind(&Halt::Halted(String::new())), &said),
            Repair::InstallDependencies
        );
    }

    #[test]
    fn a_voluntary_stop_is_not_repaired_because_it_is_a_correct_outcome() {
        assert_eq!(
            repair_for(
                kind(&Halt::Halted(String::new())),
                "the SPEC is ambiguous: which folder holds a session?"
            ),
            Repair::Nothing
        );
    }

    #[test]
    fn a_failure_is_left_alone_because_its_state_is_unknown() {
        // Cleaning on a guess would destroy the evidence of the only failure
        // class worth a human's attention. Even saying the dependency words:
        // a FAILED stage did not reach that gate.
        assert_eq!(
            repair_for(kind(&Halt::Failed(String::new())), MISSING_DEPENDENCIES),
            Repair::Nothing
        );
    }

    #[test]
    fn an_unknown_word_is_not_treated() {
        assert_eq!(repair_for("", ""), Repair::Nothing);
        assert_eq!(repair_for("ok", MISSING_DEPENDENCIES), Repair::Nothing);
    }

    #[test]
    fn a_repairs_own_trace_is_not_read_back_as_a_fresh_failure() {
        // Otherwise the loop repairs the same thing at every tick.
        assert!(!is_failure(Repair::CleanWorkspace.outcome()));
        assert!(!is_failure(Repair::Nothing.outcome()));
        assert!(is_failure(Halt::Quota(String::new()).prefix()));
        assert!(is_failure(Halt::Failed(String::new()).prefix()));
    }

    #[test]
    fn each_repair_has_its_own_word_in_the_ledger() {
        let words = [
            Repair::Nothing.outcome(),
            Repair::CleanWorkspace.outcome(),
            Repair::InstallDependencies.outcome(),
        ];
        for (at, word) in words.iter().enumerate() {
            assert!(!words[at + 1..].contains(word), "{word} names two repairs");
            // And none of them reads back as a fresh failure.
            assert!(!is_failure(word), "{word} would be repaired again");
        }
    }
}
