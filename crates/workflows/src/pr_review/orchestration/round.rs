//! The review's round: the sequence, and the one failure it lives with.
//!
//! A single round — [`ReviewRun`](crate::pr_review::orchestration::workflow::ReviewRun)
//! carries `remaining = 1` — so the generic [`Round`] of the core serves it as
//! is: the review doesn't branch, and nothing about it varies from one turn to
//! the next.

use harness_core::domain::Halt;
use harness_core::execution::{Round, Tolerance};

use crate::pr_review::config::Config;
use crate::pr_review::data::state::ReviewState;
use crate::pr_review::orchestration::stages;
use crate::pr_review::ports::Ports;

/// The inline pass may render nothing without the notes losing their value.
///
/// An exhausted quota is the exception: pass 2 would spend the same window
/// and come back the same.
pub struct InlineMayFail {
    /// The stage this forgives. Received as a field, per rule 4 of
    /// `ARCHITECTURE.md` — a second literal here would drift from the table.
    pub stage: String,
}

impl Tolerance for InlineMayFail {
    fn tolerate(&self, stage: &str, failed: &Halt) -> Option<String> {
        if stage != self.stage || matches!(failed, Halt::Quota(_)) {
            return None;
        }
        Some("inline pass produced no review — continuing without it".to_string())
    }
}

/// Mounts the review's round: the two paid passes, then the publication.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> Round<ReviewState> {
    Round {
        stages: stages::table(ports, config),
        post: None,
        tolerance: Some(Box::new(InlineMayFail {
            stage: stages::INLINE.to_string(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr_review::config::fake as config_fake;
    use crate::pr_review::ports::fake as ports_fake;

    #[test]
    fn the_round_holds_the_two_passes_and_the_publication() {
        let round = build(&ports_fake::ports(), &config_fake::config());
        assert_eq!(round.stages.len(), 3);
        assert!(round.tolerance.is_some());
    }

    #[test]
    fn only_the_inline_pass_is_forgiven_and_never_on_a_quota() {
        // What this guards: the summary pass spending the same exhausted
        // window to come back with the same answer.
        let rule = InlineMayFail {
            stage: stages::INLINE.to_string(),
        };
        assert!(
            rule.tolerate(stages::INLINE, &Halt::Failed("nothing".into()))
                .is_some()
        );
        assert!(
            rule.tolerate(stages::INLINE, &Halt::Quota("window".into()))
                .is_none()
        );
        assert!(
            rule.tolerate(stages::BRIEF, &Halt::Failed("nothing".into()))
                .is_none()
        );
    }
}
