//! Each rule's state, as the factory last judged it: what the rules' gate
//! wrote (`rules-status.json`, kept by the CI's `rules-hold` job on the
//! integration branch) and the contests the board holds open.
//!
//! Pure: the artifact and the board are read elsewhere and handed in.

use std::collections::BTreeMap;

use harness_core::domain::Issue;
use harness_core::ports::shell::github::Kept;
use harness_workflows::common::labels;
use harness_workflows::rules::data::request::CONTESTED;
use serde::Serialize;
use serde_json::Value;

/// One rule's state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RuleState {
    /// `green`, `red`, `pending` (a unit it needs is not built) or
    /// `untested`, as the CI said; `unknown` when it said nothing of it.
    pub state: String,
    /// Changed, its tests ahead of the code: red is expected until its
    /// conformity task lands.
    pub conform: bool,
    /// The open issue contesting one of its tests, if any.
    pub contested: Option<u64>,
    /// Why, in the tests' words: what it waits for, what does not hold.
    pub why: String,
}

/// The rules' states, and where they come from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The run that judged them; empty when no run did.
    pub run_url: String,
    /// When it last moved.
    pub at: String,
    /// Each rule by its id (`R3`).
    pub rules: BTreeMap<String, RuleState>,
    /// The open rules' tasks, by number: development waits on them.
    pub tasks: Vec<u64>,
}

/// The rule ids a text names: `R` and a number, standing alone.
fn ids_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes = text.as_bytes();
    for (at, _) in text.match_indices('R') {
        let before = at.checked_sub(1).and_then(|b| bytes.get(b));
        if before.is_some_and(u8::is_ascii_alphanumeric) {
            continue;
        }
        let digits: String = text
            .get(at + 1..)
            .unwrap_or_default()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let after = bytes.get(at + 1 + digits.len());
        if digits.is_empty()
            || digits.starts_with('0')
            || after.is_some_and(u8::is_ascii_alphanumeric)
        {
            continue;
        }
        let id = format!("R{digits}");
        if !found.contains(&id) {
            found.push(id);
        }
    }
    found
}

/// What the gate wrote and the board holds, as each rule's state.
#[must_use]
pub fn status(kept: Option<&Kept>, rules_issues: &[Issue]) -> Status {
    let mut status = Status::default();
    if let Some(kept) = kept {
        status.run_url.clone_from(&kept.run_url);
        status.at.clone_from(&kept.at);
        let read: Value = serde_json::from_str(&kept.text).unwrap_or(Value::Null);
        for rule in read
            .get("rules")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(id) = rule.get("id").and_then(Value::as_str) else {
                continue;
            };
            let why = rule
                .get("tests")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|t| t.get("why").and_then(Value::as_str))
                .find(|why| !why.is_empty())
                .unwrap_or_default();
            status.rules.insert(
                id.to_string(),
                RuleState {
                    state: rule
                        .get("state")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    conform: rule.get("conform").and_then(Value::as_bool) == Some(true),
                    contested: None,
                    why: why.to_string(),
                },
            );
        }
    }
    for issue in rules_issues
        .iter()
        .filter(|i| i.is_open() && i.has(labels::RULES))
    {
        status.tasks.push(issue.number);
        if !issue.title.trim_start().starts_with(CONTESTED) {
            continue;
        }
        for id in ids_in(&issue.body) {
            let rule = status.rules.entry(id).or_insert_with(|| RuleState {
                state: "unknown".to_string(),
                ..RuleState::default()
            });
            rule.contested.get_or_insert(issue.number);
        }
    }
    status.tasks.sort_unstable();
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn issue(number: u64, state: &str, title: &str, body: &str) -> Issue {
        Issue {
            number,
            title: title.to_string(),
            body: body.to_string(),
            state: state.to_string(),
            labels: vec![labels::RULES.to_string()],
            ..Issue::default()
        }
    }

    #[test]
    fn the_gate_s_states_and_the_open_contests_make_each_rule_s_state() {
        let kept = Kept {
            text: r#"{"at": "x", "rules": [
                {"id": "R1", "state": "green", "conform": false, "tests": []},
                {"id": "R2", "state": "red", "conform": true, "tests": [
                    {"test": "a", "state": "red", "why": ""},
                    {"test": "b", "state": "red", "why": "still listed"}
                ]},
                {"id": "R3", "state": "pending", "tests": [{"why": "waiting for capability:List"}]},
                {"state": "green"}
            ]}"#
            .to_string(),
            run_url: "https://github.com/o/r/actions/runs/7".to_string(),
            at: "2026-10-14T10:00:00Z".to_string(),
        };
        let issues = [
            issue(
                9,
                "open",
                "Rules contested: PR #12",
                "R3 cannot hold; see R30, not CR3",
            ),
            issue(5, "open", "Rules changed: R2", "R1"),
            issue(4, "closed", "Rules contested: PR #2", "R1"),
        ];
        let read = status(Some(&kept), &issues);
        assert_eq!(read.run_url, kept.run_url);
        assert_eq!(read.tasks, [5, 9]);
        assert_eq!(read.rules["R1"].state, "green");
        assert_eq!(
            read.rules["R1"].contested, None,
            "a closed contest is settled"
        );
        assert!(read.rules["R2"].conform);
        assert_eq!(read.rules["R2"].why, "still listed");
        assert_eq!(read.rules["R3"].contested, Some(9));
        assert_eq!(read.rules["R3"].why, "waiting for capability:List");
        assert_eq!(read.rules["R30"].state, "unknown");
        assert_eq!(read.rules["R30"].contested, Some(9));
        assert_eq!(read.rules.len(), 4, "CR3 names no rule");
    }

    #[test]
    fn without_a_run_only_the_board_speaks() {
        let read = status(None, &[]);
        assert!(read.run_url.is_empty() && read.rules.is_empty() && read.tasks.is_empty());
        let garbled = Kept {
            text: "not json".to_string(),
            ..Kept {
                text: String::new(),
                run_url: "u".to_string(),
                at: String::new(),
            }
        };
        assert!(status(Some(&garbled), &[]).rules.is_empty());
    }

    proptest! {
        #[test]
        fn a_rule_id_is_found_wherever_it_stands_alone(
            n in 1u32..100_000,
            before in "[ .,;(]{0,3}",
            after in "[ .,;)]{0,3}",
        ) {
            let text = format!("x{before}R{n}{after}y");
            let alone = !before.is_empty() && !after.is_empty();
            prop_assert_eq!(ids_in(&text), if alone { vec![format!("R{n}")] } else { vec![] });
        }

        #[test]
        fn reading_any_text_never_panics(text in ".{0,200}") {
            let _ = ids_in(&text);
        }
    }
}
