//! A task the plan lacks, as a session's stop names it — read here, with no
//! I/O, for the stop handler to open it under the milestone.
//!
//! `ForgeDesMondes`, 2026-10-22: #191 could not be finished without a
//! `ProjectPg` on the `BattlemapInstance` aggregate, which no task of
//! milestone #25 built. The session opened #405 for a human, who had
//! nothing to decide: the session had said what to build, in which unit,
//! and why. The stop now says it in split's own words — one slice, as
//! `split` writes it ([`TaskItem`]), on the stop line:
//!
//! `AGENT_LOOP_STOP: task {"title": …, "branch": "feat/…", "unit": …, …}`
//!
//! and the harness opens it with split's renderer, deterministically: no
//! session is paid to write a SPEC the stop already carries. A slice that is
//! not whole by split's own measure ([`plan::spec_gaps`]) — or that names
//! no unit, no system, no id, no branch — is [`Asked::Unwritable`], and the
//! stop handler falls back to a person.

use harness_architecture::vocabulary::Unit;
use harness_core::domain::Issue;

use crate::common::json_reply;
use crate::common::{labels, sections};
use crate::split::data::plan::{self, TaskItem};

/// What a stop that names a missing task asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// A slice whole enough to be opened as it is.
    Task(Box<TaskItem>),
    /// A missing task the harness cannot write a SPEC for safely.
    Unwritable {
        /// What the stop said after `task`, for a person to read.
        said: String,
        /// Why no SPEC can be written from it.
        why: String,
    },
}

/// The missing task a stop names: `AGENT_LOOP_STOP: task <slice JSON>`.
/// `None` when the stop asks something else.
#[must_use]
pub fn asked(reason: &str) -> Option<Asked> {
    let (_, after) = reason.split_once("AGENT_LOOP_STOP")?;
    let said = after
        .trim_start_matches(|c: char| c == ':' || c == '`' || c == '*' || c.is_whitespace())
        .strip_prefix("task ")?
        .trim();
    if said.is_empty() {
        return None;
    }
    let unwritable = |why: String| Asked::Unwritable {
        said: said.to_string(),
        why,
    };
    let mut item: TaskItem = match json_reply::parse_object(said) {
        Ok(item) => item,
        Err(error) => {
            return Some(unwritable(format!(
                "the stop carries no slice that reads ({error})"
            )));
        }
    };
    // A slice index points into split's array; a lone slice has none.
    item.depends_on.clear();
    let gaps = gaps(&item);
    Some(if gaps.is_empty() {
        Asked::Task(Box::new(item))
    } else {
        unwritable(gaps.join("; "))
    })
}

/// What keeps a slice from being opened as it is: split's own measure, and
/// what a task opened outside a split must carry on its own.
fn gaps(item: &TaskItem) -> Vec<String> {
    let mut gaps = Vec::new();
    if item.needs_human {
        gaps.push("the session says only a person can do it".to_string());
        return gaps;
    }
    if item.title.trim().is_empty() {
        gaps.push("it has no `title`".to_string());
    }
    let branch = item.branch.trim();
    if branch.is_empty() || !branch.contains('/') || branch.contains(char::is_whitespace) {
        gaps.push(format!(
            "its `branch` ({branch:?}) is no `<type>/<slug>` branch"
        ));
    }
    if Unit::parse(&item.unit).is_none() {
        gaps.push(format!(
            "its `unit` ({:?}) is no unit of the architecture",
            item.unit
        ));
    }
    if item.system.trim().is_empty() {
        gaps.push("it names no `system`".to_string());
    }
    if id(item).is_none() {
        gaps.push("it names nothing it `produces`".to_string());
    }
    gaps.extend(plan::spec_gaps(std::slice::from_ref(item)));
    gaps
}

/// The id a missing task is known by: the first it produces.
#[must_use]
pub fn id(item: &TaskItem) -> Option<&str> {
    item.produces
        .iter()
        .map(|id| id.trim())
        .find(|id| !id.is_empty() && *id != "-")
}

/// The `## Architecture` section with the task's `id:` under its `unit:` line.
///
/// As the contract manager writes it.
#[must_use]
pub fn with_id(architecture: &str, id: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut written = false;
    for line in architecture.lines() {
        lines.push(line.to_string());
        if !written && line.trim_start().starts_with("unit:") {
            lines.push(format!("id: {id}"));
            written = true;
        }
    }
    if !written {
        lines.insert(0, format!("id: {id}"));
    }
    lines.join("\n")
}

/// Whether `issue`, a sub-issue of the milestone, already is the missing task.
///
/// An open agent task under the same title, or one known by the same id. A
/// person's task of the same name is not reused — opening the agent task is
/// the point.
#[must_use]
pub fn is_the_same(issue: &Issue, item: &TaskItem) -> bool {
    if !issue.is_open() || !issue.has(labels::AGENT) {
        return false;
    }
    if folded(&issue.title) == folded(&item.title) {
        return true;
    }
    let Some(id) = id(item) else {
        return false;
    };
    sections::parse(&issue.body)
        .get(sections::ARCHITECTURE)
        .is_some_and(|architecture| {
            architecture.lines().any(|line| {
                let line = line.trim();
                line.strip_prefix("id:")
                    .is_some_and(|named| named.trim().trim_matches('`') == id)
                    || line.strip_prefix("produces:").is_some_and(|named| {
                        named
                            .split(',')
                            .any(|named| named.trim().trim_matches('`') == id)
                    })
            })
        })
}

/// A title as compared: case and spacing aside.
fn folded(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The stop #191's session would have written for #405.
#[cfg(test)]
pub(crate) const BATTLEMAP: &str = r#"AGENT_LOOP_STOP: task {"title": "BattlemapInstance ProjectPg: project the aggregate's rows into battlemap_instance, token, combat and initiative_entry", "branch": "feat/table-battlemap-instance-projection", "brief": "Give the BattlemapInstance aggregate its ProjectPg, so the state its commands write fills battlemap_instance, token, combat and initiative_entry (pattern: crates/media/aggregates/Media/src/tables.rs).", "unit": "aggregate", "system": "table", "aggregate": "BattlemapInstance", "produces": ["BattlemapInstance"], "goal": "The table's read model is filled by the aggregate's own commands, so table.getBattlemapInstance answers the board the DM built.", "acceptance": ["given a token placed by a BattlemapInstance command, when the projection runs, then token holds its row", "given a combat started, when the projection runs, then combat and initiative_entry hold it"]} (/code)"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> TaskItem {
        match asked(BATTLEMAP) {
            Some(Asked::Task(item)) => *item,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_stop_names_a_whole_slice() {
        let item = item();
        assert_eq!(item.unit, "aggregate");
        assert_eq!(item.branch, "feat/table-battlemap-instance-projection");
        assert_eq!(id(&item), Some("BattlemapInstance"));
        assert_eq!(item.acceptance.len(), 2);
        let marked = BATTLEMAP.replacen("AGENT_LOOP_STOP: ", "**AGENT_LOOP_STOP:** `", 1);
        assert!(matches!(asked(&marked), Some(Asked::Task(_))), "{marked}");
    }

    #[test]
    fn a_stop_on_anything_else_names_no_task() {
        assert_eq!(asked("AGENT_LOOP_STOP: contract a.b — x"), None);
        assert_eq!(asked("AGENT_LOOP_STOP: which provider?"), None);
        assert_eq!(asked("AGENT_LOOP_STOP: task "), None);
        assert_eq!(asked("task {}"), None, "no stop");
    }

    #[test]
    fn a_slice_that_is_not_whole_is_unwritable_and_says_why() {
        let why = |reason: &str| match asked(reason) {
            Some(Asked::Unwritable { why, said }) => {
                assert_ne!(said, "");
                why
            }
            other => panic!("{other:?}"),
        };
        assert!(why("AGENT_LOOP_STOP: task the projection of X").contains("no slice that reads"));
        let without = |field: &str| {
            let mut value: serde_json::Value = serde_json::from_str(
                &BATTLEMAP[BATTLEMAP.find('{').unwrap_or(0)..=BATTLEMAP.rfind('}').unwrap_or(0)],
            )
            .expect("the fixture reads");
            value[field] = match field {
                "acceptance" | "produces" => serde_json::json!([]),
                _ => serde_json::json!(""),
            };
            format!("AGENT_LOOP_STOP: task {value}")
        };
        assert!(why(&without("goal")).contains("no `goal`"));
        assert!(why(&without("acceptance")).contains("no `acceptance`"));
        assert!(why(&without("produces")).contains("`produces`"));
        assert!(why(&without("system")).contains("no `system`"));
        assert!(why(&without("unit")).contains("no unit of the architecture"));
        assert!(why(&without("title")).contains("no `title`"));
        assert!(why(&without("branch")).contains("no `<type>/<slug>` branch"));
        let spaced = BATTLEMAP.replace(
            "feat/table-battlemap-instance-projection",
            "feat/table battlemap",
        );
        assert!(why(&spaced).contains("branch"));
        let bare = BATTLEMAP.replace("feat/table-battlemap-instance-projection", "projection");
        assert!(why(&bare).contains("branch"));
        let human = BATTLEMAP.replace(r#""unit""#, r#""needs_human": true, "unit""#);
        assert_eq!(why(&human), "the session says only a person can do it");
    }

    #[test]
    fn split_s_slice_index_means_nothing_alone() {
        let indexed = BATTLEMAP.replace(r#""unit""#, r#""depends_on": [3], "unit""#);
        let Some(Asked::Task(item)) = asked(&indexed) else {
            panic!("a task");
        };
        assert_eq!(item.depends_on, Vec::<usize>::new());
    }

    #[test]
    fn the_id_is_written_under_the_unit() {
        assert_eq!(
            with_id("unit: aggregate\nsystem: table", "BattlemapInstance"),
            "unit: aggregate\nid: BattlemapInstance\nsystem: table"
        );
        assert_eq!(
            with_id("unit: a\nunit: b", "X"),
            "unit: a\nid: X\nunit: b",
            "once"
        );
        assert_eq!(with_id("system: t", "X"), "id: X\nsystem: t");
        let mut item = item();
        item.produces = vec![" - ".to_string(), "Y".to_string()];
        assert_eq!(id(&item), Some("Y"));
    }

    #[test]
    fn the_same_task_is_an_open_agent_sibling_by_title_or_by_id() {
        let item = item();
        let sibling = |title: &str, body: &str, labels: &[&str], state: &str| Issue {
            number: 9,
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.iter().map(ToString::to_string).collect(),
            state: state.to_string(),
            ..Issue::default()
        };
        let title = item.title.to_uppercase().replace(' ', "  ");
        assert!(is_the_same(
            &sibling(&title, "", &[labels::AGENT], "open"),
            &item
        ));
        assert!(
            !is_the_same(&sibling(&title, "", &[labels::HUMAN], "open"), &item),
            "#405, a person's task, is not reused"
        );
        assert!(
            !is_the_same(&sibling(&title, "", &[labels::AGENT], "closed"), &item),
            "a closed one is done"
        );
        let by_id = "## Architecture\n\nunit: aggregate\nid: `BattlemapInstance`\n";
        assert!(is_the_same(
            &sibling("other", by_id, &[labels::AGENT], "open"),
            &item
        ));
        let by_produces =
            "## Architecture\n\nunit: aggregate\nproduces: Token, BattlemapInstance\n";
        assert!(is_the_same(
            &sibling("other", by_produces, &[labels::AGENT], "open"),
            &item
        ));
        let elsewhere = "## Architecture\n\nunit: aggregate\nproduces: Token\nid: Token\n\n## Scope\n\nid: BattlemapInstance";
        assert!(!is_the_same(
            &sibling("other", elsewhere, &[labels::AGENT], "open"),
            &item
        ));
        let mut nameless = item;
        nameless.produces.clear();
        assert!(!is_the_same(
            &sibling("other", by_id, &[labels::AGENT], "open"),
            &nameless
        ));
    }
}
