//! The circuit breaker: when an identical session has already failed, stop
//! paying for it.
//!
//! The failure this guards against is one a human watched happen: a stage
//! stopped itself on an ambiguity nobody resolved, the polling loop came back
//! a minute later, and the same prompt was paid for five times in a row to
//! receive the same refusal. The budget went; the answer never changed.
//!
//! # What makes two sessions the same
//!
//! The task, the stage, and **the prompt** — fingerprinted, because the
//! prompt is what the session actually reads. This is what makes the breaker
//! reset itself without any human gesture on the ledger: a prompt is built
//! from the issue body, so resolving the ambiguity in the issue changes the
//! fingerprint, and the next session is not the one that failed.
//!
//! # What counts as an error
//!
//! [`STOP`] and [`FAILED`] — the session ran, was billed, and rendered
//! nothing usable. [`QUOTA`] does not: the window is exhausted, no session
//! ran, nothing was billed, and coming back later is the correct behaviour.
//! Counting it would freeze the pipeline for the length of a reset over an
//! error that costs nothing.
//!
//! Only the **trailing** run of errors counts. An `ok` on the same identity
//! clears the slate: whatever went wrong then is not what is happening now.

/// A voluntary stop, as the ledger's `outcome` column writes it.
pub const STOP: &str = "STOP";
/// A stage that rendered nothing usable, as the `outcome` column writes it.
pub const FAILED: &str = "FAILED";
/// An exhausted subscription window, as the `outcome` column writes it.
pub const QUOTA: &str = "QUOTA";

/// How many identical failures are enough to stop paying.
///
/// Two, not one: a single failure can be a flake. A second identical one is
/// not — a stage that stops on an ambiguity in its SPEC says the same thing
/// every time, because the SPEC has not moved. Measured on one task that
/// repeated the same refusal: at three, the refusals cost 1.13 $ before the
/// breaker spoke.
pub const LIMIT: u32 = 2;

/// The 64-bit FNV-1a of `prompt`, in hexadecimal.
///
/// Hand-written rather than taken from `DefaultHasher`: that one is explicitly
/// allowed to change between releases, and this value is compared against
/// values written to disk by earlier runs. A fingerprint that drifts reads as
/// "a different prompt" and quietly disables the breaker.
///
/// Not a security primitive, and no secret goes in: it answers "is this the
/// same text as last time", where a collision costs one refused session.
#[must_use]
pub fn fingerprint(prompt: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in prompt.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

/// Whether this `outcome` word is a session that ran and failed.
#[must_use]
pub fn is_error(outcome: &str) -> bool {
    outcome == STOP || outcome == FAILED
}

/// How many errors end `outcomes`, which are in the order they happened.
///
/// Counts backwards and stops at the first success: that one says the
/// identity is not hopeless, whatever came before it.
///
/// [`QUOTA`] is **transparent** — neither counted nor clearing. It says
/// nothing about the prompt, and letting it clear the slate is how a breaker
/// never trips: an exhausted window between two refusals is the most ordinary
/// thing in a loop that polls.
#[must_use]
pub fn trailing_failures<S: AsRef<str>>(outcomes: &[S]) -> u32 {
    let mut failures: u32 = 0;
    for outcome in outcomes.iter().rev() {
        let word = outcome.as_ref();
        if is_error(word) {
            failures = failures.saturating_add(1);
        } else if word != QUOTA {
            break;
        }
    }
    failures
}

/// Whether that many identical failures is enough to refuse the next session.
#[must_use]
pub const fn tripped(failures: u32) -> bool {
    failures >= LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Halt;

    #[test]
    fn the_counted_words_are_the_ones_halt_actually_writes() {
        // Two halves of the same contract: the ledger writes `Halt::prefix`,
        // and this module reads that column back. A renamed prefix on one
        // side alone would silently stop counting.
        assert!(is_error(Halt::Halted(String::new()).prefix()));
        assert!(is_error(Halt::Unreadable(String::new()).prefix()));
        assert!(is_error(Halt::Failed(String::new()).prefix()));
    }

    #[test]
    fn an_exhausted_window_is_not_a_failed_session() {
        // Nothing ran and nothing was billed: refusing to come back would
        // freeze the pipeline until the reset, over a free error.
        assert!(!is_error(Halt::Quota(String::new()).prefix()));
        assert!(!is_error(QUOTA));
        assert!(!is_error("ok"));
    }

    #[test]
    fn the_same_text_fingerprints_the_same_and_a_changed_one_does_not() {
        assert_eq!(
            fingerprint("the same prompt"),
            fingerprint("the same prompt")
        );
        assert_ne!(
            fingerprint("the same prompt"),
            fingerprint("the same prompt.")
        );
        assert_eq!(fingerprint("").len(), 16, "a fixed-width hex column");
    }

    #[test]
    fn a_fingerprint_is_never_empty_so_it_cannot_match_a_row_written_before_it() {
        // Rows from before the column existed read back as "", and an empty
        // cell must never look like a match.
        assert!(!fingerprint("").is_empty());
    }

    #[test]
    fn identical_failures_in_a_row_trip_the_breaker() {
        let history = [STOP, FAILED, STOP];
        assert_eq!(trailing_failures(&history), 3);
        assert!(tripped(trailing_failures(&history)));
    }

    #[test]
    fn one_is_not_enough_because_a_single_failure_can_be_a_flake() {
        assert!(!tripped(trailing_failures(&[STOP])));
        assert!(tripped(trailing_failures(&[STOP, STOP])));
    }

    #[test]
    fn a_success_clears_what_came_before_it() {
        // The real case: the issue was fixed, the session succeeded, and it
        // broke again for another reason. The old failures are not this one.
        assert_eq!(trailing_failures(&[STOP, STOP, "ok", STOP]), 1);
    }

    #[test]
    fn an_exhausted_window_between_two_refusals_hides_neither() {
        // The real case: the loop polls, the window runs out mid-sequence.
        // Letting QUOTA clear the slate is how a breaker never trips.
        assert_eq!(trailing_failures(&[STOP, STOP, QUOTA, STOP]), 3);
        assert!(tripped(trailing_failures(&[
            STOP, QUOTA, STOP, QUOTA, STOP
        ])));
    }

    #[test]
    fn no_history_at_all_is_no_failure() {
        let empty: [&str; 0] = [];
        assert_eq!(trailing_failures(&empty), 0);
        assert!(!tripped(0));
    }
}
