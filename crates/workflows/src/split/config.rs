//! What this run changes for a split round, by value.
//!
//! Separated from the core's `Settings` on purpose: `Settings` carries what
//! **every** workflow has — `--dry-run`, `--stages` — and the core reads it
//! from the `Context`. What is here belongs only to split.
//!
//! Also separated from [`crate::split::ports::Ports`]: this is only values,
//! never an injected capability.

use std::path::PathBuf;

use harness_core::ports::agent::SessionSpec;

/// The settings for a split round.
pub struct Config {
    /// This milestone's lock directory — one split run at a time.
    pub split_dir: PathBuf,
    /// The branch the milestone's own branch is created from, if it does
    /// not exist yet — `main_agent`.
    pub base_branch: String,
    /// Model and effort for the slice step.
    pub slice: SessionSpec,
    /// What the slices are cut for: `strong`, `mid` or `small` — how much
    /// one session of the model that will build them is trusted to hold.
    /// Deduced from the loop's model when the run names none.
    pub profile: String,
    /// The checkout's root — where the architecture's manifests are read.
    pub root: PathBuf,
    /// The human's prompt edits, read once per run — a stage sends
    /// `prompts.text(KEY, DEFAULT)` (see [`crate::prompts`]).
    pub prompts: std::rc::Rc<harness_core::domain::prompt_book::PromptBook>,
}

/// The profile a model is cut for: how much one session of it holds.
///
/// `opus` and the like are `strong`; `sonnet` is `mid`; anything smaller —
/// `haiku`, a local model — is `small`. An explicit profile wins.
#[must_use]
pub fn profile_for(explicit: &str, model: &str) -> String {
    let explicit = explicit.trim().to_lowercase();
    if ["strong", "mid", "small"].contains(&explicit.as_str()) {
        return explicit;
    }
    let model = model.to_lowercase();
    if model.contains("opus") || model.contains("fable") {
        "strong".to_string()
    } else if model.contains("sonnet") || model.is_empty() {
        "mid".to_string()
    } else {
        "small".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::profile_for;

    #[test]
    fn the_profile_follows_the_model_unless_named() {
        assert_eq!(profile_for("", "opus"), "strong");
        assert_eq!(profile_for("", "claude-sonnet-5-5"), "mid");
        assert_eq!(profile_for("", ""), "mid");
        assert_eq!(profile_for("", "haiku"), "small");
        assert_eq!(profile_for("", "qwen2.5-coder"), "small");
        assert_eq!(
            profile_for("Small", "opus"),
            "small",
            "an explicit profile wins"
        );
        assert_eq!(
            profile_for("huge", "opus"),
            "strong",
            "an unknown profile falls back on the model"
        );
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config, disposable folder.

    use std::path::PathBuf;

    use harness_core::ports::agent::SessionSpec;

    use super::Config;

    /// A test config, under this folder.
    pub fn in_dir(split_dir: PathBuf) -> Config {
        Config {
            prompts: std::rc::Rc::default(),
            split_dir,
            base_branch: "main_agent".to_string(),
            profile: "mid".to_string(),
            slice: SessionSpec {
                model: "sonnet".to_string(),
                effort: "high".to_string(),
            },
            root: PathBuf::from("/tmp/split-root"),
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/split"))
    }
}
