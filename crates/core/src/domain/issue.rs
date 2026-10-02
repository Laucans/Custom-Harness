//! What an issue is, and nothing of what a workflow makes of it.
//!
//! Work tracking lives in GitHub issues. This module carries the **form** —
//! a number, a title, a state, labels, a body, blockers — and the only
//! questions you can ask it without knowing what it is for.
//!
//! **The vocabulary, not the definition.** What makes an issue a "task ready
//! to run" — which labels matter, which to pick next — is the definition of
//! a workflow and lives with it. This separation is what allows
//! `adapters::shell::github` to yield `Issue`s: an adapter deserializes, it
//! does not decide what a label *means*.

/// A GitHub issue, as the harness sees it.
///
/// The same type serves for a roadmap item, a milestone, a task, a human
/// action, and a PR: what distinguishes them is a label, not a class. This is
/// what allows `blocked_by` to carry issues of any kind — a dependency does
/// not ask what it blocks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Issue {
    /// Its number.
    pub number: u64,
    /// Its title.
    pub title: String,
    /// `open` or `closed`, as the API says.
    pub state: String,
    /// Its labels, by name.
    pub labels: Vec<String>,
    /// Its body. For a task, the body **is** the SPEC.
    pub body: String,
    /// Its blockers, **with their state**: knowing an issue is blocked is not
    /// enough, you must know if the blocker is still open.
    pub blocked_by: Vec<Self>,
}

impl Issue {
    /// The issue's identity for resume state: its number.
    ///
    /// A number does not change when you rewrite the title — which was not
    /// true when a task was designated by `<num>|<title>`.
    #[must_use]
    pub fn key(&self) -> String {
        self.number.to_string()
    }

    /// `#12` — as a message names it.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("#{}", self.number)
    }

    /// True as long as GitHub has not closed it.
    ///
    /// Anything that is not `closed` counts as open: a state this version does
    /// not know must not make work disappear.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.state != "closed"
    }

    /// True if GitHub has closed it.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.state == "closed"
    }

    /// True if it bears this label.
    #[must_use]
    pub fn has(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(state: &str, labels: &[&str]) -> Issue {
        Issue {
            number: 12,
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    #[test]
    fn an_unknown_state_counts_as_open_rather_than_losing_the_work() {
        let odd = issue("something_new", &[]);
        assert!(odd.is_open());
        assert!(!odd.is_closed());
    }

    #[test]
    fn closed_is_the_only_state_that_closes() {
        assert!(issue("closed", &[]).is_closed());
        assert!(!issue("closed", &[]).is_open());
    }

    #[test]
    fn labels_are_matched_exactly_not_by_prefix() {
        let task = issue("open", &["pipeline:agent"]);
        assert!(task.has("pipeline:agent"));
        assert!(!task.has("pipeline"));
    }

    #[test]
    fn the_key_is_the_number_so_a_retitled_task_keeps_its_identity() {
        let mut task = issue("open", &[]);
        let before = task.key();
        task.title = "un titre tout neuf".to_string();
        assert_eq!(task.key(), before);
        assert_eq!(task.reference(), "#12");
    }
}
