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
