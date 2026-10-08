//! Where a task sits: its roadmap item, its milestone, its sibling tasks —
//! and where a milestone sits: its roadmap item, its sibling milestones.
//!
//! Read by the loop and by the refinement, so every session that works on an
//! issue knows what its neighbours own.

use harness_core::domain::{Issue, Named, Outcome, Scope, Sibling};
use harness_core::ports::shell::github::GitHub;

use crate::common::{labels, sections};

/// How much of a neighbour's own words to carry, at most.
///
/// Per sibling, on top of the block-wide budget in
/// [`GIST_BUDGET`](harness_core::domain::prompts::GIST_BUDGET): one task whose
/// Scope section runs long must not eat the whole block.
///
/// **Lowered from 1 400 after measuring it.** On milestone #17 the eight
/// neighbours came to 7 811 characters in every prompt, and the gists were
/// sitting at the old cap — a refined `## Business Goal` runs to about 1 900,
/// so the budget was deciding the content. What a session needs from a
/// neighbour is one sentence, which is why [`boundary_first`] exists: the cap
/// is now small enough to force the choice, and the ordering makes the choice
/// the right one.
const GIST_MAX: usize = 300;

/// What a session needs of the roadmap: the boundaries, not the narrative.
///
/// Measured on roadmap #12 — 10 984 characters in every prompt, of which the
/// vision, the product tour, the typical-session walk-through and the
/// milestone-by-milestone journey are about 8 000. That is what a *human* reads
/// to understand the product. What keeps a session inside its scope is the
/// boundary and the non-negotiable constraint; the story does not, and it is
/// paid for on every turn.
///
/// Both spellings of each marker, because the heading is written by whoever
/// wrote the roadmap and [`sections::keeping`] matches the text as it stands. A
/// roadmap under none of them keeps its whole body rather than losing
/// everything.
const ROADMAP_KEPT: [&str; 13] = [
    "frontière",
    "frontiere",
    "boundar",
    "contrainte",
    "constraint",
    "critère de succès",
    "critere de succes",
    "success",
    "hors scope",
    "out of scope",
    // The roadmap's systems and Concepts: the bounded contexts a milestone
    // is drawn inside of, and the vocabulary its tasks implement.
    "system",
    "système",
    "concept",
];

/// Markers of the one line in a gist that must never be the part cut off.
const BOUNDARY: [&str; 7] = [
    "boundary",
    "frontière",
    "frontiere",
    "does not cover",
    "ne couvre pas",
    "hors scope",
    "out of scope",
];

/// The lines that state a boundary, then the rest in its written order.
///
/// A budget applied to the text as written cut exactly the sentence worth
/// keeping: a refined `## Business Goal` ends on its boundary bullet, so the
/// cap took the "what it does not cover" and left the "who" and the "why now".
/// Measured on #61 — 1 900 characters of goal, `- **Boundary:**` last.
///
/// Paragraph shape is lost when a line is pulled out of the middle. That is the
/// trade: this text is a bullet list by the time it matters, and a reordered
/// list still reads.
fn boundary_first(text: &str) -> String {
    let (edge, rest): (Vec<&str>, Vec<&str>) = text.lines().partition(|line| {
        let lower = line.to_lowercase();
        BOUNDARY.iter().any(|marker| lower.contains(marker))
    });
    if edge.is_empty() {
        return text.to_string();
    }
    let mut ordered = edge;
    ordered.extend(rest);
    ordered.join("\n")
}

/// What a neighbour is for, in its own words: a few lines, never its body.
///
/// Read in this order, and the order is the whole decision:
///
/// 1. `## Scope` — the slice's own boundary, written by `split`, and the one
///    thing a neighbour's reader actually needs: covered, not covered.
/// 2. `## Business Goal` — once a refinement has run, the shortest statement
///    of what the task is for. A few hundred tokens, against several thousand
///    for the body it sits in.
/// 3. the raw body, for a task nobody has structured yet — a hand-written one,
///    or one from a `split` that predates the Scope section.
///
/// Never the Technical sections, the plan, or the assumptions: those are a
/// neighbour's internals, and a session reading them starts doing its
/// neighbour's work.
///
/// Whichever of the three it came from, the boundary is moved to the front
/// before the cap applies — see [`boundary_first`].
fn gist_of(body: &str) -> String {
    let found = sections::parse(body);
    let text = found
        .get(sections::SCOPE)
        .or_else(|| found.get("business-goal"))
        .map_or_else(
            || body.trim().to_string(),
            |section| section.trim().to_string(),
        );
    cut_to(&boundary_first(&text), GIST_MAX)
}

/// The first whole lines of `text` that fit in `max` characters.
///
/// Whole lines, and whole characters: cutting mid-line leaves a sentence that
/// reads as a statement of fact with its qualifier removed, and slicing a
/// `String` by bytes would panic on the first accented character — this repo's
/// issues are written in French.
fn cut_to(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut kept = String::new();
    for line in text.lines() {
        if kept.chars().count() + line.chars().count() + 1 > max {
            break;
        }
        if !kept.is_empty() {
            kept.push('\n');
        }
        kept.push_str(line);
    }
    if kept.is_empty() {
        // One line longer than the budget: cut it, on a character boundary.
        kept = text.chars().take(max).collect();
    }
    format!("{} […]", kept.trim_end())
}

/// A task as its neighbours see it: title, status, and what it covers.
#[must_use]
pub fn sibling(task: &Issue) -> Sibling {
    let status = if task.is_closed() {
        "done"
    } else if task.has(labels::WAITING_MERGE) {
        "delivered, waiting for merge"
    } else {
        "open"
    };
    Sibling {
        number: task.number.to_string(),
        title: task.title.clone(),
        status: status.to_string(),
        gist: gist_of(&task.body),
    }
}

/// What sits above a milestone: its roadmap item and every milestone of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Above {
    /// The roadmap item, its boundaries only — see `ROADMAP_KEPT`.
    pub roadmap: Named,
    /// The roadmap's milestones, the one asked about included.
    pub milestones: Vec<Sibling>,
}

/// The open roadmap item that holds this milestone as a sub-issue.
///
/// The roadmap is context, not a requirement: none found is `None`, but a
/// failed read still propagates.
///
/// # Errors
///
/// A failed GitHub read.
pub async fn roadmap_of(gh: &dyn GitHub, milestone: u64) -> Outcome<Option<Named>> {
    Ok(above(gh, milestone).await?.map(|found| found.roadmap))
}

/// The open roadmap item that holds this milestone, and its milestones.
///
/// `None` when no open roadmap item holds it.
///
/// # Errors
///
/// A failed GitHub read.
pub async fn above(gh: &dyn GitHub, milestone: u64) -> Outcome<Option<Above>> {
    for item in gh.issues_labelled(labels::ROADMAP, "open").await? {
        let subs = gh.sub_issues(item.number).await?;
        if !subs.iter().any(|sub| sub.number == milestone) {
            continue;
        }
        return Ok(Some(Above {
            roadmap: Named {
                number: item.number.to_string(),
                title: item.title,
                // The boundaries, not the narrative — see `ROADMAP_KEPT`. Done
                // here, the one place the roadmap is read, so the loop and the
                // refinement cannot end up carrying two different versions of
                // it.
                body: sections::keeping(&item.body, &ROADMAP_KEPT),
            },
            milestones: subs.iter().map(sibling).collect(),
        }));
    }
    Ok(None)
}

/// The hierarchy around a task, found from the task alone: the open
/// milestone that holds it, its sibling tasks, and the roadmap above.
///
/// `None` when no open milestone holds the task.
///
/// # Errors
///
/// A failed GitHub read.
pub async fn around(gh: &dyn GitHub, task: &Issue) -> Outcome<Option<Scope>> {
    for milestone in gh.issues_labelled(labels::MILESTONE, "open").await? {
        let subs = gh.sub_issues(milestone.number).await?;
        if !subs.iter().any(|sub| sub.number == task.number) {
            continue;
        }
        return Ok(Some(Scope {
            roadmap: roadmap_of(gh, milestone.number).await?,
            siblings: subs.iter().map(sibling).collect(),
            milestone: Named {
                number: milestone.number.to_string(),
                title: milestone.title,
                body: milestone.body,
            },
            task: Named {
                number: task.number.to_string(),
                title: task.title.clone(),
                body: String::new(),
            },
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("item {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_task_is_placed_under_its_milestone_and_roadmap() {
        let task = issue(11, &[labels::AGENT]);
        let gh = FakeGitHub {
            issues: vec![issue(2, &[labels::ROADMAP]), issue(4, &[labels::MILESTONE])],
            subs: vec![
                (2, vec![issue(4, &[labels::MILESTONE])]),
                (4, vec![issue(10, &[]), task.clone()]),
            ],
            ..FakeGitHub::default()
        };
        let found = around(&gh, &task).await.expect("read").expect("a scope");
        assert_eq!(found.milestone.number, "4");
        assert_eq!(found.roadmap.expect("roadmap").number, "2");
        assert_eq!(found.siblings.len(), 2);
    }

    #[tokio::test]
    async fn a_milestone_is_placed_under_its_roadmap_among_its_neighbours() {
        let gh = FakeGitHub {
            issues: vec![issue(2, &[labels::ROADMAP])],
            subs: vec![(
                2,
                vec![
                    issue(4, &[labels::MILESTONE]),
                    issue(5, &[labels::MILESTONE]),
                ],
            )],
            ..FakeGitHub::default()
        };
        let found = above(&gh, 5).await.expect("read").expect("a roadmap");
        assert_eq!(found.roadmap.number, "2");
        assert_eq!(found.milestones.len(), 2);
        assert!(above(&gh, 9).await.expect("read").is_none());
    }

    #[test]
    fn a_neighbours_gist_is_its_scope_section_when_it_has_one() {
        let task = Issue {
            body: "## Scope\n\nbranch: feat/x\n\nCovers: the fence. Does not cover: the gate.\n\n\
                   ## Business Goal\n\nthe owner sleeps better\n\n\
                   ## Technical Implementation Plan\n\n1. touch every file\n"
                .to_string(),
            ..issue(11, &[])
        };
        let gist = sibling(&task).gist;
        assert!(gist.contains("Does not cover: the gate."));
        // Never a neighbour's internals: that is how a session starts doing
        // its neighbour's work.
        assert!(!gist.contains("touch every file"));
        assert!(!gist.contains("sleeps better"));
    }

    #[test]
    fn a_refined_neighbour_falls_back_to_its_business_goal() {
        // Tasks split before the Scope section existed have none, and their
        // shortest honest statement of purpose is the Business Goal.
        let task = Issue {
            body: "## Business Goal\n\nthe owner sleeps better\n\n\
                   ## Acceptance Criteria\n\n- a hundred bullets\n"
                .to_string(),
            ..issue(11, &[])
        };
        let gist = sibling(&task).gist;
        assert_eq!(gist, "the owner sleeps better");
    }

    #[test]
    fn a_neighbour_with_no_section_at_all_gives_its_raw_body() {
        let task = Issue {
            body: "a request someone typed by hand".to_string(),
            ..issue(11, &[])
        };
        assert_eq!(sibling(&task).gist, "a request someone typed by hand");
    }

    #[test]
    fn a_long_gist_is_cut_on_whole_lines_and_says_it_was_cut() {
        let body = format!("## Scope\n\n{}", "une ligne de contexte\n".repeat(200));
        let gist = sibling(&Issue {
            body,
            ..issue(11, &[])
        })
        .gist;
        assert!(
            gist.chars().count() <= GIST_MAX + 8,
            "{}",
            gist.chars().count()
        );
        assert!(gist.ends_with("[…]"));
        // Whole lines: no half sentence left reading as a complete one.
        assert!(gist.contains("une ligne de contexte"));
    }

    #[test]
    fn one_line_longer_than_the_budget_is_cut_without_panicking() {
        // Accented characters are not one byte: slicing a String by bytes
        // would panic here, and this repo's issues are written in French.
        let body = format!("## Scope\n\n{}", "é".repeat(GIST_MAX * 2));
        let gist = sibling(&Issue {
            body,
            ..issue(11, &[])
        })
        .gist;
        assert!(gist.chars().count() <= GIST_MAX + 8);
    }

    #[test]
    fn the_boundary_survives_the_cap_even_when_it_was_written_last() {
        // The regression this exists for: a refined `## Business Goal` ends on
        // its boundary bullet, so a cap applied in written order cut exactly
        // the sentence a neighbour's reader needs.
        let body = format!(
            "## Business Goal\n\n{}\n- **Boundary:** no statblock yet.\n",
            "du contexte que personne ne lit\n".repeat(30)
        );
        let gist = sibling(&Issue {
            body,
            ..issue(11, &[])
        })
        .gist;
        assert!(gist.contains("no statblock yet."), "{gist}");
        assert!(gist.chars().count() <= GIST_MAX + 8, "{gist}");
    }

    #[test]
    fn a_gist_with_no_boundary_keeps_its_written_order() {
        let body = "## Business Goal\n\nd'abord ceci\n- puis cela\n";
        let gist = sibling(&Issue {
            body: body.to_string(),
            ..issue(11, &[])
        })
        .gist;
        assert_eq!(gist, "d'abord ceci\n- puis cela");
    }

    #[tokio::test]
    async fn the_roadmap_carries_its_boundaries_and_not_its_narrative() {
        // 10 984 characters of roadmap #12 were in every prompt, about 8 000 of
        // them the product story. What keeps a session in its scope is the
        // boundary, not the story.
        let roadmap = Issue {
            body: "## La vision\n\nun long récit du produit\n\n\
                   ## Frontières\n\npas de génération de contenu\n\n\
                   ## Contraintes non négociables\n\nle coffre est canon\n"
                .to_string(),
            ..issue(2, &[labels::ROADMAP])
        };
        let gh = FakeGitHub {
            issues: vec![roadmap],
            subs: vec![(2, vec![issue(4, &[labels::MILESTONE])])],
            ..FakeGitHub::default()
        };
        let found = roadmap_of(&gh, 4).await.expect("read").expect("a roadmap");
        assert!(found.body.contains("pas de génération de contenu"));
        assert!(found.body.contains("le coffre est canon"));
        assert!(!found.body.contains("un long récit"), "{}", found.body);
    }

    #[tokio::test]
    async fn a_roadmap_under_unknown_headings_keeps_everything() {
        // Failing open: losing the whole roadmap is worse than carrying it.
        let roadmap = Issue {
            body: "## Notes\n\ntout ce qu'on sait\n".to_string(),
            ..issue(2, &[labels::ROADMAP])
        };
        let gh = FakeGitHub {
            issues: vec![roadmap],
            subs: vec![(2, vec![issue(4, &[labels::MILESTONE])])],
            ..FakeGitHub::default()
        };
        let found = roadmap_of(&gh, 4).await.expect("read").expect("a roadmap");
        assert!(found.body.contains("tout ce qu'on sait"));
    }

    #[tokio::test]
    async fn a_task_outside_any_milestone_has_no_hierarchy() {
        let gh = FakeGitHub::default();
        assert!(around(&gh, &issue(11, &[])).await.expect("read").is_none());
    }
}
