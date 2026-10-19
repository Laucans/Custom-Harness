//! What is asked, and what the session decided — read and checked here,
//! with no I/O.

use serde::Deserialize;

use crate::common::decision::Question;
use crate::common::json_reply::{self, ParseError};

/// What a decision may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The decision is written; the task goes on from it.
    Answer,
    /// The task's PR is merged as it is.
    Merge,
    /// The task's PR is repaired once more, its repairs counted anew.
    Repair,
    /// The task is closed, with why.
    Cut,
    /// The task waits for another issue — a milestone or a task — to land,
    /// through a GitHub blocker: nothing is paid until it does.
    Wait {
        /// The issue waited for.
        on: u64,
    },
}

impl Action {
    /// How the decision names it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Answer => "answer",
            Self::Merge => "merge",
            Self::Repair => "repair",
            Self::Cut => "cut",
            Self::Wait { .. } => "wait",
        }
    }
}

/// What the session answers.
#[derive(Debug, Clone, Deserialize)]
struct Answered {
    action: String,
    summary: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    confidence: u8,
    /// For a `wait`: the issue waited for, as a number or as `#20`.
    #[serde(default)]
    on: Option<serde_json::Value>,
}

/// An issue number as a session writes it: `20`, `"20"` or `"#20"`.
fn issue_number(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.trim().trim_start_matches('#').parse().ok(),
        _ => None,
    }
}

/// A decision, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decided {
    /// What to do.
    pub action: Action,
    /// The decision, in a sentence.
    pub summary: String,
    /// What the next session reads under `## Decision`: the answers, the
    /// choices made, markdown.
    pub answer: String,
    /// How sure it was, 1 to 5.
    pub confidence: u8,
}

/// The decision a session answered, for issue `number`, with or without a
/// PR.
///
/// # Errors
///
/// No JSON object, an action it does not have, a `merge` or a `repair`
/// without a PR, a `wait` naming no issue or the issue itself, an empty
/// summary.
pub fn parse(text: &str, has_pr: bool, number: u64) -> Result<Decided, String> {
    let answered: Answered =
        json_reply::parse_object(text).map_err(|e: ParseError| e.to_string())?;
    let action = match answered.action.trim().to_lowercase().as_str() {
        "answer" | "proceed" => Action::Answer,
        "merge" => Action::Merge,
        "repair" => Action::Repair,
        "cut" => Action::Cut,
        "wait" => {
            let on = answered.on.as_ref().and_then(issue_number).ok_or_else(|| {
                "`wait` names no issue: give `on`, the number of the issue waited for".to_string()
            })?;
            if on == number {
                return Err(format!(
                    "`wait` on #{number} itself: nothing would ever land"
                ));
            }
            Action::Wait { on }
        }
        _ => {
            return Err(format!(
                "`{}` is no action: answer, merge, repair, wait or cut",
                answered.action
            ));
        }
    };
    if !has_pr && matches!(action, Action::Merge | Action::Repair) {
        return Err(format!(
            "`{}` needs a pull request, and this issue has none",
            action.name()
        ));
    }
    if answered.summary.trim().is_empty() {
        return Err("the decision has no summary".to_string());
    }
    Ok(Decided {
        action,
        summary: answered.summary.trim().to_string(),
        answer: answered.answer.trim().to_string(),
        confidence: answered.confidence.clamp(1, 5),
    })
}
/// What kind of decision is asked.
///
/// The `kind:` of `## Decision` when it says, else what the issue shows — a
/// task waiting on its PR asks about its review, a graph question the
/// refinement's, anything else its spec.
#[must_use]
pub fn kind_of(question: Option<&Question>, review_pending: bool, rules: bool) -> String {
    if let Some(kind) = question.and_then(|q| q.kind.clone()) {
        return kind;
    }
    if rules {
        return "rules".to_string();
    }
    if review_pending {
        return "review".to_string();
    }
    if question.is_some() {
        return "graph".to_string();
    }
    "spec".to_string()
}

/// The `## Decision` section the decision leaves: the question as it was
/// asked, decided by the AI with its summary, then the answer the next
/// session reads.
#[must_use]
pub fn section(question: Option<&Question>, kind: &str, decided: &Decided) -> String {
    let mut q = question.cloned().unwrap_or_else(|| Question {
        kind: Some(kind.to_string()),
        blocks: format!("a decision was asked ({kind})"),
        round: 1,
        ..Question::default()
    });
    q.decided_by = Some("ai".to_string());
    q.verdict = Some(format!(
        "{} — {} ({}/5)",
        decided.action.name(),
        decided.summary,
        decided.confidence
    ));
    if decided.answer.is_empty() {
        q.render()
    } else {
        format!("{}\n\n{}", q.render(), decided.answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_decision_reads_back_and_one_without_a_pr_cannot_touch_one() {
        let text = r#"I looked. ```json
{"action": "repair", "summary": " fix the null check ", "answer": "- keep the API", "confidence": 9}
```"#;
        let read = parse(text, true, 15).expect("read");
        assert_eq!(read.action, Action::Repair);
        assert_eq!(read.summary, "fix the null check");
        assert_eq!(read.confidence, 5, "clamped");
        assert!(
            parse(text, false, 15)
                .expect_err("no PR")
                .contains("needs a pull request")
        );
        assert!(parse(r#"{"action":"vote","summary":"x"}"#, true, 15).is_err());
        assert!(parse(r#"{"action":"cut","summary":" "}"#, true, 15).is_err());
        assert_eq!(
            parse(r#"{"action":"proceed","summary":"go"}"#, false, 15)
                .expect("an alias")
                .action,
            Action::Answer
        );
        assert!(parse("no json", true, 15).is_err());
    }

    #[test]
    fn the_kind_is_the_question_s_or_what_the_issue_shows() {
        let stop = Question {
            kind: Some("stop".to_string()),
            blocks: "b".to_string(),
            ..Question::default()
        };
        assert_eq!(kind_of(Some(&stop), true, false), "stop");
        assert_eq!(kind_of(None, true, false), "review");
        assert_eq!(kind_of(None, false, true), "rules");
        let graph = Question {
            blocks: "b".to_string(),
            ..Question::default()
        };
        assert_eq!(kind_of(Some(&graph), false, false), "graph");
        assert_eq!(kind_of(None, false, false), "spec");
    }

    #[test]
    fn a_wait_names_the_issue_waited_for_and_never_the_issue_itself() {
        for on in ["20", r#""20""#, r##""#20""##, r#"" #20 ""#] {
            let text = format!(r##"{{"action":"Wait","on":{on},"summary":"#20 first"}}"##);
            assert_eq!(
                parse(&text, false, 15).expect("read").action,
                Action::Wait { on: 20 },
                "{on}"
            );
        }
        assert_eq!(Action::Wait { on: 20 }.name(), "wait");
        assert!(
            parse(r#"{"action":"wait","summary":"x"}"#, true, 15)
                .expect_err("no issue")
                .contains("names no issue")
        );
        assert!(
            parse(r#"{"action":"wait","on":"soon","summary":"x"}"#, true, 15)
                .expect_err("no number")
                .contains("names no issue")
        );
        assert!(
            parse(r#"{"action":"wait","on":15,"summary":"x"}"#, true, 15)
                .expect_err("itself")
                .contains("itself")
        );
        assert_eq!(
            parse(r#"{"action":"answer","on":15,"summary":"x"}"#, true, 15)
                .expect("read")
                .action,
            Action::Answer,
            "`on` means nothing to another action"
        );
    }

    #[test]
    fn the_section_keeps_the_question_and_says_the_ai_decided() {
        let decided = Decided {
            action: Action::Answer,
            summary: "Stripe".to_string(),
            answer: "- test mode first".to_string(),
            confidence: 4,
        };
        let asked = Question {
            kind: Some("stop".to_string()),
            blocks: "which provider".to_string(),
            round: 2,
            ..Question::default()
        };
        let text = section(Some(&asked), "stop", &decided);
        let read = Question::parse(&text).expect("still a question");
        assert_eq!(read.round, 2);
        assert_eq!(read.decided_by.as_deref(), Some("ai"));
        assert_eq!(read.verdict.as_deref(), Some("answer — Stripe (4/5)"));
        assert!(text.ends_with("\n\n- test mode first"));
        let fresh = section(None, "spec", &decided);
        assert!(fresh.starts_with("kind: spec\nblocks: a decision was asked (spec)"));
    }

    proptest! {
        #[test]
        fn reading_any_answer_never_panics(text in ".{0,300}", has_pr in any::<bool>(), number in any::<u64>()) {
            let _ = parse(&text, has_pr, number);
        }
    }
}
