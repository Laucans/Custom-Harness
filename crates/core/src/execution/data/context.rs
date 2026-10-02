//! What execution carries during a run: the settings, workflow-specific state,
//! the log, what each paid step answered.

use std::collections::HashMap;

use crate::adapters::agent::Reply;
use crate::traces::Logbook;

/// What execution holds during a run, generic over `S` — the workflow-specific
/// state that uses it.
///
/// `harness-core` remains ignorant of workflows: it never names a concrete `S`,
/// only the parameter. This is what replaces `RoundCtx(Ctx)` and `RoundState`
/// on the Python side, without downcast — verified at compile time.
pub struct Context<S> {
    /// The run settings.
    pub settings: Settings,
    /// What belongs to the workflow — declared by it, not by `core`.
    pub state: S,
    /// The log of this run.
    pub traces: Logbook,
    /// What each paid step answered, by its name.
    ///
    /// Here and not in `state`: a response feeds the next step of the same
    /// pass, it doesn't re-read from a resume — it re-pays. The dev loop reads
    /// none of them; PR review reads the inline pass to write its notes,
    /// refinement reads the router and sections for the coherence pass.
    pub results: HashMap<String, Reply>,
}

impl<S> Context<S> {
    /// A fresh context, for this state and these settings.
    #[must_use]
    pub fn new(settings: Settings, state: S, traces: Logbook) -> Self {
        Self {
            settings,
            state,
            traces,
            results: HashMap::new(),
        }
    }
}

/// The parameters of a run.
///
/// Immutable: on the Python side, `provisioning.mount()` rewrote `cfg.workspace`
/// in place, with the comment that « a second path would mean two places to
/// keep in sync ». Here, mounting the workspace makes `Settings` final
/// *before* the `Context` exists — the hack is no longer needed.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Executes nothing, spends nothing — describes what would have run.
    pub dry_run: bool,
    /// `--stages` filter: empty means "all".
    pub stages: String,
}

impl Settings {
    /// True if `stage` should run according to the `--stages` filter.
    ///
    /// Empty means "all" — it is the default, and it lets no workflow
    /// override this rule.
    #[must_use]
    pub fn runs(&self, stage: &str) -> bool {
        self.stages.is_empty() || self.stages.split_whitespace().any(|s| s == stage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(stages: &str) -> Settings {
        Settings {
            dry_run: false,
            stages: stages.to_string(),
        }
    }

    #[test]
    fn empty_filter_runs_everything() {
        assert!(settings("").runs("code"));
    }

    #[test]
    fn filter_runs_only_the_named_stages() {
        let cfg = settings("business-analyst code");
        assert!(cfg.runs("code"));
        assert!(!cfg.runs("create-test"));
    }
}
