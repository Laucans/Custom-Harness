//! What the harness cannot tell, observed instead: whether its watch still
//! runs, which issues wait on a human, how much of Claude's windows is left.

use harness_core::domain::Issue;
use harness_core::domain::quota::Reading;
use harness_workflows::common::labels;
use harness_workflows::rules::data::request::CONTESTED;

/// An issue that waits on a human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    /// Its number.
    pub number: u64,
    /// Its title.
    pub title: String,
    /// The label that says why: `harness:human` or `harness:needs-decision`.
    pub label: String,
}

/// A diagnosis the doctor was asked about one run, once it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnosed {
    /// The line the run was on: `split`.
    pub workflow: String,
    /// The run: `20261008-145556`.
    pub run: String,
    /// The doctor answered — otherwise the question went unanswered.
    pub answered: bool,
    /// When it ended, as the traces write a clock.
    pub at: String,
}

/// Everything observed, as of one read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    /// The watch process of the checkout runs — `None` when nobody looked.
    pub watch_running: Option<bool>,
    /// The open issues that wait on a human.
    pub waiting: Vec<Waiting>,
    /// The last reading of Claude's windows.
    pub claude: Option<Reading>,
    /// When that reading was made, as the traces write a clock.
    pub claude_at: String,
    /// Now, in seconds since the epoch — what says a window has reset.
    pub now: u64,
    /// The diagnoses the doctor was asked for that have ended.
    pub diagnoses: Vec<Diagnosed>,
    /// The open rules' tasks that settle a contested test: the AI decides,
    /// the human is warned and may act.
    pub contested: Vec<Waiting>,
}

/// The open issues among `issues` that carry `harness:human` or
/// `harness:needs-decision`, in number order, each once.
#[must_use]
pub fn waiting_on_a_human<'a>(issues: impl IntoIterator<Item = &'a Issue>) -> Vec<Waiting> {
    let mut waiting: Vec<Waiting> = issues
        .into_iter()
        .filter(|issue| issue.is_open())
        .filter_map(|issue| {
            [labels::HUMAN, labels::NEEDS_DECISION]
                .into_iter()
                .find(|label| issue.has(label))
                .map(|label| Waiting {
                    number: issue.number,
                    title: issue.title.clone(),
                    label: label.to_string(),
                })
        })
        .collect();
    waiting.sort_by_key(|w| w.number);
    waiting.dedup_by_key(|w| w.number);
    waiting
}

/// The open rules' tasks among `issues` that settle a contested test —
/// `harness:rules`, titled `Rules contested…` — in number order.
#[must_use]
pub fn contested<'a>(issues: impl IntoIterator<Item = &'a Issue>) -> Vec<Waiting> {
    let mut contested: Vec<Waiting> = issues
        .into_iter()
        .filter(|issue| {
            issue.is_open()
                && issue.has(labels::RULES)
                && issue.title.trim_start().starts_with(CONTESTED)
        })
        .map(|issue| Waiting {
            number: issue.number,
            title: issue.title.clone(),
            label: labels::RULES.to_string(),
        })
        .collect();
    contested.sort_by_key(|w| w.number);
    contested.dedup_by_key(|w| w.number);
    contested
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64, state: &str, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("issue {number}"),
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    #[test]
    fn only_open_rules_tasks_titled_as_a_contest_are_contested() {
        let titled = |number, state, labels: &[&str], title: &str| {
            let mut issue = issue(number, state, labels);
            issue.title = title.to_string();
            issue
        };
        let issues = [
            titled(9, "open", &[labels::RULES], "Rules contested: PR #12"),
            titled(
                4,
                "open",
                &[labels::RULES],
                "Rules: write the rules and their tests",
            ),
            titled(5, "closed", &[labels::RULES], "Rules contested: PR #3"),
            titled(6, "open", &[labels::AGENT], "Rules contested: PR #4"),
            titled(
                2,
                "open",
                &[labels::RULES, labels::READY],
                "Rules contested: PR #1",
            ),
        ];
        let found = contested(&issues);
        let numbers: Vec<u64> = found.iter().map(|w| w.number).collect();
        assert_eq!(numbers, [2, 9]);
        assert_eq!(found[1].label, labels::RULES);
        assert_eq!(found[1].title, "Rules contested: PR #12");
    }

    #[test]
    fn only_open_issues_labelled_for_a_human_wait() {
        let issues = [
            issue(31, "open", &[labels::HUMAN]),
            issue(12, "open", &[labels::NEEDS_DECISION, labels::AGENT]),
            issue(13, "closed", &[labels::HUMAN]),
            issue(14, "open", &[labels::AGENT]),
            issue(31, "open", &[labels::HUMAN]),
        ];
        let waiting = waiting_on_a_human(&issues);
        let numbers: Vec<u64> = waiting.iter().map(|w| w.number).collect();
        assert_eq!(numbers, [12, 31]);
        assert_eq!(waiting[0].label, labels::NEEDS_DECISION);
    }
}
