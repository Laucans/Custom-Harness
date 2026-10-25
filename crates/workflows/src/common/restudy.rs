//! When a task is due a re-study — read here, with no I/O, for the stop
//! handler that counts the discoveries, the router that routes the
//! re-study, and the re-study itself.
//!
//! `ForgeDesMondes`' #219, 2026-10-25: six stops in one day, each declaring
//! the next missing units — a journey stops at its first failure, which
//! hides the next. Two stops in a row that opened a missing task are a
//! **streak**, kept on the task's `## Decision` (`discovered:`,
//! `restudied:`, `restudies:`), so a restart reads it back: at
//! [`AFTER`] discoveries a re-study walks every rule at once, then again
//! [`EVERY`] discoveries later, at most [`MAX`] times a streak.

use harness_core::domain::Issue;

use crate::common::decision::Question;
use crate::common::sections;

/// The discoveries in a row a first re-study waits for.
pub const AFTER: u32 = 2;
/// The discoveries after a re-study before the next one.
pub const EVERY: u32 = 3;
/// The most re-studies a streak pays for.
pub const MAX: u32 = 2;

/// A task's streak of discoveries, as its `## Decision` carries it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Streak {
    /// Stops in a row that opened at least one missing task.
    pub discovered: u32,
    /// `discovered` when the last re-study ran.
    pub restudied: Option<u32>,
    /// Re-studies run in this streak.
    pub restudies: u32,
}

impl Streak {
    /// The streak a question carries.
    #[must_use]
    pub const fn of(question: &Question) -> Self {
        Self {
            discovered: question.discovered,
            restudied: question.restudied,
            restudies: question.restudies,
        }
    }

    /// The streak after a stop: one more discovery when the stop opened a
    /// task, carried as it was otherwise (a refusal, a decision asked).
    #[must_use]
    pub fn after(previous: Option<&Question>, opened: bool) -> Self {
        let carried = previous.map(Self::of).unwrap_or_default();
        Self {
            discovered: if opened {
                carried.discovered.saturating_add(1)
            } else {
                carried.discovered
            },
            ..carried
        }
    }

    /// The streak once a re-study ran on it.
    #[must_use]
    pub const fn restudied(self) -> Self {
        Self {
            restudied: Some(self.discovered),
            restudies: self.restudies.saturating_add(1),
            ..self
        }
    }

    /// `question`, carrying this streak.
    #[must_use]
    pub fn onto(self, question: Question) -> Question {
        Question {
            discovered: self.discovered,
            restudied: self.restudied,
            restudies: self.restudies,
            ..question
        }
    }
}

/// Whether the question's streak is due a re-study.
///
/// [`AFTER`] discoveries in a row, fewer than [`MAX`] re-studies, [`EVERY`]
/// more discoveries since the last one, a milestone to open the slices
/// under, and no owner's decision to study over.
#[must_use]
pub fn due(question: &Question) -> bool {
    question.discovered >= AFTER
        && question.restudies < MAX
        && question
            .restudied
            .is_none_or(|at| question.discovered >= at.saturating_add(EVERY))
        && question.milestone.is_some()
        && !question.decided_by_owner()
}

/// Whether the issue's `## Decision` is due a re-study ([`due`]).
#[must_use]
pub fn due_on(issue: &Issue) -> bool {
    question_of(issue).is_some_and(|question| due(&question))
}

/// The issue's `## Decision`, when it reads as a question.
#[must_use]
pub fn question_of(issue: &Issue) -> Option<Question> {
    sections::parse(&issue.body)
        .get(sections::DECISION)
        .and_then(|text| Question::parse(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(discovered: u32, at: Option<u32>, count: u32) -> Question {
        Question {
            kind: Some("wait".to_string()),
            blocks: "AGENT_LOOP_STOP: tasks [...]".to_string(),
            round: 1,
            milestone: Some(28),
            discovered,
            restudied: at,
            restudies: count,
            ..Question::default()
        }
    }

    #[test]
    fn a_re_study_is_due_at_the_second_discovery_then_three_later_twice_at_most() {
        let table = [
            (0, None, 0, false),
            (1, None, 0, false),
            (2, None, 0, true),
            (3, None, 0, true),
            (2, Some(2), 1, false),
            (3, Some(2), 1, false),
            (4, Some(2), 1, false),
            (5, Some(2), 1, true),
            (6, Some(2), 1, true),
            (5, Some(5), 2, false),
            (9, Some(5), 2, false),
            (9, None, 2, false),
        ];
        for (discovered, restudied, restudies, expected) in table {
            assert_eq!(
                due(&question(discovered, restudied, restudies)),
                expected,
                "{discovered} {restudied:?} {restudies}"
            );
        }
        let nowhere = Question {
            milestone: None,
            ..question(2, None, 0)
        };
        assert!(!due(&nowhere), "no milestone to open the slices under");
        let owner = Question {
            decided_by: Some("owner".to_string()),
            ..question(2, None, 0)
        };
        assert!(!due(&owner), "an owner's decision is never studied over");
        let ai = Question {
            decided_by: Some("ai".to_string()),
            ..question(2, None, 0)
        };
        assert!(due(&ai));
    }

    #[test]
    fn a_streak_counts_the_stops_that_opened_carries_the_rest_and_starts_at_nothing() {
        let previous = question(1, Some(1), 1);
        assert_eq!(
            Streak::after(Some(&previous), true),
            Streak {
                discovered: 2,
                restudied: Some(1),
                restudies: 1
            }
        );
        assert_eq!(Streak::after(Some(&previous), false), Streak::of(&previous));
        assert_eq!(
            Streak::after(None, true),
            Streak {
                discovered: 1,
                ..Streak::default()
            }
        );
        assert_eq!(Streak::after(None, false), Streak::default());
        let restudied = Streak::after(Some(&previous), true).restudied();
        assert_eq!(
            restudied,
            Streak {
                discovered: 2,
                restudied: Some(2),
                restudies: 2
            }
        );
        let onto = restudied.onto(Question::default());
        assert_eq!(
            (onto.discovered, onto.restudied, onto.restudies),
            (2, Some(2), 2)
        );
    }

    #[test]
    fn an_issue_is_due_by_its_decision_section() {
        let issue = |body: &str| Issue {
            body: body.to_string(),
            ..Issue::default()
        };
        let body = format!(
            "## Scope\n\nx\n\n## Decision\n\n{}",
            question(2, None, 0).render()
        );
        assert!(due_on(&issue(&body)));
        assert!(!due_on(&issue("## Scope\n\nx")));
        assert!(!due_on(&issue(
            &body.replace("discovered: 2", "discovered: 1")
        )));
        assert_eq!(question_of(&issue(&body)).map(|q| q.discovered), Some(2));
    }
}
