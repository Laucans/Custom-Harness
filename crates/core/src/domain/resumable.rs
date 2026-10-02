//! What round state must know so a stage is never re-billed twice.

/// Carried by workflow state: at most once per stage, across all runs.
///
/// Replaces the `done: list[str]` passed to each Python-side call, and the
/// special `mark=False` case of the archive stage.
pub trait Resumable {
    /// The stages already done for the current task.
    fn done(&self) -> &[String];

    /// Mark a stage done. Idempotent: marking it twice does not duplicate
    /// anything in `done()`.
    fn mark(&mut self, stage: &str);

    /// True if this stage has already run.
    fn is_done(&self, stage: &str) -> bool {
        self.done().iter().any(|s| s == stage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct State {
        done: Vec<String>,
    }

    impl Resumable for State {
        fn done(&self) -> &[String] {
            &self.done
        }

        fn mark(&mut self, stage: &str) {
            if !self.is_done(stage) {
                self.done.push(stage.to_string());
            }
        }
    }

    #[test]
    fn marking_twice_does_not_duplicate() {
        let mut state = State::default();
        state.mark("code");
        state.mark("code");
        assert_eq!(state.done(), &["code".to_string()]);
    }

    #[test]
    fn is_done_reflects_what_was_marked() {
        let mut state = State::default();
        assert!(!state.is_done("code"));
        state.mark("code");
        assert!(state.is_done("code"));
    }
}
