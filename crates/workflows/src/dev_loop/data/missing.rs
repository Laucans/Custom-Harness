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
//! no unit the architecture has, no system, no id, no branch — is
//! [`Asked::Refused`]: the stop handler says why on the task, and the next
//! session re-declares it. A unit a well-known spelling names
//! (`server-fixtures`, [`aliased`]) is read as that unit. A slice only a
//! person can do is [`Asked::Unwritable`], and goes to a person.

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
    /// A slice declared wrong — a unit the architecture does not name, a
    /// field missing, JSON that does not read: refused back to the session,
    /// which re-declares it. Never dropped in silence (`ForgeDesMondes`'
    /// #219, 2026-10-25: a `server-fixtures` slice was lost and the task
    /// stopped four more times on it).
    Refused {
        /// The slice's title, or the start of what the stop said.
        title: String,
        /// What the stop said for it, whole.
        said: String,
        /// What is wrong with it, in words the session acts on.
        why: String,
    },
    /// A missing task only a person can do (`needs_human`).
    Unwritable {
        /// What the stop said after `task`, for a person to read.
        said: String,
        /// Why no SPEC can be written from it.
        why: String,
    },
}

/// The most of a stop's words a refused slice's title keeps, in characters.
const TITLE_MAX: usize = 80;

/// Every unit key a slice may declare, as [`Unit::key`] spells them.
#[must_use]
pub fn unit_keys() -> String {
    Unit::ALL
        .iter()
        .map(|unit| unit.key())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The unit a well-known spelling outside the vocabulary means, if any.
///
/// Only one that means exactly one unit: fixtures or seed data the server
/// loads, and the plumbing, are `infrastructure` — they write the store,
/// the write side.
#[must_use]
pub fn aliased(unit: &str) -> Option<Unit> {
    let folded: String = unit
        .trim()
        .to_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    [
        "plumbing",
        "fixture",
        "fixtures",
        "serverfixture",
        "serverfixtures",
        "seed",
        "seeds",
        "seeddata",
        "serverseed",
        "serverseeds",
        "datafixtures",
    ]
    .contains(&folded.as_str())
    .then_some(Unit::Infrastructure)
}

/// A declared slice's title, from what the stop said for it — its `title`
/// when it reads as one object, else its start — on one line.
#[must_use]
pub fn title_of(said: &str) -> String {
    let title = serde_json::from_str::<serde_json::Value>(said)
        .ok()
        .and_then(|value| {
            value
                .get("title")
                .and_then(|t| t.as_str())
                .map(ToString::to_string)
        })
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| said.to_string());
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(TITLE_MAX)
        .collect()
}

/// A slice refused, its title read from what the stop said.
fn refused(said: &str, why: String) -> Asked {
    Asked::Refused {
        title: title_of(said),
        said: said.to_string(),
        why,
    }
}

/// The most slices one stop may name: a guard against a session that would
/// re-plan the milestone in a stop, not a target.
pub const MAX_SLICES: usize = 8;

/// What follows `AGENT_LOOP_STOP`, its markdown and colon trimmed.
fn after_stop(reason: &str) -> Option<&str> {
    let (_, after) = reason.split_once("AGENT_LOOP_STOP")?;
    Some(after.trim_start_matches(|c: char| c == ':' || c == '`' || c == '*' || c.is_whitespace()))
}

/// The missing task a stop names: `AGENT_LOOP_STOP: task <slice JSON>`.
/// `None` when the stop asks something else.
#[must_use]
pub fn asked(reason: &str) -> Option<Asked> {
    let said = after_stop(reason)?.strip_prefix("task ")?.trim();
    if said.is_empty() {
        return None;
    }
    Some(match json_reply::parse_object::<TaskItem>(said) {
        Ok(item) => judged(item, said),
        Err(error) => refused(
            said,
            format!("the stop carries no slice that reads ({error})"),
        ),
    })
}

/// Every missing task a stop names.
///
/// `task <slice>` as a list of one, or
/// `tasks [<slice>, …]` — each one whole or unwritable, the same title
/// (case and spacing aside) named once. `None` when the stop asks
/// something else.
///
/// `ForgeDesMondes`' #486 found four missing units over four rounds, one a
/// stop ($4.88, each session re-reading the context): one stop now names
/// them all. More than [`MAX_SLICES`] is one unwritable answer.
#[must_use]
pub fn asked_all(reason: &str) -> Option<Vec<Asked>> {
    let rest = after_stop(reason)?;
    let Some(list) = rest.strip_prefix("tasks ") else {
        return asked(reason).map(|one| vec![one]);
    };
    let said = list.trim();
    if said.is_empty() {
        return None;
    }
    let slices: Vec<serde_json::Value> = match json_reply::parse_array(said) {
        Ok(slices) => slices,
        Err(error) => {
            return Some(vec![refused(
                said,
                format!("the stop carries no list of slices that reads ({error})"),
            )]);
        }
    };
    if slices.len() > MAX_SLICES {
        return Some(vec![refused(
            said,
            format!(
                "too many slices in one stop ({} — at most {MAX_SLICES})",
                slices.len()
            ),
        )]);
    }
    let mut titles: Vec<String> = Vec::new();
    let mut found = Vec::new();
    for slice in slices {
        let text = slice.to_string();
        let asked = match serde_json::from_value::<TaskItem>(slice) {
            Ok(item) => {
                let title = folded(&item.title);
                if !title.is_empty() && titles.contains(&title) {
                    continue;
                }
                titles.push(title);
                judged(item, &text)
            }
            Err(error) => refused(&text, format!("a slice that does not read ({error})")),
        };
        found.push(asked);
    }
    Some(found)
}

/// A slice read, judged whole, refused, or a person's.
///
/// A unit outside the vocabulary that a well-known spelling names
/// ([`aliased`]) is read as that unit, and the opened task says so.
#[must_use]
pub fn judged(mut item: TaskItem, said: &str) -> Asked {
    // A slice index points into split's array; a stop's slices have none.
    item.depends_on.clear();
    if Unit::parse(&item.unit).is_none()
        && let Some(unit) = aliased(&item.unit)
    {
        item.brief = format!(
            "{}\n\nThe stop declared its unit as `{}`, which the architecture does not name: \
             the harness reads it as `{}` — fixtures and seed data the server loads write the \
             store, so this task is on the write side.",
            item.brief.trim(),
            item.unit.trim(),
            unit.key()
        );
        item.unit = unit.key().to_string();
    }
    let gaps = gaps(&item);
    if gaps.is_empty() {
        Asked::Task(Box::new(item))
    } else if item.needs_human {
        Asked::Unwritable {
            said: said.to_string(),
            why: gaps.join("; "),
        }
    } else {
        Asked::Refused {
            title: title_of(if item.title.trim().is_empty() {
                said
            } else {
                &item.title
            }),
            said: said.to_string(),
            why: gaps.join("; "),
        }
    }
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
    let unit = Unit::parse(&item.unit);
    if unit.is_none() {
        gaps.push(format!(
            "its `unit` ({:?}) is no unit of the architecture — one of: {}",
            item.unit,
            unit_keys()
        ));
    }
    if item.system.trim().is_empty() {
        gaps.push("it names no `system`".to_string());
    }
    // Plumbing and tooling ship no manifest id: their branch names them.
    let named_by_branch = matches!(unit, Some(Unit::Infrastructure | Unit::Tooling));
    if id(item).is_none() && !named_by_branch {
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
/// An open agent task under the same title; or one known by the same id
/// **that has not started** — no PR under review, not delivered. Two
/// changes to one unit asked under different titles once the first is in
/// review are two tasks, chained by their own blockers (`ForgeDesMondes`'
/// #448 was parked on #473, a different change to the same id). A
/// person's task of the same name is not reused — opening the agent task
/// is the point.
#[must_use]
pub fn is_the_same(issue: &Issue, item: &TaskItem) -> bool {
    if !issue.is_open() || !issue.has(labels::AGENT) {
        return false;
    }
    if folded(&issue.title) == folded(&item.title) {
        return true;
    }
    // One branch is one task: a slice re-declared under another title.
    let branch = item.branch.trim();
    if !branch.is_empty()
        && crate::dev_loop::data::tasks::declared_branch(&issue.body).as_deref() == Some(branch)
    {
        return true;
    }
    let started = issue.has(labels::REVIEW_PENDING) || issue.has(labels::WAITING_MERGE);
    let Some(id) = id(item).filter(|_| !started) else {
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

/// What a stop declared, one key per slice, sorted, each once.
///
/// The key is its branch, or a bounded form of its words when it has none
/// that reads ([`key_of`]): two stops declaring the same keys declare the
/// same tasks, and a decision already taken on them is no new fact.
#[must_use]
pub fn declared(asked: &[Asked]) -> Vec<String> {
    let mut keys: Vec<String> = asked
        .iter()
        .map(|one| match one {
            Asked::Task(item) => branch_key(&item.branch).unwrap_or_else(|| key_of(&item.title)),
            Asked::Refused { said, .. } | Asked::Unwritable { said, .. } => branch_said(said)
                .as_deref()
                .and_then(branch_key)
                .unwrap_or_else(|| key_of(said)),
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// The most of a slice's words its key keeps, in characters.
const KEY_WORDS_MAX: usize = 40;

/// A branch as a key: one `git` spells, so it survives the comma-separated
/// `declared:` line — else `None`.
fn branch_key(branch: &str) -> Option<String> {
    let branch = branch.trim();
    let spelled = !branch.is_empty()
        && branch.len() <= 100
        && branch.contains('/')
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'));
    spelled.then(|| branch.to_string())
}

/// The key of a slice with no branch that reads: a slug of its title, then
/// a hash of its words (case and spacing aside) — comma-free and bounded,
/// whatever the stop said.
#[must_use]
pub fn key_of(said: &str) -> String {
    let mut slug = String::new();
    for c in title_of(said).to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= KEY_WORDS_MAX {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    let hash = format!("{:08x}", fnv1a(&folded(said)) & 0xffff_ffff);
    if slug.is_empty() {
        format!("slice-{hash}")
    } else {
        format!("{slug}-{hash}")
    }
}

/// FNV-1a, 64 bits: a hash that never changes with the toolchain, for a
/// key written on an issue and read back by another process.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The branch a slice's words declare, when they read as one object.
fn branch_said(said: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(said)
        .ok()?
        .get("branch")?
        .as_str()
        .map(|branch| branch.trim().to_string())
        .filter(|branch| !branch.is_empty())
}

/// Whether `issue`, a sibling under the milestone, built the slice's branch
/// already and is closed: landed, or closed by a person. One branch is one
/// feature — the slice is no task to open again.
#[must_use]
pub fn built_already(issue: &Issue, item: &TaskItem) -> bool {
    let branch = item.branch.trim();
    !issue.is_open()
        && !branch.is_empty()
        && crate::dev_loop::data::tasks::declared_branch(&issue.body).as_deref() == Some(branch)
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
    fn a_slice_that_is_not_whole_is_refused_and_says_why() {
        let why = |reason: &str| match asked(reason) {
            Some(Asked::Refused { why, said, title }) => {
                assert_ne!(said, "");
                assert_ne!(title, "");
                why
            }
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
        assert!(
            matches!(asked(&human), Some(Asked::Unwritable { .. })),
            "a person's, not refused"
        );
    }

    /// The two stops #219's session wrote, 2026-10-25 07:04 and 07:38.
    const STOP_219_ROUND_1: &str = include_str!("../../../tests/fixtures/stop-219-round-1.txt");
    const STOP_219_ROUND_2: &str = include_str!("../../../tests/fixtures/stop-219-round-2.txt");

    #[test]
    fn a_unit_no_key_names_is_refused_with_every_key_listed() {
        let stop = STOP_219_ROUND_2.replace(r#""unit": "server-fixtures""#, r#""unit": "widget""#);
        let found = asked_all(&stop).expect("a list");
        assert_eq!(found.len(), 2);
        for one in &found {
            let Asked::Refused { title, why, said } = one else {
                panic!("{one:?}");
            };
            assert!(title.starts_with("Server fixtures: load the"), "{title}");
            assert!(said.contains(r#""unit":"widget""#), "{said}");
            assert!(
                why.contains(r#"its `unit` ("widget") is no unit of the architecture — one of: "#),
                "{why}"
            );
            for unit in Unit::ALL {
                assert!(why.contains(unit.key()), "{}: {why}", unit.key());
            }
        }
        assert_eq!(
            unit_keys(),
            "capability, micro-ui, concept, data-capability, persisted-query, composition, \
             aggregate, infrastructure, tooling"
        );
    }

    #[test]
    fn server_fixtures_and_plumbing_read_as_infrastructure_and_say_so() {
        for stop in [STOP_219_ROUND_1, STOP_219_ROUND_2] {
            let found = asked_all(stop).expect("a list");
            assert_eq!(found.len(), 2, "{found:?}");
            for one in &found {
                assert!(matches!(one, Asked::Task(_)), "{one:?}");
            }
        }
        let Some(Asked::Task(fixtures)) = asked_all(STOP_219_ROUND_2)
            .expect("a list")
            .into_iter()
            .next()
        else {
            panic!("a task");
        };
        assert_eq!(fixtures.unit, "infrastructure");
        assert!(
            fixtures.brief.ends_with(
                "The stop declared its unit as `server-fixtures`, which the architecture does \
                 not name: the harness reads it as `infrastructure` — fixtures and seed data the \
                 server loads write the store, so this task is on the write side."
            ),
            "{}",
            fixtures.brief
        );
        for spelled in [
            "plumbing",
            "Server Fixtures",
            "seed_data",
            "FIXTURES",
            "server-seed",
        ] {
            assert_eq!(aliased(spelled), Some(Unit::Infrastructure), "{spelled}");
        }
        for unknown in ["server", "widget", "", "fixture-ui"] {
            assert_eq!(aliased(unknown), None, "{unknown}");
        }
        // A unit the vocabulary names is never rewritten.
        let Some(Asked::Task(item)) = asked(BATTLEMAP) else {
            panic!("a task");
        };
        assert!(!item.brief.contains("The stop declared its unit"));
    }

    #[test]
    fn plumbing_and_tooling_may_produce_nothing_but_a_unit_with_manifests_may_not() {
        let bare = |unit: &str| {
            BATTLEMAP
                .replace(r#""unit": "aggregate""#, &format!(r#""unit": "{unit}""#))
                .replace(r#""produces": ["BattlemapInstance"]"#, r#""produces": []"#)
        };
        assert!(matches!(
            asked(&bare("infrastructure")),
            Some(Asked::Task(_))
        ));
        assert!(matches!(asked(&bare("tooling")), Some(Asked::Task(_))));
        assert!(
            matches!(asked(&bare("aggregate")), Some(Asked::Refused { why, .. }) if why.contains("`produces`"))
        );
    }

    #[test]
    fn a_declaration_is_known_by_its_branches() {
        let round_1 = asked_all(STOP_219_ROUND_1).expect("a list");
        assert_eq!(
            declared(&round_1),
            [
                "feat/micro-ui-previous-summary-session",
                "feat/server-session-scene-journal-battlemap-fixtures"
            ]
        );
        let widget =
            STOP_219_ROUND_2.replace(r#""unit": "server-fixtures""#, r#""unit": "widget""#);
        assert_eq!(
            declared(&asked_all(&widget).expect("a list")),
            [
                "feat/server-battlemap-instance-fixtures",
                "feat/server-session-scene-journal-fixtures"
            ],
            "a refused slice by its branch too"
        );
        let vague = asked("AGENT_LOOP_STOP: task the  Projection").expect("one");
        let key = key_of("the  Projection");
        assert_eq!(declared(&[vague]), [key.as_str()]);
        assert!(key.starts_with("the-projection-"), "{key}");
        assert_eq!(key, key_of("THE projection"), "case and spacing aside");
        assert_ne!(key, key_of("the projection, again"));
        assert_eq!(key_of(", ; !").len(), "slice-".len() + 8);
        assert!(key_of(&"word ".repeat(40)).len() <= KEY_WORDS_MAX + 9);
        // A branch git would not spell is keyed by the words around it.
        let odd = BATTLEMAP.replace("feat/table-battlemap-instance-projection", "feat/a,b");
        let keys = declared(&asked_all(&odd).expect("one"));
        assert!(!keys[0].contains(','), "{keys:?}");
        assert_eq!(branch_key(" feat/x "), Some("feat/x".to_string()));
        assert_eq!(branch_key("feat"), None);
        assert_eq!(branch_key(&format!("feat/{}", "x".repeat(96))), None);
        assert!(branch_key(&format!("feat/{}", "x".repeat(95))).is_some());
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(title_of(r#"{"title": "  A   b "}"#), "A b");
        assert_eq!(title_of(&"x".repeat(200)).len(), TITLE_MAX);
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

    #[test]
    fn a_closed_sibling_on_the_slice_s_branch_built_it_already() {
        let item = item();
        let sibling = |body: &str, state: &str| Issue {
            number: 300,
            title: "Another title".to_string(),
            body: body.to_string(),
            state: state.to_string(),
            ..Issue::default()
        };
        let on_branch = "## Scope\n\nbranch: feat/table-battlemap-instance-projection\n";
        assert!(built_already(&sibling(on_branch, "closed"), &item));
        assert!(
            !built_already(&sibling(on_branch, "open"), &item),
            "an open one is waited on"
        );
        assert!(!built_already(
            &sibling("## Scope\n\nbranch: feat/other\n", "closed"),
            &item
        ));
        let mut branchless = item;
        branchless.branch = "  ".to_string();
        assert!(!built_already(
            &sibling("## Scope\n\nbranch: \n", "closed"),
            &branchless
        ));
    }

    fn slice(title: &str, id: &str) -> String {
        format!(
            r#"{{"title": "{title}", "branch": "feat/{id}", "brief": "b", "unit": "data-capability", "system": "campaign", "aggregate": "Entity", "produces": ["campaign.{id}"], "goal": "g", "acceptance": ["given a, when b, then c"]}}"#
        )
    }

    fn titles(found: &[Asked]) -> Vec<String> {
        found
            .iter()
            .map(|asked| match asked {
                Asked::Task(item) => item.title.clone(),
                Asked::Refused { why, .. } => format!("refused: {why}"),
                Asked::Unwritable { why, .. } => format!("unwritable: {why}"),
            })
            .collect()
    }

    proptest::proptest! {
        #[test]
        fn a_list_of_whole_slices_reads_back_whole(n in 1usize..=MAX_SLICES) {
            let slices: Vec<String> = (0..n).map(|k| slice(&format!("Save {k}"), &format!("save{k}"))).collect();
            let stop = format!("AGENT_LOOP_STOP: tasks [{}] (/code)", slices.join(", "));
            let found = asked_all(&stop).expect("a list");
            let expected: Vec<String> = (0..n).map(|k| format!("Save {k}")).collect();
            proptest::prop_assert_eq!(titles(&found), expected);
        }

        #[test]
        fn every_key_declared_reads_back_from_a_rendered_question(
            words in "[ -~]{1,120}",
            n in 1usize..4,
        ) {
            let vague = format!("AGENT_LOOP_STOP: task {words}, with filters");
            let unread = format!("AGENT_LOOP_STOP: tasks [{{\"title\": \"{}\"}}, 3]", words.replace(['"', '\\'], ""));
            let listed: Vec<String> = (0..n).map(|k| slice(&format!("Save {k}, {words}").replace(['"', '\\'], ""), &format!("save{k}"))).collect();
            let whole = format!("AGENT_LOOP_STOP: tasks [{}]", listed.join(", "));
            for stop in [vague, unread, whole] {
                let Some(found) = asked_all(&stop) else { continue };
                let keys = declared(&found);
                let question = crate::common::decision::Question {
                    blocks: "b".to_string(),
                    declared: keys.clone(),
                    ..crate::common::decision::Question::default()
                };
                let read = crate::common::decision::Question::parse(&question.render()).expect("a question");
                proptest::prop_assert_eq!(read.declared, keys);
            }
        }

        #[test]
        fn titles_compare_case_and_space_aside(title in "[A-Za-z]{1,8}( [A-Za-z]{1,8}){0,3}") {
            let spaced = format!("  {}  ", title.to_uppercase().replace(' ', "   "));
            proptest::prop_assert_eq!(folded(&title), folded(&spaced));
        }
    }

    #[test]
    fn one_task_object_is_a_list_of_one() {
        let found = asked_all(BATTLEMAP).expect("one");
        assert_eq!(found.len(), 1);
        assert!(matches!(&found[0], Asked::Task(_)));
        assert_eq!(asked_all("AGENT_LOOP_STOP: contract a.b — c"), None);
    }

    #[test]
    fn duplicate_titles_in_one_stop_open_once() {
        let stop = format!(
            "AGENT_LOOP_STOP: tasks [{}, {}, {}]",
            slice("Save entity", "save"),
            slice("save   ENTITY", "other"),
            slice("List entities", "list")
        );
        assert_eq!(
            titles(&asked_all(&stop).expect("a list")),
            ["Save entity", "List entities"]
        );
    }

    #[test]
    fn nine_slices_are_unwritable() {
        let slices: Vec<String> = (0..9)
            .map(|k| slice(&format!("T{k}"), &format!("t{k}")))
            .collect();
        let found =
            asked_all(&format!("AGENT_LOOP_STOP: tasks [{}]", slices.join(","))).expect("x");
        assert_eq!(found.len(), 1);
        assert!(matches!(&found[0], Asked::Refused { why, .. } if why.contains("too many slices")));
    }

    #[test]
    fn a_same_id_different_title_is_the_same_task_only_until_it_started() {
        // ForgeDesMondes' #448 and #473: two changes to one id.
        let item = match asked(&format!(
            "AGENT_LOOP_STOP: task {}",
            slice("Add source", "saveEntity")
        )) {
            Some(Asked::Task(item)) => *item,
            other => panic!("{other:?}"),
        };
        let mut existing = Issue {
            number: 473,
            state: "open".to_string(),
            title: "Save entity v2".to_string(),
            labels: vec![labels::AGENT.to_string()],
            body: "## Architecture\n\nunit: data-capability\nproduces: campaign.saveEntity"
                .to_string(),
            ..Issue::default()
        };
        assert!(
            is_the_same(&existing, &item),
            "not started: the same unit's task"
        );
        existing.labels.push(labels::REVIEW_PENDING.to_string());
        assert!(!is_the_same(&existing, &item), "in review: another task");
        existing.title = "add   SOURCE".to_string();
        assert!(is_the_same(&existing, &item), "the same title always is");
        // The same branch is the same task, whatever its title or state.
        existing.title = "Reworded".to_string();
        existing.body = "## Scope\n\nbranch: feat/saveEntity\n".to_string();
        assert!(is_the_same(&existing, &item), "one branch, one task");
        existing.body = "## Scope\n\nbranch: feat/other\n".to_string();
        assert!(!is_the_same(&existing, &item));
        let mut branchless = item;
        branchless.branch = String::new();
        existing.body = "## Scope\n\nbranch: \n".to_string();
        assert!(
            !is_the_same(&existing, &branchless),
            "no branch names nothing"
        );
    }
}
