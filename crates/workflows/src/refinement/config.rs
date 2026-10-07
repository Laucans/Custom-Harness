//! What this run changes for a refinement round, by value.
//!
//! Separated from the core's `Settings` on purpose: `Settings` carries what
//! **every** workflow has — `--dry-run`, `--stages` — and the core reads it
//! from the `Context`. What is here belongs only to refinement.
//!
//! Also separated from [`crate::refinement::ports::Ports`]: this is only
//! values, never an injected capability.

use std::path::PathBuf;

use harness_core::ports::agent::SessionSpec;

/// The settings for a refinement round.
///
/// One `SessionSpec` per paid step, not a global override like the loop:
/// each section has its own cost/quality tradeoff, and it's the launcher that
/// decides.
pub struct Config {
    /// This issue's artifacts folder — kept body, lock.
    pub refinement_dir: PathBuf,
    /// What a human asked for this round, verbatim.
    pub context: String,
    /// `--explore`: no map, each step re-reads the repository.
    pub explore: bool,
    /// Model and effort for the "Business Goal" section.
    pub goal: SessionSpec,
    /// Model and effort for the "Technical" section.
    pub technical: SessionSpec,
    /// Model and effort for the "Acceptance Criteria" section.
    pub criteria: SessionSpec,
    /// Model and effort for the "Business Rules" section.
    pub rules: SessionSpec,
    /// Model and effort for the "Technical Implementation Plan" section.
    pub plan: SessionSpec,
    /// Model and effort for the router.
    pub router: SessionSpec,
    /// Model and effort for the coherence pass.
    pub coherence: SessionSpec,
    /// Model and effort for the advice on the technical refinement.
    pub advice: SessionSpec,
    /// This issue's artifacts folder — same files the repository map drops
    /// there, resolved to the current round's name.
    pub artifacts_dir: PathBuf,
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config: all steps at `sonnet`/`high`.

    use std::path::PathBuf;

    use harness_core::ports::agent::SessionSpec;

    use super::Config;

    fn spec() -> SessionSpec {
        SessionSpec {
            model: "sonnet".to_string(),
            effort: "high".to_string(),
        }
    }

    /// A test config, artifacts under this folder.
    pub fn in_dir(refinement_dir: PathBuf, context: &str) -> Config {
        Config {
            artifacts_dir: refinement_dir.join("25"),
            refinement_dir,
            context: context.to_string(),
            explore: false,
            goal: spec(),
            technical: spec(),
            criteria: spec(),
            rules: spec(),
            plan: spec(),
            router: spec(),
            coherence: spec(),
            advice: spec(),
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/refinement"), "")
    }
}
