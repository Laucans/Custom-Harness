//! The `## Decision` section: a question the technical refinement could not
//! settle inside the task's `## Architecture`, and what was decided about it.
//!
//! The refinement raises it; the arbiter reads it, settles what it may and
//! writes its verdict under it; a human reads what is left. `round` counts
//! how many times the question was raised: the first time goes to the
//! arbiter, the second to a human — a question raised again after an
//! arbitration is one the arbitration did not settle, and no second agent
//! is paid to disagree with the first.

/// What the refinement found it could not honour, and what became of it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Question {
    /// One sentence on what does not hold.
    pub blocks: String,
    /// The ids the design builds on that `needs` does not name.
    pub needs: Vec<String>,
    /// The ids the task would have to deliver beyond `produces`.
    pub produces: Vec<String>,
    /// How the task should be cut in two, if it should.
    pub cut: Option<String>,
    /// How many times the question was raised, from 1.
    pub round: u32,
    /// Who settled it: `harness` (mechanically), `ai` (the arbiter) — or
    /// nobody yet.
    pub decided_by: Option<String>,
    /// What was decided, or why it waits for a human.
    pub verdict: Option<String>,
}

/// The line the question's reason is on; a section without it is no question.
const BLOCKS: &str = "blocks:";

impl Question {
    /// Reads a question from the section's text, or from a reply shaped the
    /// same way: `None` without a `blocks:` line.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut found = Self {
            round: 1,
            ..Self::default()
        };
        let mut seen = false;
        for line in text.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix(BLOCKS) {
                found.blocks = value.trim().to_string();
                seen = true;
            } else if let Some(value) = line.strip_prefix("needs:") {
                found.needs = list(value);
            } else if let Some(value) = line.strip_prefix("produces:") {
                found.produces = list(value);
            } else if let Some(value) = line.strip_prefix("cut:") {
                found.cut = Some(value.trim().to_string()).filter(|v| !v.is_empty() && v != "-");
            } else if let Some(value) = line.strip_prefix("round:") {
                found.round = value.trim().parse().unwrap_or(1).max(1);
            } else if let Some(value) = line.strip_prefix("decided-by:") {
                found.decided_by =
                    Some(value.trim().to_string()).filter(|v| !v.is_empty() && v != "-");
            } else if let Some(value) = line.strip_prefix("verdict:") {
                found.verdict = Some(value.trim().to_string()).filter(|v| !v.is_empty());
            }
        }
        (seen && !found.blocks.is_empty()).then_some(found)
    }

    /// The section's text: one `key: value` line each.
    #[must_use]
    pub fn render(&self) -> String {
        let mut lines = vec![format!("{BLOCKS} {}", self.blocks)];
        if !self.needs.is_empty() {
            lines.push(format!("needs: {}", self.needs.join(", ")));
        }
        if !self.produces.is_empty() {
            lines.push(format!("produces: {}", self.produces.join(", ")));
        }
        if let Some(cut) = &self.cut {
            lines.push(format!("cut: {cut}"));
        }
        lines.push(format!("round: {}", self.round));
        if let Some(who) = &self.decided_by {
            lines.push(format!("decided-by: {who}"));
        }
        if let Some(verdict) = &self.verdict {
            lines.push(format!("verdict: {verdict}"));
        }
        lines.join("\n")
    }

    /// Whether the question asks more than a `needs` the graph can settle:
    /// a `produces` to change or a task to cut is a decision on the plan,
    /// never the arbiter's.
    #[must_use]
    pub const fn changes_the_plan(&self) -> bool {
        !self.produces.is_empty() || self.cut.is_some()
    }

    /// Raised once already and settled, so this time goes to a human.
    #[must_use]
    pub const fn raised_again(&self) -> bool {
        self.round >= 2
    }
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|id| id.trim().trim_matches('`').to_string())
        .filter(|id| !id.is_empty() && id != "-")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_round_trips_through_its_text() {
        let question = Question {
            blocks: "the list needs the campaign's characters".to_string(),
            needs: vec!["Personnage".to_string(), "campagne.listPjs".to_string()],
            produces: Vec::new(),
            cut: None,
            round: 2,
            decided_by: Some("ai".to_string()),
            verdict: Some("chained on #12".to_string()),
        };
        let text = question.render();
        assert_eq!(
            text,
            "blocks: the list needs the campaign's characters\nneeds: Personnage, campagne.listPjs\n\
             round: 2\ndecided-by: ai\nverdict: chained on #12"
        );
        assert_eq!(Question::parse(&text), Some(question));
    }

    #[test]
    fn without_a_blocks_line_there_is_no_question() {
        assert_eq!(Question::parse("needs: A\nround: 1"), None);
        assert_eq!(Question::parse("blocks:   \nneeds: A"), None);
        assert_eq!(Question::parse(""), None);
    }

    #[test]
    fn a_fresh_question_is_round_one_and_dashes_are_nothing() {
        let found = Question::parse("blocks: no producer for Risk\nneeds: `Risk@3`, -\ncut: -")
            .expect("a question");
        assert_eq!(found.round, 1);
        assert_eq!(found.needs, ["Risk@3"]);
        assert_eq!(found.cut, None);
        assert!(found.decided_by.is_none());
        assert!(!found.changes_the_plan());
        assert!(!found.raised_again());
    }

    #[test]
    fn a_produces_or_a_cut_changes_the_plan() {
        let produces = Question::parse("blocks: x\nproduces: campagne.closePj").expect("q");
        assert!(produces.changes_the_plan());
        let cut = Question::parse("blocks: x\ncut: one task per command").expect("q");
        assert!(cut.changes_the_plan());
        assert_eq!(
            cut.render(),
            "blocks: x\ncut: one task per command\nround: 1"
        );
    }

    #[test]
    fn a_dash_or_nothing_after_decided_by_is_nobody() {
        assert_eq!(
            Question::parse("blocks: x\ndecided-by: -")
                .expect("q")
                .decided_by,
            None
        );
        assert_eq!(
            Question::parse("blocks: x\ndecided-by:  ")
                .expect("q")
                .decided_by,
            None
        );
        assert_eq!(
            Question::parse("blocks: x\ndecided-by: harness")
                .expect("q")
                .decided_by
                .as_deref(),
            Some("harness")
        );
        assert_eq!(
            Question::parse("blocks: x\nverdict:  ").expect("q").verdict,
            None
        );
    }

    #[test]
    fn a_bad_round_reads_as_one_and_a_second_round_is_raised_again() {
        assert_eq!(
            Question::parse("blocks: x\nround: zero").expect("q").round,
            1
        );
        assert_eq!(Question::parse("blocks: x\nround: 0").expect("q").round, 1);
        assert!(
            Question::parse("blocks: x\nround: 2")
                .expect("q")
                .raised_again()
        );
    }
}
