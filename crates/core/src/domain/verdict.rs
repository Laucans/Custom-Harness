//! What an executable returns: not its data, the control of what follows.

use crate::domain::halt::Halt;

/// The result of an `execute()` that didn't stop the sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The work was done, or the sequence can proceed.
    Continue,
    /// Nothing to do here; the reason is logged by the caller.
    Skip(String),
    /// There's nothing left to do **at all** — a success, not a stop.
    ///
    /// Distinct from [`Halt`]: "nothing left to play" and "something broke"
    /// must never share an exit code. Only a `Round` emits it; the repetition
    /// stops on it and returns success.
    NothingLeft(String),
}

/// What every executable returns: the control, never the payload.
///
/// The data produced goes in the `Context`; `Outcome` carries only what's
/// needed to decide if the sequence continues, skips, stops cleanly
/// (`NothingLeft` via `Verdict`), or failed (`Err`).
pub type Outcome<T> = Result<T, Halt>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_and_nothing_left_are_not_the_same_verdict() {
        assert_eq!(Verdict::Skip("a".into()), Verdict::Skip("a".into()));
        assert_ne!(Verdict::Skip("a".into()), Verdict::NothingLeft("a".into()));
    }
}
