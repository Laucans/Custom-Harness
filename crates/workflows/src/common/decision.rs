//! The `## Decision` section: a question the technical refinement could not
//! settle inside the task's `## Architecture`, and what was decided about it.
//!
//! The refinement raises it; the arbiter reads it, settles what it may and
//! writes its verdict under it; what the arbiter may not settle goes to the
//! `decide` workflow, which decides and tells the human. `round` counts how
//! many times the question was raised. A `kind:` line says what kind of
//! question it is when it is not the refinement's: a session that stopped
//! (`stop`), the breaker (`breaker`), a review its repairs did not clear
//! (`review`).

/// What the refinement found it could not honour, and what became of it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Question {
    /// What kind of question: `stop`, `breaker`, `review`, … — `None` for
    /// the refinement's graph question.
    pub kind: Option<String>,
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
    /// Who settled it: `harness` (mechanically), `ai` (the arbiter or
    /// `decide`), `owner` (a human) — or nobody yet.
    pub decided_by: Option<String>,
    /// What was decided.
    pub verdict: Option<String>,
    /// The milestone the task belongs to, for a stop that declared missing
    /// tasks: where a decision that approves them opens them.
    pub milestone: Option<u64>,
    /// What a stop declared missing, one key per slice (its branch): the
    /// same keys asked again after a decision are no new fact.
    pub declared: Vec<String>,
    /// The verdict `decide` gave on `declared`, carried by the refusals
    /// that send it back to the session: a ruling outlives the question.
    pub ruled: Option<String>,
    /// The round `ruled` was given at.
    pub ruled_at: Option<u32>,
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
            if let Some(value) = line.strip_prefix("kind:") {
                found.kind = Some(value.trim().to_string()).filter(|v| !v.is_empty() && v != "-");
            } else if let Some(value) = line.strip_prefix(BLOCKS) {
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
            } else if let Some(value) = line.strip_prefix("milestone:") {
                found.milestone = value.trim().trim_start_matches('#').parse().ok();
            } else if let Some(value) = line.strip_prefix("declared:") {
                found.declared = list(value);
            } else if let Some(value) = line.strip_prefix("ruled:") {
                found.ruled = Some(value.trim().to_string()).filter(|v| !v.is_empty());
            } else if let Some(value) = line.strip_prefix("ruled-at:") {
                found.ruled_at = value.trim().parse().ok();
            }
        }
        (seen && !found.blocks.is_empty()).then_some(found)
    }

    /// The section's text: one `key: value` line each.
    #[must_use]
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        if let Some(kind) = &self.kind {
            lines.push(format!("kind: {kind}"));
        }
        lines.push(format!("{BLOCKS} {}", self.blocks));
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
        if let Some(milestone) = self.milestone {
            lines.push(format!("milestone: #{milestone}"));
        }
        if !self.declared.is_empty() {
            lines.push(format!("declared: {}", self.declared.join(", ")));
        }
        if let Some(ruled) = &self.ruled {
            lines.push(format!("ruled: {ruled}"));
        }
        if let Some(at) = self.ruled_at {
            lines.push(format!("ruled-at: {at}"));
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

    /// Raised once already and settled: the arbiter does not settle it a
    /// second time, `decide` does.
    #[must_use]
    pub const fn raised_again(&self) -> bool {
        self.round >= 2
    }

    /// Whether `decided_by` is a human's: what no agent decides over again.
    #[must_use]
    pub fn decided_by_owner(&self) -> bool {
        self.decided_by.as_deref() == Some("owner")
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
    fn a_kind_and_an_owner_s_decision_read_back() {
        let question = Question {
            kind: Some("stop".to_string()),
            blocks: "which payment provider".to_string(),
            round: 2,
            decided_by: Some("owner".to_string()),
            verdict: Some("Stripe".to_string()),
            ..Question::default()
        };
        let read = Question::parse(&question.render()).expect("a question");
        assert_eq!(read, question);
        assert!(read.decided_by_owner());
        assert!(question.render().starts_with("kind: stop\nblocks:"));
        let graph = Question::parse("blocks: x").expect("a question");
        assert_eq!(graph.kind, None);
        assert!(!graph.decided_by_owner());
    }

    #[test]
    fn a_question_round_trips_through_its_text() {
        let question = Question {
            kind: None,
            blocks: "the list needs the campaign's characters".to_string(),
            needs: vec!["Personnage".to_string(), "campagne.listPjs".to_string()],
            produces: Vec::new(),
            cut: None,
            round: 2,
            decided_by: Some("ai".to_string()),
            verdict: Some("chained on #12".to_string()),
            milestone: Some(28),
            declared: vec!["feat/a".to_string(), "feat/b".to_string()],
            ruled: Some("answer — build it here (4/5)".to_string()),
            ruled_at: Some(3),
        };
        let text = question.render();
        assert_eq!(
            text,
            "blocks: the list needs the campaign's characters\nneeds: Personnage, campagne.listPjs\n\
             round: 2\ndecided-by: ai\nverdict: chained on #12\nmilestone: #28\ndeclared: feat/a, \
             feat/b\nruled: answer — build it here (4/5)\nruled-at: 3"
        );
        assert_eq!(Question::parse(&text), Some(question));
    }

    proptest::proptest! {
        #[test]
        fn a_declaration_and_its_ruling_round_trip(
            declared in proptest::collection::vec("[a-z0-9][a-z0-9/_.-]{0,40}", 0..6),
            milestone in proptest::option::of(1u64..100_000),
            ruled in proptest::option::of("[a-z][a-z0-9 ,:#()/—-]{0,60}[a-z0-9)]"),
            ruled_at in proptest::option::of(1u32..50),
        ) {
            let question = Question {
                kind: Some("refused".to_string()),
                blocks: "AGENT_LOOP_STOP: task x, y".to_string(),
                round: 4,
                milestone,
                declared,
                ruled,
                ruled_at,
                ..Question::default()
            };
            proptest::prop_assert_eq!(Question::parse(&question.render()), Some(question));
        }
    }

    #[test]
    fn an_empty_ruling_or_a_bad_round_is_none() {
        let read = Question::parse("blocks: x\nruled:   \nruled-at: soon").expect("q");
        assert_eq!(read.ruled, None);
        assert_eq!(read.ruled_at, None);
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
