//! The milestone's board as the graph sees it, and the judgement.
//!
//! Pure business logic. Two readings of the board and one judgement:
//!
//! - [`resolve`]: what the harness settles **mechanically** — an id a
//!   sibling task already `produces` is chained on it, an id the
//!   repository already holds needs no producer; nothing is paid for.
//! - [`brief`]: the board as one line per task, what the session reads
//!   instead of twenty refined SPECs.
//! - [`judge`]: the mechanical resolution, the session's verdict and the
//!   question's own limits, folded into what is written — and what goes to
//!   a human, with the reason.

use harness_architecture::tree::Tree;
use harness_architecture::vocabulary::Declaration;
use harness_core::domain::Issue;

use crate::arbiter::data::verdict::Verdict;
use crate::common::decision::Question;
use crate::common::{labels, sections};

/// One task of the milestone, as the graph reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its number.
    pub number: u64,
    /// Its title.
    pub title: String,
    /// Still open.
    pub open: bool,
    /// Already started — opened to the loop, delivered, or done: its
    /// `produces` is no longer anyone's to change.
    pub started: bool,
    /// Its `## Architecture`, when it parses.
    pub declaration: Option<Declaration>,
}

impl Entry {
    /// Whether this task produces the id — case-insensitively, as `split`
    /// chains.
    #[must_use]
    pub fn produces(&self, id: &str) -> bool {
        self.declaration
            .as_ref()
            .is_some_and(|d| d.produces.iter().any(|p| same(p, id)))
    }
}

/// Two ids are one when they differ only by case or spacing.
fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// A task has started once a human opened the tap on it, or once it was
/// delivered or closed: what it produces is settled from then on.
#[must_use]
pub fn started(issue: &Issue) -> bool {
    issue.is_closed()
        || issue.has(labels::READY)
        || issue.has(labels::WAITING_MERGE)
        || issue.has(labels::REVIEW_PENDING)
}

/// The task's `## Architecture`, parsed.
#[must_use]
pub fn declaration_of(issue: &Issue) -> Option<Declaration> {
    sections::parse(&issue.body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
}

/// The board: every task of the milestone but the one in question.
#[must_use]
pub fn entries(task: u64, siblings: &[Issue]) -> Vec<Entry> {
    siblings
        .iter()
        .filter(|issue| issue.number != task)
        .map(|issue| Entry {
            number: issue.number,
            title: issue.title.clone(),
            open: issue.is_open(),
            started: started(issue),
            declaration: declaration_of(issue),
        })
        .collect()
}

/// The task of the board that produces this id, if one does.
#[must_use]
pub fn producer_of<'a>(entries: &'a [Entry], id: &str) -> Option<&'a Entry> {
    entries.iter().find(|entry| entry.produces(id))
}

/// What the harness settled without a session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolution {
    /// Each id a task of the board produces, with that task.
    pub chained: Vec<(String, u64)>,
    /// The ids the repository already holds: nothing to wait on.
    pub present: Vec<String>,
    /// The ids nobody produces: what the session is asked about.
    pub unresolved: Vec<String>,
}

/// Settles what needs no judgement: a producer on the board, or an id
/// already in the repository. An id both produced and present is chained —
/// the task that reshapes it is what the reader waits on.
#[must_use]
pub fn resolve(needs: &[String], entries: &[Entry], existing: &[String]) -> Resolution {
    let mut found = Resolution::default();
    for id in needs {
        if let Some(producer) = producer_of(entries, id) {
            found.chained.push((id.clone(), producer.number));
        } else if existing.iter().any(|known| same(known, id)) {
            found.present.push(id.clone());
        } else {
            found.unresolved.push(id.clone());
        }
    }
    found
}

/// The ids the repository holds, as its manifests name them: aggregates,
/// `DataCapabilities`, Capabilities, Micro-UIs, frozen screens.
#[must_use]
pub fn existing_ids(tree: &Tree) -> Vec<String> {
    let named = |manifest: &harness_architecture::tree::Manifest, key: &str| {
        manifest
            .json
            .as_ref()
            .and_then(|json| json.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    let mut ids: Vec<String> = Vec::new();
    ids.extend(tree.aggregates.iter().filter_map(|m| named(m, "aggregate")));
    ids.extend(
        tree.data_capabilities
            .iter()
            .filter_map(|m| named(m, "dataCapability")),
    );
    ids.extend(
        tree.capabilities
            .iter()
            .filter_map(|c| c.manifest.as_ref())
            .filter_map(|m| named(m, "capability")),
    );
    ids.extend(
        tree.micro_uis
            .iter()
            .filter_map(|(_, m)| named(m, "microUi")),
    );
    ids.extend(tree.screens.iter().cloned());
    ids.sort();
    ids.dedup();
    ids
}

/// The board, one line per task: what the session reads.
#[must_use]
pub fn brief(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "- (no other task in the milestone)".to_string();
    }
    entries
        .iter()
        .map(|entry| {
            let status = if !entry.open {
                "done"
            } else if entry.started {
                "started"
            } else {
                "not started"
            };
            let what = entry.declaration.as_ref().map_or_else(
                || "(no architecture section)".to_string(),
                |d| {
                    format!(
                        "{}, produces: {}, needs: {}",
                        d.unit.key(),
                        list(&d.produces),
                        list(&d.needs)
                    )
                },
            );
            format!("- #{} {} — {what} — {status}", entry.number, entry.title)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn list(ids: &[String]) -> String {
    if ids.is_empty() {
        "-".to_string()
    } else {
        ids.join(", ")
    }
}

/// One id chained on the task that produces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The id.
    pub id: String,
    /// The task that produces it.
    pub producer: u64,
    /// The producer did not list the id yet: its `produces` is widened.
    pub reassigned: bool,
}

/// What is written, and what is handed to a human.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Judgement {
    /// The chains to draw: the task's `needs`, its `blocked_by`, a
    /// producer's `produces` where reassigned.
    pub links: Vec<Link>,
    /// The ids the repository holds: `needs` only.
    pub present: Vec<String>,
    /// Who settled the question: `harness` when no session was needed,
    /// `ai` when the verdict was applied — `None` when a human has it.
    pub by: Option<&'static str>,
    /// Why a human has it, when they do.
    pub human: Option<String>,
}

impl Judgement {
    /// One line for the `## Decision` section and the comment.
    #[must_use]
    pub fn verdict_line(&self) -> String {
        if let Some(why) = &self.human {
            return format!("for a human — {why}");
        }
        let chains: Vec<String> = self
            .links
            .iter()
            .map(|link| {
                if link.reassigned {
                    format!("{} ← #{} (now produces it)", link.id, link.producer)
                } else {
                    format!("{} ← #{}", link.id, link.producer)
                }
            })
            .chain(
                self.present
                    .iter()
                    .map(|id| format!("{id} (already there)")),
            )
            .collect();
        if chains.is_empty() {
            "nothing to chain".to_string()
        } else {
            chains.join("; ")
        }
    }
}

/// Folds the question, the mechanical resolution and the verdict into one
/// judgement.
///
/// The mechanical chains are always kept — a producer on the board is a
/// fact, not an opinion. The verdict's are kept only whole: a verdict
/// applied in part would leave the graph half-settled under a
/// `decided-by: ai` nobody could read.
#[must_use]
pub fn judge(
    question: &Question,
    resolution: &Resolution,
    entries: &[Entry],
    verdict: Option<&Verdict>,
) -> Judgement {
    let mut judged = Judgement {
        links: resolution
            .chained
            .iter()
            .map(|(id, producer)| Link {
                id: id.clone(),
                producer: *producer,
                reassigned: false,
            })
            .collect(),
        present: resolution.present.clone(),
        by: None,
        human: None,
    };
    if question.raised_again() {
        judged.human = Some(
            "raised a second time after an arbitration — no second agent disagrees with the first"
                .to_string(),
        );
        return judged;
    }
    if question.changes_the_plan() {
        judged.human = Some(
            "a `produces` to change or a task to cut is a decision on the plan, not on the graph"
                .to_string(),
        );
        return judged;
    }
    if resolution.unresolved.is_empty() {
        judged.by = Some("harness");
        return judged;
    }
    let Some(verdict) = verdict else {
        judged.human = Some(format!(
            "nobody produces {} and no verdict was read",
            resolution.unresolved.join(", ")
        ));
        return judged;
    };
    if !verdict.settles() {
        judged.human = Some(format!(
            "the arbiter did not settle it ({}, confidence {}/5): {}",
            verdict.decision.trim(),
            verdict.confidence,
            verdict.reason.trim()
        ));
        return judged;
    }
    let mut named: Vec<Link> = Vec::new();
    for id in &resolution.unresolved {
        let Some(chain) = verdict.chain_for(id) else {
            judged.human = Some(format!("the arbiter left {id} without a producer"));
            return judged;
        };
        let Some(entry) = entries.iter().find(|e| e.number == chain.producer) else {
            judged.human = Some(format!(
                "the arbiter named #{} for {id}, which is not a task of this milestone",
                chain.producer
            ));
            return judged;
        };
        if !entry.open {
            judged.human = Some(format!(
                "the arbiter named #{} for {id}, which is closed",
                entry.number
            ));
            return judged;
        }
        let reassigned = !entry.produces(id);
        if reassigned && entry.started {
            judged.human = Some(format!(
                "the arbiter named #{} for {id}, which has started — its produces is settled",
                entry.number
            ));
            return judged;
        }
        named.push(Link {
            id: id.clone(),
            producer: entry.number,
            reassigned,
        });
    }
    judged.links.extend(named);
    judged.by = Some("ai");
    judged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arbiter::data::verdict;
    use harness_architecture::tree::Manifest;

    fn issue(number: u64, labels: &[&str], architecture: &str) -> Issue {
        Issue {
            number,
            title: format!("task {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: format!("## Scope\n\nbranch: x\n\n## Architecture\n\n{architecture}\n"),
            ..Issue::default()
        }
    }

    fn board() -> Vec<Entry> {
        let mut done = issue(
            10,
            &[],
            "unit: aggregate\nsystem: campagne\nproduces: Campagne",
        );
        done.state = "closed".to_string();
        entries(
            20,
            &[
                done,
                issue(
                    12,
                    &[labels::READY],
                    "unit: aggregate\nsystem: campagne\nproduces: PJ\nneeds: Campagne",
                ),
                issue(
                    14,
                    &[],
                    "unit: data-capability\nsystem: campagne\nproduces: campagne.ajouterPj\nneeds: PJ",
                ),
                issue(
                    20,
                    &[],
                    "unit: micro-ui\nsystem: campagne\nproduces: Pjs\nneeds: PJ",
                ),
                issue(21, &[], "no architecture here"),
            ],
        )
    }

    fn question(needs: &[&str]) -> Question {
        Question {
            blocks: "x".to_string(),
            needs: needs.iter().map(|s| (*s).to_string()).collect(),
            round: 1,
            ..Question::default()
        }
    }

    #[test]
    fn the_board_leaves_the_task_out_and_reads_each_sibling_s_status() {
        let found = board();
        assert_eq!(
            found.iter().map(|e| e.number).collect::<Vec<_>>(),
            [10, 12, 14, 21]
        );
        assert!(!found[0].open && found[0].started, "closed is started");
        assert!(found[1].open && found[1].started, "ready is started");
        assert!(found[2].open && !found[2].started);
        assert!(found[3].declaration.is_none());
        assert!(found[1].produces("pj"), "case-insensitive");
        assert_eq!(producer_of(&found, "Campagne").map(|e| e.number), Some(10));
        assert_eq!(producer_of(&found, "Risk"), None);
    }

    #[test]
    fn a_task_has_started_once_opened_delivered_reviewed_or_closed() {
        for label in [labels::READY, labels::WAITING_MERGE, labels::REVIEW_PENDING] {
            assert!(started(&issue(1, &[label], "")), "{label}");
        }
        assert!(!started(&issue(1, &[labels::AGENT], "")));
        let mut closed = issue(1, &[], "");
        closed.state = "closed".to_string();
        assert!(started(&closed));
    }

    #[test]
    fn the_mechanical_resolution_chains_present_and_leaves_the_rest() {
        let found = resolve(
            &["PJ".to_string(), "Risk@3".to_string(), "Dice".to_string()],
            &board(),
            &["Risk@3".to_string()],
        );
        assert_eq!(found.chained, [("PJ".to_string(), 12)]);
        assert_eq!(found.present, ["Risk@3"]);
        assert_eq!(found.unresolved, ["Dice"]);
    }

    #[test]
    fn the_brief_is_one_line_per_task_with_its_graph_and_status() {
        let text = brief(&board());
        assert!(text.contains("- #10 task 10 — aggregate, produces: Campagne, needs: - — done"));
        assert!(
            text.contains("- #12 task 12 — aggregate, produces: PJ, needs: Campagne — started")
        );
        assert!(text.contains(
            "- #14 task 14 — data-capability, produces: campagne.ajouterPj, needs: PJ — not started"
        ));
        assert!(text.contains("- #21 task 21 — (no architecture section) — not started"));
        assert_eq!(brief(&[]), "- (no other task in the milestone)");
    }

    #[test]
    fn the_repository_s_ids_come_from_every_manifest_kind() {
        let manifest = |path: &str, json: &str| Manifest {
            path: path.to_string(),
            json: serde_json::from_str(json).ok(),
        };
        let tree = Tree {
            aggregates: vec![manifest("a", r#"{"aggregate":"PJ"}"#)],
            data_capabilities: vec![manifest("d", r#"{"dataCapability":"campagne.ajouterPj"}"#)],
            capabilities: vec![harness_architecture::tree::CapabilityCrate {
                dir: "crates/campagne/capabilities/liste".to_string(),
                system: "campagne".to_string(),
                manifest: Some(manifest("c", r#"{"capability":"campagne.listePjs"}"#)),
                cargo_toml: None,
            }],
            micro_uis: vec![(
                "campagne".to_string(),
                manifest("m", r#"{"microUi":"Pjs"}"#),
            )],
            screens: vec!["Campagne".to_string(), "Pjs".to_string()],
            ..Tree::default()
        };
        assert_eq!(
            existing_ids(&tree),
            [
                "Campagne",
                "PJ",
                "Pjs",
                "campagne.ajouterPj",
                "campagne.listePjs"
            ]
        );
    }

    #[test]
    fn a_question_the_harness_settles_alone_needs_no_verdict() {
        let resolution = resolve(&["PJ".to_string()], &board(), &[]);
        let judged = judge(&question(&["PJ"]), &resolution, &board(), None);
        assert_eq!(judged.by, Some("harness"));
        assert!(judged.human.is_none());
        assert_eq!(judged.links.len(), 1);
        assert_eq!(judged.verdict_line(), "PJ ← #12");
    }

    #[test]
    fn a_confident_verdict_chains_and_reassigns_on_a_task_not_started() {
        let resolution = resolve(&["Dice".to_string()], &board(), &[]);
        let verdict = verdict::parse(
            r#"{"decision":"resolve","confidence":4,"needs":[{"id":"dice","producer":14}],"reason":"same aggregate"}"#,
        )
        .expect("parse");
        let judged = judge(&question(&["Dice"]), &resolution, &board(), Some(&verdict));
        assert_eq!(judged.by, Some("ai"));
        assert_eq!(
            judged.links,
            [Link {
                id: "Dice".to_string(),
                producer: 14,
                reassigned: true
            }]
        );
        assert_eq!(judged.verdict_line(), "Dice ← #14 (now produces it)");
    }

    #[test]
    fn what_the_arbiter_may_not_decide_goes_to_a_human_with_the_mechanical_chains_kept() {
        let resolution = resolve(&["PJ".to_string(), "Dice".to_string()], &board(), &[]);
        let human = |verdict: &str| {
            let parsed = verdict::parse(verdict).expect("parse");
            judge(
                &question(&["PJ", "Dice"]),
                &resolution,
                &board(),
                Some(&parsed),
            )
        };
        // A started task's produces is settled.
        let started =
            human(r#"{"decision":"resolve","confidence":5,"needs":[{"id":"Dice","producer":12}]}"#);
        assert!(
            started
                .human
                .as_deref()
                .is_some_and(|w| w.contains("#12") && w.contains("started"))
        );
        assert_eq!(started.by, None);
        assert_eq!(started.links.len(), 1, "PJ ← #12 is kept");
        // A closed task.
        let closed =
            human(r#"{"decision":"resolve","confidence":5,"needs":[{"id":"Dice","producer":10}]}"#);
        assert!(
            closed
                .human
                .as_deref()
                .is_some_and(|w| w.contains("closed"))
        );
        // A task outside the milestone.
        let outside =
            human(r#"{"decision":"resolve","confidence":5,"needs":[{"id":"Dice","producer":99}]}"#);
        assert!(outside.human.as_deref().is_some_and(|w| w.contains("#99")));
        // An id left out.
        let left = human(r#"{"decision":"resolve","confidence":5,"needs":[]}"#);
        assert!(left.human.as_deref().is_some_and(|w| w.contains("Dice")));
        // A hesitant verdict carries its reason.
        let hesitant = human(
            r#"{"decision":"resolve","confidence":2,"needs":[{"id":"Dice","producer":14}],"reason":"two candidates"}"#,
        );
        assert!(
            hesitant
                .human
                .as_deref()
                .is_some_and(|w| w.contains("2/5") && w.contains("two candidates"))
        );
        assert!(hesitant.verdict_line().starts_with("for a human — "));
        // No verdict at all.
        let none = judge(&question(&["PJ", "Dice"]), &resolution, &board(), None);
        assert!(
            none.human
                .as_deref()
                .is_some_and(|w| w.contains("no verdict"))
        );
    }

    #[test]
    fn a_second_round_or_a_plan_change_goes_to_a_human_before_any_verdict() {
        let resolution = Resolution::default();
        let again = Question {
            round: 2,
            ..question(&[])
        };
        assert!(
            judge(&again, &resolution, &[], None)
                .human
                .as_deref()
                .is_some_and(|w| w.contains("second"))
        );
        let cut = Question {
            cut: Some("two halves".to_string()),
            ..question(&[])
        };
        assert!(
            judge(&cut, &resolution, &[], None)
                .human
                .as_deref()
                .is_some_and(|w| w.contains("plan"))
        );
        let nothing = judge(&question(&[]), &resolution, &[], None);
        assert_eq!(nothing.by, Some("harness"));
        assert_eq!(nothing.verdict_line(), "nothing to chain");
    }
}
