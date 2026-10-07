//! What the business refinement concludes about the technical half: how much
//! this issue needs it run under a human's eyes, and how that is posted.
//!
//! Pure business logic: parsing a short reply and rendering a comment.
//!
//! # Why a score, and not only a yes/no
//!
//! The advice used to answer `human-in-the-loop: yes|no` alone. A binary says
//! what to do and nothing about how much it matters, so a board of twenty open
//! tasks gives a human twenty equal-looking verdicts to re-judge one by one.
//! The score is what makes a board triageable at a glance: a 5 is read first, a
//! 1 needs no reading at all.
//!
//! **It guides, it never decides.** Nothing here places a label: whether the
//! technical refinement runs on the issue (`harness:tech-refinement`) or
//! unattended inside the loop stays the owner's gesture. A score that silently
//! triggered a paid workflow would be a vote, not advice.

use std::fmt::Write as _;

use crate::common::labels;

/// The top of the scale. Named once: the prompt states it, the reader checks
/// against it, and a reply outside it is not a score.
pub const SCALE: u8 = 5;

/// From which score up a human is normally wanted — what the prompt asks for,
/// and what [`Read::disagrees`] measures the reply against.
pub const WANTS_A_HUMAN: u8 = 4;

/// The line the score is written on.
const SCORE_LINE: &str = "technical-refinement:";

/// The line the recommendation is written on.
const HUMAN_LINE: &str = "human-in-the-loop:";

/// What the advice reply says, as far as it can be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    /// How much the technical half needs a human, `1..=SCALE`.
    ///
    /// `None` when the reply did not say, in a shape this understands.
    /// **Never defaulted**: an invented 1 would send an irreversible decision
    /// through unattended, and an invented 5 would make every task wait on a
    /// human. Not knowing is its own answer, and it is said out loud.
    pub score: Option<u8>,
    /// Whether it recommends a human read the technical half on the issue.
    pub human: Option<bool>,
}

impl Read {
    /// Whether this advice puts a decision in front of a human.
    ///
    /// **The single source of truth for that question.** Both the gesture
    /// paragraph of [`comment`] and the `harness:needs-decision` label are
    /// decided here: a label that disagreed with the comment beside it would be
    /// worse than no label, since the board and the issue would each claim
    /// something different.
    ///
    /// `score.is_none()` counts as yes. Nothing was understood, so nobody can
    /// say the issue is safe to run unattended — and the one reading that is
    /// unreadable has to be a human.
    ///
    /// Deliberately **not** derived from the prose. The replies observed in
    /// practice write their questions under a heading of their own choosing, and
    /// in whichever language the issue is in — #63's advice came back in French,
    /// #64's in English. Hunting for "Questions for the human" would key a label
    /// on the one part of the reply with no agreed shape.
    #[must_use]
    pub const fn wants_a_human(&self) -> bool {
        matches!(self.human, Some(true)) || self.score.is_none()
    }

    /// Whether the two halves of the reply contradict each other.
    ///
    /// A 5 that recommends no human, or a 1 that demands one, is a reply whose
    /// two lines were not written by the same reasoning. Worth saying rather
    /// than picking one: the human is about to act on it.
    #[must_use]
    pub const fn disagrees(&self) -> bool {
        match (self.score, self.human) {
            (Some(score), Some(human)) => human != (score >= WANTS_A_HUMAN),
            _ => false,
        }
    }
}

/// The score on a `technical-refinement:` line, if there is a readable one.
///
/// Accepts `4`, `4/5`, `**4/5**` — what a model actually writes. A number
/// outside the scale is refused rather than clamped: a `7/5` means the reply
/// did not use this scale, and pulling it down to 5 would hide that.
fn score_in(line: &str) -> Option<u8> {
    let digits: String = line
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    let score = digits.parse::<u8>().ok()?;
    (1..=SCALE).contains(&score).then_some(score)
}

/// Whether a `human-in-the-loop:` line says yes.
fn says_yes(line: &str) -> Option<bool> {
    let after = line.split_once(':')?.1.trim().to_lowercase();
    if after.starts_with("yes") {
        return Some(true);
    }
    if after.starts_with("no") {
        return Some(false);
    }
    None
}

/// Reads the two answers out of the reply, each independently of the other.
///
/// Independently on purpose: a reply that got one line right and the other
/// wrong still carries the half it got right, and the comment then says which
/// half is missing.
#[must_use]
pub fn read(text: &str) -> Read {
    let mut found = Read {
        score: None,
        human: None,
    };
    for line in text.lines() {
        let lowered = line
            .trim()
            .trim_start_matches(['*', '#', '-', ' '])
            .to_lowercase();
        if found.score.is_none() && lowered.starts_with(SCORE_LINE) {
            found.score = score_in(&lowered);
        }
        if found.human.is_none() && lowered.starts_with(HUMAN_LINE) {
            found.human = says_yes(&lowered);
        }
    }
    found
}

/// The first line of the comment: what a human reads without opening it.
#[must_use]
pub fn headline(found: &Read) -> String {
    let score = found.score.map_or_else(
        || "necessity unstated".to_string(),
        |score| format!("necessity {score}/{SCALE}"),
    );
    let human = match found.human {
        Some(true) => "human-in-the-loop: yes",
        Some(false) => "human-in-the-loop: no",
        None => "human-in-the-loop unstated",
    };
    format!("Technical refinement — {score} · {human}")
}

/// Whether a line of the reply's head is one the headline already carries.
fn is_a_marker_line(line: &str) -> bool {
    let lowered = line
        .trim()
        .trim_start_matches(['*', '#', '-', ' '])
        .to_lowercase();
    lowered.starts_with(SCORE_LINE) || lowered.starts_with(HUMAN_LINE)
}

/// The reasons and questions alone: the two answer lines are dropped, since the
/// headline renders them.
///
/// Only at the head of the reply — the first three lines, which is where the
/// shape puts them. Further down, a line naming one of the two markers is a
/// sentence of reasoning, and dropping it would lose a reason.
fn reasons_in(text: &str) -> String {
    let mut lines: Vec<&str> = text.trim().lines().collect();
    let head = lines.len().min(3);
    lines = lines
        .iter()
        .enumerate()
        .filter(|(at, line)| *at >= head || !is_a_marker_line(line))
        .map(|(_, line)| *line)
        .collect();
    lines.join("\n").trim().to_string()
}

/// The comment posted on the issue: the headline, the gesture it implies, then
/// the reasons.
///
/// The gesture is spelled out because this is the one place a human is being
/// asked to do something, and "run the technical refinement on the issue" is a
/// label nobody remembers the spelling of.
#[must_use]
pub fn comment(number: u64, text: &str) -> String {
    let found = read(text);
    let mut out = headline(&found);
    if found.disagrees() {
        out.push_str(
            "\n\n> The score and the recommendation above do not agree — read \
             the reasons before acting on either.",
        );
    }
    if found.wants_a_human() {
        let label = labels::TECH_REFINEMENT;
        let _ = write!(
            out,
            "\n\nTo run it on the issue, where you can read it and answer \
             before anything is built: `gh issue edit {number} --add-label \
             {label}`. Left alone, the development loop writes the technical \
             sections itself, unattended."
        );
    }
    let reasons = reasons_in(text);
    if !reasons.is_empty() {
        out.push_str("\n\n");
        out.push_str(&reasons);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = "technical-refinement: 4/5
human-in-the-loop: yes

- The schema later tasks read is decided here.
- Q1: which folder holds a session?";

    #[test]
    fn a_well_shaped_reply_gives_both_answers() {
        let found = read(REPLY);
        assert_eq!(found.score, Some(4));
        assert_eq!(found.human, Some(true));
        assert!(!found.disagrees());
    }

    #[test]
    fn the_score_is_read_through_the_decoration_a_model_adds() {
        for line in [
            "technical-refinement: 2",
            "**technical-refinement: 2/5**",
            "- Technical-Refinement: 2 / 5",
            "## technical-refinement: 2",
        ] {
            assert_eq!(read(line).score, Some(2), "{line}");
        }
    }

    #[test]
    fn a_score_off_the_scale_is_not_a_score() {
        // `7/5` means the reply did not use this scale. Clamping it to 5 would
        // hide that, and 5 is the answer that stops a task.
        assert_eq!(read("technical-refinement: 7/5").score, None);
        assert_eq!(read("technical-refinement: 0").score, None);
        assert_eq!(read("technical-refinement: none").score, None);
    }

    #[test]
    fn a_reply_that_says_neither_invents_neither() {
        let found = read("I think a human should probably look at this one.");
        assert_eq!(found.score, None);
        assert_eq!(found.human, None);
        assert!(!found.disagrees(), "nothing to disagree with");
        assert!(headline(&found).contains("unstated"));
    }

    #[test]
    fn one_half_missing_does_not_lose_the_other() {
        let found = read("technical-refinement: 5\n\n- irreversible schema call");
        assert_eq!(found.score, Some(5));
        assert_eq!(found.human, None);
    }

    #[test]
    fn a_score_and_a_recommendation_that_contradict_are_flagged() {
        let found = read("technical-refinement: 5\nhuman-in-the-loop: no");
        assert!(found.disagrees());
        assert!(
            comment(50, "technical-refinement: 5\nhuman-in-the-loop: no").contains("do not agree")
        );
    }

    #[test]
    fn a_low_score_agrees_with_no_human() {
        let found = read("technical-refinement: 1\nhuman-in-the-loop: no");
        assert!(!found.disagrees());
    }

    #[test]
    fn the_comment_leads_with_what_is_read_at_a_glance() {
        let said = comment(50, REPLY);
        let first = said.lines().next().expect("a headline");
        assert!(first.contains("necessity 4/5"), "{first}");
        assert!(first.contains("human-in-the-loop: yes"), "{first}");
        // And the advice itself is kept whole, under it.
        assert!(said.contains("Q1: which folder holds a session?"));
    }

    #[test]
    fn the_two_answer_lines_are_not_repeated_under_the_headline() {
        let said = comment(50, REPLY);
        assert_eq!(
            said.matches("human-in-the-loop").count(),
            1,
            "said once, in the headline: {said}"
        );
        assert!(!said.contains("technical-refinement: 4/5"));
    }

    #[test]
    fn a_reason_further_down_that_names_a_marker_is_kept() {
        // Only the head of the reply holds the answer lines. Below, such a
        // sentence is reasoning, and dropping it would lose a reason.
        let said = comment(
            50,
            "technical-refinement: 4/5\nhuman-in-the-loop: yes\n\n- schema call\n\
             - human-in-the-loop: yes is also what #31 needed",
        );
        assert!(said.contains("is also what #31 needed"), "{said}");
    }

    #[test]
    fn a_yes_names_the_label_and_what_happens_without_it() {
        let said = comment(50, REPLY);
        assert!(said.contains(labels::TECH_REFINEMENT));
        assert!(said.contains("gh issue edit 50"));
        assert!(said.contains("unattended"));
    }

    #[test]
    fn a_no_does_not_ask_for_a_gesture_nobody_needs_to_make() {
        let said = comment(
            50,
            "technical-refinement: 1\nhuman-in-the-loop: no\n\n- mechanical",
        );
        assert!(!said.contains("gh issue edit"));
    }

    #[test]
    fn an_unreadable_reply_still_names_the_gesture() {
        // Nothing was understood, so the human has to decide: they get the
        // label spelled out rather than a comment that only says "unstated".
        let said = comment(50, "a paragraph with no shape at all");
        assert!(said.contains(labels::TECH_REFINEMENT));
    }
}
