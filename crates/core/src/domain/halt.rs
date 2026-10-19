//! Stops, as values: `Halt` replaces the exception.

use thiserror::Error;

use crate::domain::breaker;

/// The level at which a stop deserves to be logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A correct result, not a problem.
    Info,
    /// The subscription window is exhausted — neither repaired nor abandoned.
    Warn,
    /// A stage rendered nothing usable.
    Error,
}

/// What a session's halt says when its carrier killed it for its silence.
///
/// The process adapter's idle kill (`<binary> printed nothing for <n>s and
/// was killed as hung`). One text, read back by [`Halt::killed_as_hung`]: a
/// round that finds the session's pull request already open treats such a
/// halt as a delivery, not as a failed round.
pub const KILLED_AS_HUNG: &str = "was killed as hung";

/// The four ways a run stops without raising.
///
/// Exit codes, prefixes and levels are those already read by the Python
/// pipeline's external scheduler — unchanged by the migration: an external
/// scheduler reads them, and moving them would be a hidden contract change
/// disguised as refactoring.
#[derive(Debug, Clone, Error)]
pub enum Halt {
    /// Voluntary stop: the loop refuses to guess.
    #[error("{0}")]
    Halted(String),
    /// A store (GitHub, the resume point) didn't answer.
    ///
    /// Separate from [`Halt::Halted`]: reading "unreadable" as "nothing to do"
    /// would charge a `/planner` for an expired token.
    #[error("{0}")]
    Unreadable(String),
    /// A stage rendered nothing usable. Not a correct result.
    #[error("{0}")]
    Failed(String),
    /// The subscription window is exhausted — come back later, unchanged.
    #[error("{0}")]
    Quota(String),
}

impl Halt {
    /// The code that an external scheduler reads.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Halted(_) | Self::Unreadable(_) => 1,
            Self::Failed(_) => 2,
            Self::Quota(_) => 3,
        }
    }

    /// How the line announces itself in the log (`STOP`, `FAILED`, `QUOTA`).
    ///
    /// The same word lands in the ledger's `outcome` column, which
    /// [`breaker`](crate::domain::breaker) reads back to count identical
    /// failures — hence the shared constants rather than literals here.
    #[must_use]
    pub const fn prefix(&self) -> &'static str {
        match self {
            Self::Halted(_) | Self::Unreadable(_) => breaker::STOP,
            Self::Failed(_) => breaker::FAILED,
            Self::Quota(_) => breaker::QUOTA,
        }
    }

    /// The level at which to log this stop.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        match self {
            Self::Halted(_) | Self::Unreadable(_) => Severity::Info,
            Self::Failed(_) => Severity::Error,
            Self::Quota(_) => Severity::Warn,
        }
    }

    /// Whether this is a session killed for its silence ([`KILLED_AS_HUNG`]):
    /// a failure of the session, which may still have delivered.
    #[must_use]
    pub fn killed_as_hung(&self) -> bool {
        matches!(self, Self::Failed(reason) if reason.contains(KILLED_AS_HUNG))
    }

    /// The reason carried, whatever the variant.
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Halted(r) | Self::Unreadable(r) | Self::Failed(r) | Self::Quota(r) => r,
        }
    }

    /// The same stop, with the encompassing reason added in front.
    ///
    /// The original reason is kept behind: what broke below and what that
    /// prevented above are two halves of the same sentence, and an autopsy
    /// needs both.
    #[must_use]
    pub fn but(self, context: &str) -> Self {
        let said = format!("{context} ({})", self.reason());
        match self {
            Self::Halted(_) => Self::Halted(said),
            Self::Unreadable(_) => Self::Unreadable(said),
            Self::Failed(_) => Self::Failed(said),
            Self::Quota(_) => Self::Quota(said),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journal_calls_an_error_what_the_halt_calls_one() {
        // `traces` reads the kind as text: a renamed prefix would turn every
        // failure grey in the journal without a test to say so.
        for halt in [
            Halt::Halted(String::new()),
            Halt::Unreadable(String::new()),
            Halt::Failed(String::new()),
            Halt::Quota(String::new()),
        ] {
            let told = crate::traces::Event::Halted {
                workflow: "w".to_string(),
                kind: halt.prefix().to_string(),
                reason: String::new(),
            };
            assert_eq!(
                told.level() == crate::traces::Level::Error,
                halt.severity() == Severity::Error,
                "{halt:?}"
            );
        }
    }

    #[test]
    fn only_a_failure_that_says_so_is_a_session_killed_as_hung() {
        let hung = Halt::Failed(format!(
            "claude printed nothing for 1200s and {KILLED_AS_HUNG}"
        ));
        assert!(hung.killed_as_hung());
        assert!(!Halt::Failed("claude exited with code 1".into()).killed_as_hung());
        assert!(
            !Halt::Halted(format!("the session {KILLED_AS_HUNG}")).killed_as_hung(),
            "a voluntary stop is never a kill"
        );
        assert!(!Halt::Quota(KILLED_AS_HUNG.into()).killed_as_hung());
    }

    #[test]
    fn exit_codes_match_the_frozen_contract() {
        assert_eq!(Halt::Halted(String::new()).exit_code(), 1);
        assert_eq!(Halt::Unreadable(String::new()).exit_code(), 1);
        assert_eq!(Halt::Failed(String::new()).exit_code(), 2);
        assert_eq!(Halt::Quota(String::new()).exit_code(), 3);
    }

    #[test]
    fn but_keeps_the_original_reason_behind_the_new_one() {
        let halt = Halt::Failed("token expired".into()).but("refresh failed");
        assert_eq!(halt.reason(), "refresh failed (token expired)");
    }

    #[test]
    fn halted_and_unreadable_are_both_info_but_stay_distinct_variants() {
        assert_eq!(Halt::Halted(String::new()).severity(), Severity::Info);
        assert_eq!(Halt::Unreadable(String::new()).severity(), Severity::Info);
        assert!(matches!(Halt::Halted(String::new()), Halt::Halted(_)));
        assert!(matches!(
            Halt::Unreadable(String::new()),
            Halt::Unreadable(_)
        ));
    }
}
