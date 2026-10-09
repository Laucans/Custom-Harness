//! What the harness cannot tell, observed instead: whether its watch still
//! runs, which issues wait on a human, how much of Claude's windows is left.

use harness_core::domain::Issue;
use harness_core::domain::quota::Reading;
use harness_workflows::common::labels;

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

/// Everything observed, as of one read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    /// The watch process of the checkout runs — `None` when nobody looked.
    pub watch_running: Option<bool>,
    /// The open issues that wait on a human.
    pub waiting: Vec<Waiting>,
    /// The last reading of Claude's windows.
    pub claude: Option<Reading>,
    /// Now, in seconds since the epoch — what says a window has reset.
    pub now: u64,
    /// Now, as the traces write a clock.
    pub now_at: String,
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
