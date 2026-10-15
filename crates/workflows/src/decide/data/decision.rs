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
        }
    }

    fn of(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "answer" | "proceed" => Some(Self::Answer),
            "merge" => Some(Self::Merge),
            "repair" => Some(Self::Repair),
            "cut" => Some(Self::Cut),
            _ => None,
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

/// The decision a session answered, for a case with or without a PR.
///
/// # Errors
///
/// No JSON object, an action it does not have, a `merge` or a `repair`
/// without a PR, an empty summary.
pub fn parse(text: &str, has_pr: bool) -> Result<Decided, String> {
    let answered: Answered =
        json_reply::parse_object(text).map_err(|e: ParseError| e.to_string())?;
    let action = Action::of(&answered.action).ok_or_else(|| {
        format!(
            "`{}` is no action: answer, merge, repair or cut",
            answered.action
        )
    })?;
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
pub fn kind_of(question: Option<&Question>, review_pending: bool) -> String {
    if let Some(kind) = question.and_then(|q| q.kind.clone()) {
        return kind;
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
        let read = parse(text, true).expect("read");
        assert_eq!(read.action, Action::Repair);
        assert_eq!(read.summary, "fix the null check");
        assert_eq!(read.confidence, 5, "clamped");
        assert!(
            parse(text, false)
                .expect_err("no PR")
                .contains("needs a pull request")
        );
        assert!(parse(r#"{"action":"vote","summary":"x"}"#, true).is_err());
        assert!(parse(r#"{"action":"cut","summary":" "}"#, true).is_err());
        assert_eq!(
            parse(r#"{"action":"proceed","summary":"go"}"#, false)
                .expect("an alias")
                .action,
            Action::Answer
        );
        assert!(parse("no json", true).is_err());
    }

    #[test]
    fn the_kind_is_the_question_s_or_what_the_issue_shows() {
        let stop = Question {
            kind: Some("stop".to_string()),
            blocks: "b".to_string(),
            ..Question::default()
        };
        assert_eq!(kind_of(Some(&stop), true), "stop");
        assert_eq!(kind_of(None, true), "review");
        let graph = Question {
            blocks: "b".to_string(),
            ..Question::default()
        };
        assert_eq!(kind_of(Some(&graph), false), "graph");
        assert_eq!(kind_of(None, false), "spec");
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
        fn reading_any_answer_never_panics(text in ".{0,300}", has_pr in any::<bool>()) {
            let _ = parse(&text, has_pr);
        }
    }
}
