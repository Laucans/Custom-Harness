//! What this run changes for the loop, by value.
//!
//! Separated from the core's `Settings` on purpose. `Settings` carries what
//! **every** workflow has — `--dry-run`, `--stages` — and the core reads it
//! at each round, from the `Context`. What is here belongs only to the loop,
//! and is read once, at assembly: its integration branch, the model override.
//!
//! Also separated from [`crate::dev_loop::ports::Ports`]: this is only values
//! (`String`, `bool`), never an `Rc<dyn Trait>`.

/// What this workflow cites as having injected the prompt.
///
/// Here and not in `harness-core::domain::prompts`: this is the entry point of
/// **this** workflow, and a session reading this name should be able to open
/// the file that injected it. The core itself names no instance — its default
/// remains [`prompts::INJECTOR`](harness_core::domain::prompts::INJECTOR).
pub const INJECTOR: &str = "harness-launcher (dev_loop)";

/// The settings for a loop run.
pub struct Config {
    /// The branch on which the loop works and merges.
    pub integration_branch: String,
    /// Model override for the entire run. Empty: each stage keeps its own.
    pub model: String,
    /// Effort override for the entire run. Empty: same.
    pub effort: String,
    /// Replay a stage that resumption would skip.
    pub restart: bool,
    /// The repository's own build and test configuration, verbatim — see
    /// [`crate::dev_loop::data::stack`]. Empty when the stack was not
    /// recognised, and then no block is injected.
    ///
    /// Read once at assembly, like everything else here: it is a property of
    /// the checkout, identical for every stage and every turn of the run.
    pub stack: String,
    /// The public shape of every indexed file — see
    /// [`crate::dev_loop::data::signatures`]. Built once per run; which files a
    /// prompt carries is decided per task.
    pub signatures: std::rc::Rc<crate::dev_loop::data::signatures::Index>,
    /// The checkout's root — what the architecture gates read after `code`.
    pub root: std::path::PathBuf,
}

impl Config {
    /// The model and effort for this stage, with run overrides applied.
    ///
    /// `MODEL`/`EFFORT` are the brute-force override: one value for the entire
    /// run. Applied here and in one place only — on the Python side, `resolve`
    /// rebuilt an entire `StageSpec`, and a field added to the table was lost
    /// as soon as a run provided `--model`.
    #[must_use]
    pub fn spec(&self, model: &str, effort: &str) -> harness_core::ports::agent::SessionSpec {
        harness_core::ports::agent::SessionSpec {
            model: pick(&self.model, model),
            effort: pick(&self.effort, effort),
        }
    }

    /// The name cited as the injector in this workflow's prompts.
    #[must_use]
    pub const fn injector(&self) -> &'static str {
        INJECTOR
    }
}

fn pick(forced: &str, default: &str) -> String {
    if forced.is_empty() {
        default.to_string()
    } else {
        forced.to_string()
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config, no overrides.

    use super::Config;

    /// A test config: branch `main_agent`, no overrides.
    pub fn config() -> Config {
        Config {
            integration_branch: "main_agent".to_string(),
            model: String::new(),
            effort: String::new(),
            restart: false,
            stack: String::new(),
            signatures: std::rc::Rc::default(),
            root: std::path::PathBuf::from("/ws"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::prompts;

    #[test]
    fn the_injector_this_workflow_names_is_not_the_cores_placeholder() {
        // The core remains generic; a session reading "injected by …" should
        // be able to open the file that injected it.
        assert_ne!(INJECTOR, prompts::INJECTOR);
    }

    #[test]
    fn without_a_forced_model_each_stage_keeps_its_own() {
        let spec = fake::config().spec("sonnet", "high");
        assert_eq!(spec.model, "sonnet");
        assert_eq!(spec.effort, "high");
    }

    #[test]
    fn a_forced_model_overrides_every_stage_of_the_run() {
        let mut forced = fake::config();
        forced.model = "opus".to_string();
        let spec = forced.spec("sonnet", "high");
        assert_eq!(spec.model, "opus");
        // Forcing one does not force the other: these are two separate variables.
        assert_eq!(spec.effort, "high");
    }
}
