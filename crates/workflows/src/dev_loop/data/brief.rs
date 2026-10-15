//! What each stage's prompt carries of the brief, and what it leaves out.
//!
//! The waste this exists to remove is structural rather than accidental: the
//! three stages of a round received **the same** SCOPE block, so each one paid,
//! on every turn, for the parts of the brief the other two act on.
//!
//! Measured on #65 — SCOPE was 60 363 characters, and the body alone 40 367:
//!
//! | section | characters |
//! | --- | --- |
//! | `## Business Goal` | 1 557 |
//! | `## Acceptance Criteria` | 2 238 |
//! | `## Business Rules` | 7 873 |
//! | `## Technical` | 9 306 |
//! | `## Technical Implementation Plan` | 16 861 |
//! | `## Assumptions (autonomous run)` | 2 333 |
//!
//! `technical-refinement` was receiving 26 167 characters of sections **it was
//! about to write**.
//!
//! # What this is not
//!
//! Not a cost optimisation, and it should not be sold as one. The fixed prompt
//! is about 17 000 tokens of a context that averaged 200 000 across #65's
//! `code` stage — the rest is accumulated tool output. Halving SCOPE removes a
//! few percent of the cache read, which is inside the noise of two runs of the
//! same task.
//!
//! What it is: a prompt that stops telling a stage about work that is not its
//! own. The claim is attention and honesty, not dollars, and neither is
//! something this repository can measure.
//!
//! # Why a list of headings and not a list of keys
//!
//! `## Assumptions (autonomous run)` is written by a session and is not one of
//! `common::sections::SECTIONS`. A drop-list of canonical keys could not name
//! it, and the heading text is what the issue actually carries — see
//! [`sections::without`](crate::common::sections::without) for how a name is
//! compared.

use harness_architecture::vocabulary::Declaration;
use harness_core::domain::{Scope, Sibling};

use crate::common::sections;

/// How much of the milestone's body an executing stage carries, at most.
///
/// Its goal and its `## Architecture` — what says what the batch delivers and
/// where this task sits in it — and no more: the rest was what `split` and
/// the refinement decided from, and the task's own plan carries the result.
/// Whole sections, never a cut mid-sentence (`sections::within`).
pub const MILESTONE_BUDGET: usize = 2_000;

/// How much of the task's own body a stage carries, at most.
///
/// A guard against the outlier, not the thing that decides: #65's refined
/// body was 40 367 characters, and every section of it was the stage's to
/// read. What this catches is a body that grew past what a plan needs — a
/// round of refinement that pasted a document — and it drops whole trailing
/// sections, naming them, rather than cutting a plan mid-step.
pub const TASK_BUDGET: usize = 48_000;

/// What the milestone keeps for an executing stage: its goal and its place
/// in the architecture. Matched on the heading text, as `sections::keeping`
/// does.
const MILESTONE_KEPT: [&str; 2] = ["business goal", "architecture"];

/// What a stage's prompt carries of the brief, and which of the body's sections
/// it leaves out.
///
/// The variants mirror [`prompts::Brief`](harness_core::domain::prompts::Brief)
/// — this one adds the sections to drop, which is a decision about stages and
/// therefore cannot live in `core`.
#[derive(Debug, Clone, Copy)]
pub enum Cut {
    /// Nothing: the stage works on no task.
    Unscoped,
    /// The task and the hierarchy around it, minus these headings.
    Situated(&'static [&'static str]),
    /// The task alone, minus these headings.
    TaskOnly(&'static [&'static str]),
    /// The task, its milestone's goal and architecture, and the neighbours
    /// it builds on or delivers to — no roadmap — minus these headings.
    ///
    /// For the stage that executes a plan: the roadmap was a third of SCOPE,
    /// identical for every task of every milestone. See [`executing`].
    Executing(&'static [&'static str]),
}

impl Cut {
    /// The headings dropped from the issue body before it is injected.
    #[must_use]
    pub const fn without(self) -> &'static [&'static str] {
        match self {
            Self::Unscoped => &[],
            Self::Situated(headings) | Self::TaskOnly(headings) | Self::Executing(headings) => {
                headings
            }
        }
    }
}

/// The scope an executing stage receives, reduced.
///
/// No roadmap, the milestone's goal and architecture under
/// [`MILESTONE_BUDGET`], the task's body under [`TASK_BUDGET`], and only the
/// neighbours related to the task.
///
/// `declaration` is the task's own `## Architecture`. Without one — a task
/// opened by hand — every neighbour is kept: a reduction that cannot tell
/// which neighbours matter keeps them all rather than guessing none.
#[must_use]
pub fn executing(scope: Scope, declaration: Option<&Declaration>) -> Scope {
    let fetch = format!("gh issue view {}", scope.milestone.number);
    let milestone_body = sections::within(
        &sections::keeping(&scope.milestone.body, &MILESTONE_KEPT),
        MILESTONE_BUDGET,
        &fetch,
    );
    let siblings = related(&scope.siblings, &scope.task.number, declaration);
    let task_fetch = format!("gh issue view {}", scope.task.number);
    let task_body = sections::within(&scope.task.body, TASK_BUDGET, &task_fetch);
    Scope {
        roadmap: None,
        siblings,
        milestone: harness_core::domain::Named {
            body: milestone_body,
            ..scope.milestone
        },
        task: harness_core::domain::Named {
            body: task_body,
            ..scope.task
        },
    }
}

/// The neighbours the task builds on or delivers to — those whose ids meet
/// its `needs` or `produces` — and the task itself. All of them when the
/// task declares nothing.
fn related(siblings: &[Sibling], current: &str, declaration: Option<&Declaration>) -> Vec<Sibling> {
    let Some(declaration) = declaration else {
        return siblings.to_vec();
    };
    let mine: Vec<String> = declaration
        .needs
        .iter()
        .chain(declaration.produces.iter())
        .map(|id| id.to_lowercase())
        .collect();
    siblings
        .iter()
        .filter(|sibling| {
            sibling.number == current
                || sibling
                    .ids
                    .iter()
                    .any(|id| mine.contains(&id.to_lowercase()))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unscoped_stage_names_no_section_to_drop() {
        // There is no body to drop one from: the list has to be empty rather
        // than whatever the previous variant carried.
        assert_eq!(Cut::Unscoped.without(), [] as [&str; 0]);
    }

    #[test]
    fn both_scoped_shapes_carry_their_own_drop_list() {
        assert_eq!(Cut::Situated(&["Technical"]).without(), ["Technical"]);
        assert_eq!(
            Cut::TaskOnly(&["Business Goal"]).without(),
            ["Business Goal"]
        );
        assert_eq!(Cut::Executing(&["Assumptions"]).without(), ["Assumptions"]);
    }

    use harness_core::domain::Named;

    fn sibling(number: &str, ids: &[&str]) -> Sibling {
        Sibling {
            number: number.to_string(),
            title: format!("task {number}"),
            status: "open".to_string(),
            gist: String::new(),
            ids: ids.iter().map(|id| (*id).to_string()).collect(),
        }
    }

    fn scope() -> Scope {
        Scope {
            roadmap: Some(Named {
                number: "5".to_string(),
                title: "the plan".to_string(),
                body: "a long story".to_string(),
            }),
            siblings: vec![
                sibling("30", &["credit.calculateRisk"]),
                sibling("31", &["orders.list"]),
                sibling("34", &["RiskBadge", "credit.calculateRisk"]),
                sibling("35", &["RiskBadge"]),
                sibling("36", &[]),
            ],
            milestone: Named {
                number: "12".to_string(),
                title: "Risk".to_string(),
                body: "## Business Goal\n\nsee the risk\n\n## Acceptance Criteria\n\n- a\n\n\
                       ## Business Rules\n\n- b\n\n## Architecture\n\nsystems: credit\n"
                    .to_string(),
            },
            task: Named {
                number: "34".to_string(),
                title: "RiskBadge".to_string(),
                body: "## Architecture\n\nunit: micro-ui\nsystem: credit\n\
                       produces: RiskBadge\nneeds: credit.calculateRisk\n"
                    .to_string(),
            },
        }
    }

    fn declaration(scope: &Scope) -> Declaration {
        let found = sections::parse(&scope.task.body);
        Declaration::parse(found.get(sections::ARCHITECTURE).expect("declared")).expect("a unit")
    }

    #[test]
    fn an_executing_scope_drops_the_roadmap_and_keeps_the_milestone_s_goal_and_architecture() {
        let here = scope();
        let declared = declaration(&here);
        let reduced = executing(here, Some(&declared));
        assert!(reduced.roadmap.is_none());
        assert!(
            reduced.milestone.body.contains("## Business Goal"),
            "{}",
            reduced.milestone.body
        );
        assert!(
            reduced.milestone.body.contains("## Architecture"),
            "{}",
            reduced.milestone.body
        );
        assert!(
            !reduced.milestone.body.contains("Acceptance Criteria"),
            "{}",
            reduced.milestone.body
        );
        assert!(
            !reduced.milestone.body.contains("Business Rules"),
            "{}",
            reduced.milestone.body
        );
    }

    #[test]
    fn only_the_neighbours_sharing_an_id_with_the_task_stay_and_the_task_itself() {
        let here = scope();
        let declared = declaration(&here);
        let reduced = executing(here, Some(&declared));
        let numbers: Vec<&str> = reduced.siblings.iter().map(|s| s.number.as_str()).collect();
        assert_eq!(
            numbers,
            ["30", "34", "35"],
            "30 produces what it needs, 35 needs what it produces"
        );
    }

    #[test]
    fn a_task_declaring_nothing_keeps_every_neighbour() {
        let reduced = executing(scope(), None);
        assert_eq!(reduced.siblings.len(), 5);
    }

    #[test]
    fn ids_are_matched_without_regard_to_case() {
        let mut here = scope();
        here.siblings = vec![sibling("30", &["CREDIT.CALCULATERISK"])];
        let declared = declaration(&here);
        let reduced = executing(here, Some(&declared));
        assert_eq!(reduced.siblings.len(), 1);
    }

    #[test]
    fn a_milestone_over_budget_is_cut_on_whole_sections_and_says_so() {
        let mut here = scope();
        here.milestone.body = format!(
            "## Business Goal\n\n{}\n\n## Architecture\n\nsystems: credit\n",
            "g".repeat(MILESTONE_BUDGET - 30)
        );
        let reduced = executing(here, None);
        assert!(reduced.milestone.body.contains("## Business Goal"));
        assert!(
            !reduced.milestone.body.contains("systems: credit"),
            "{}",
            reduced.milestone.body
        );
        assert!(
            reduced
                .milestone
                .body
                .contains("`gh issue view 12` reads them whole"),
            "{}",
            reduced.milestone.body
        );
    }

    #[test]
    fn a_task_body_over_budget_loses_whole_trailing_sections_and_names_them() {
        let mut here = scope();
        here.task.body = format!(
            "## Technical Implementation Plan\n\n{}\n\n## Assumptions (autonomous run)\n\n- x\n",
            "p".repeat(TASK_BUDGET - 50)
        );
        let reduced = executing(here, None);
        assert!(
            reduced
                .task
                .body
                .starts_with("## Technical Implementation Plan")
        );
        assert!(
            !reduced.task.body.contains("- x"),
            "{}",
            &reduced.task.body[reduced.task.body.len() - 200..]
        );
        assert!(
            reduced
                .task
                .body
                .contains("`gh issue view 34` reads them whole")
        );
    }

    #[test]
    fn a_task_body_under_budget_is_carried_whole() {
        let here = scope();
        let body = here.task.body.clone();
        let reduced = executing(here, None);
        assert_eq!(reduced.task.body, body.trim());
    }
}
