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
    /// What tasks of each unit have cost, one line each, for the slice
    /// prompt — so the cut sees what it costs. Empty: nothing measured yet.
    pub cost_note: String,
}

/// What one task may cost before the split cuts finer, in dollars: a profile
/// whose tasks' third quartile runs past this is one notch too coarse for
/// the model that builds them.
pub const PER_TASK_CEILING: f64 = 12.0;

/// The profile one notch finer: `strong` → `mid` → `small` → `small`.
#[must_use]
pub fn finer(profile: &str) -> String {
    match profile {
        "strong" => "mid".to_string(),
        _ => "small".to_string(),
    }
}

/// The profile adjusted by what its tasks have cost.
///
/// One notch finer when at least [`harness_core::domain::budget::ENOUGH`]
/// tasks were measured and their third quartile runs past
/// [`PER_TASK_CEILING`]. An explicit profile is never adjusted — the human
/// said so.
#[must_use]
pub fn adjusted(
    profile: &str,
    explicit: bool,
    typical: Option<&harness_core::domain::budget::Typical>,
) -> String {
    match typical {
        Some(found)
            if !explicit
                && found.count >= harness_core::domain::budget::ENOUGH
                && found.p75 > PER_TASK_CEILING =>
        {
            finer(profile)
        }
        _ => profile.to_string(),
    }
}

/// The cost note from what the history measured of every unit.
///
/// # Errors
///
/// A read failure of the history: an unreadable ledger is not an empty one.
pub fn cost_note_for(
    history: &dyn harness_core::ports::store::costs::CostHistory,
) -> harness_core::domain::Outcome<String> {
    let mut measured = Vec::new();
    for unit in harness_architecture::vocabulary::Unit::ALL {
        if let Some(typical) = history.typical(unit.key(), "")? {
            measured.push((unit.key(), typical));
        }
    }
    Ok(cost_note(&measured))
}

/// The cost note of the inventory: one line per unit that was measured.
#[must_use]
pub fn cost_note(measured: &[(&str, harness_core::domain::budget::Typical)]) -> String {
    let lines: Vec<String> = measured
        .iter()
        .filter(|(_, typical)| typical.count > 0)
        .map(|(unit, typical)| {
            format!(
                "- {unit}: median {:.2} $, third quartile {:.2} $ over {} task(s)",
                typical.median, typical.p75, typical.count
            )
        })
        .collect();
    lines.join("\n")
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
    use super::{PER_TASK_CEILING, adjusted, cost_note, finer, profile_for};
    use harness_core::domain::budget::Typical;

    fn measured(count: usize, p75: f64) -> Typical {
        Typical {
            count,
            median: p75 / 2.0,
            p75,
        }
    }

    #[test]
    fn a_profile_whose_tasks_cost_too_much_is_cut_one_notch_finer() {
        let dear = measured(3, PER_TASK_CEILING + 1.0);
        assert_eq!(adjusted("strong", false, Some(&dear)), "mid");
        assert_eq!(adjusted("mid", false, Some(&dear)), "small");
        assert_eq!(
            adjusted("small", false, Some(&dear)),
            "small",
            "nothing finer"
        );
        assert_eq!(finer("strong"), "mid");
    }

    #[test]
    fn a_profile_is_kept_when_its_tasks_are_cheap_few_or_the_human_named_it() {
        let cheap = measured(5, PER_TASK_CEILING - 1.0);
        assert_eq!(adjusted("mid", false, Some(&cheap)), "mid");
        let few = measured(2, PER_TASK_CEILING * 3.0);
        assert_eq!(
            adjusted("mid", false, Some(&few)),
            "mid",
            "two tasks prove nothing"
        );
        let dear = measured(9, PER_TASK_CEILING * 3.0);
        assert_eq!(
            adjusted("strong", true, Some(&dear)),
            "strong",
            "explicit wins"
        );
        assert_eq!(adjusted("mid", false, None), "mid");
    }

    #[test]
    fn the_note_is_read_from_the_history_unit_by_unit() {
        struct Only(&'static str);
        impl harness_core::ports::store::costs::CostHistory for Only {
            fn typical(
                &self,
                unit: &str,
                _profile: &str,
            ) -> harness_core::domain::Outcome<Option<Typical>> {
                Ok((unit == self.0).then(|| measured(3, 2.0)))
            }
        }
        let note = super::cost_note_for(&Only("aggregate")).expect("read");
        assert!(note.starts_with("- aggregate:"), "{note}");
        assert_eq!(note.lines().count(), 1);
    }

    #[test]
    fn the_cost_note_lists_the_measured_units_only() {
        let note = cost_note(&[
            ("capability", measured(4, 3.5)),
            ("aggregate", measured(0, 0.0)),
        ]);
        assert_eq!(
            note,
            "- capability: median 1.75 $, third quartile 3.50 $ over 4 task(s)"
        );
        assert_eq!(cost_note(&[]), "");
    }

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
            cost_note: String::new(),
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/split"))
    }
}
