//! What the session answers, read and checked here with no I/O.
//!
//! One of three things: the contract **evolved** (same id, a higher
//! version), **duplicated** (a new id in the requester's system, at version
//! 1: the architecture's rule for a shared Capability that must behave
//! differently), or the request **handed back** (no contract change: a
//! wrong reading of the frontier, a need the requester meets itself).
//! `ForgeDesMondes`' #227 had only the first: a v2 of
//! `campaign.listCharacters` that changed no field, and a task for nothing.

use serde::Deserialize;
use serde_json::Value;

use crate::common::json_reply::{self, ParseError};

/// What the session answers.
#[derive(Debug, Clone, Deserialize)]
struct Answered {
    /// `evolve` (the default, as the first sessions answered), `duplicate`
    /// or `hand_back`.
    #[serde(default)]
    answer: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    manifest: Value,
    #[serde(default)]
    fixture_path: Option<String>,
    #[serde(default)]
    fixture: Option<Value>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    task_title: String,
    #[serde(default)]
    task_brief: String,
    #[serde(default)]
    task_architecture: String,
    #[serde(default)]
    breaking: bool,
}

/// The task that builds the new version, as the session drafted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDraft {
    /// Its title.
    pub title: String,
    /// What it does, under `## Scope`.
    pub brief: String,
    /// Its `## Architecture` lines (`unit:`, `id:`, `system:`, `side:`…).
    pub architecture: String,
}

/// What the session decided of the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The contract moves to a new version, same id.
    Evolve(Change),
    /// A new Capability, in the requester's system, built from the old one:
    /// the requester reads it instead.
    Duplicate(Change),
    /// No contract changes: why, for the requester.
    HandBack(String),
}

/// The new version of a contract, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The id the manifest carries: the request's, or the duplicate's.
    pub id: String,
    /// The manifest's path in the repository.
    pub path: String,
    /// The manifest, whole, its `version` bumped.
    pub manifest: Value,
    /// The version the manifest now carries.
    pub version: u32,
    /// The fixture that stands for the new version until built, with its
    /// path, when the session wrote one.
    pub fixture: Option<(String, Value)>,
    /// The change, in a sentence.
    pub summary: String,
    /// The task that builds it.
    pub task: TaskDraft,
    /// Whether a consumer built against the old version breaks.
    pub breaking: bool,
}

/// What the session answered, for the id at this version.
///
/// # Errors
///
/// No JSON object; an `answer` that is none of the three; a hand-back with
/// no reason; a path that is not a manifest's; a manifest that is not an
/// object, that names the wrong id (the request's for `evolve`, a new one of
/// another namespace than the request's for `duplicate`), or whose version
/// is wrong (must go up for `evolve`, must be 1 for `duplicate`); an empty
/// summary.
pub fn parse(text: &str, id: &str, current: u32) -> Result<Answer, String> {
    let answered: Answered =
        json_reply::parse_object(text).map_err(|e: ParseError| e.to_string())?;
    match answered.answer.trim() {
        "" | "evolve" => change(answered, id, current, false).map(Answer::Evolve),
        "duplicate" => change(answered, id, current, true).map(Answer::Duplicate),
        "hand_back" => {
            let reason = answered.reason.trim();
            if reason.is_empty() {
                return Err("handed back with no `reason`".to_string());
            }
            Ok(Answer::HandBack(reason.to_string()))
        }
        other => Err(format!(
            "`answer` is `{other}`: one of `evolve`, `duplicate`, `hand_back`"
        )),
    }
}

/// The manifest the session wrote, checked against the request: the same
/// id one version up, or — a duplicate — a new id of the same kind, in
/// another system than the original's, at version 1.
fn change(answered: Answered, id: &str, current: u32, duplicate: bool) -> Result<Change, String> {
    let path = answered.path.trim().trim_start_matches('/').to_string();
    if !(path.ends_with("capability.json") || path.ends_with("data-capability.json")) {
        return Err(format!(
            "`{path}` is no manifest's path: a capability.json or a data-capability.json"
        ));
    }
    let Some(object) = answered.manifest.as_object() else {
        return Err("the manifest is not a JSON object".to_string());
    };
    let named = object
        .get("capability")
        .or_else(|| object.get("dataCapability"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let version = object
        .get("version")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| "the manifest has no integer `version`".to_string())?;
    if duplicate {
        if named == id || named.is_empty() {
            return Err(format!(
                "a duplicate carries a new id, not `{id}`: `<requester's system>.<name>`"
            ));
        }
        if named.split_once('.').map(|(s, _)| s) == id.split_once('.').map(|(s, _)| s) {
            return Err(format!(
                "`{named}` is of `{id}`'s own system: a duplicate goes to the requester's"
            ));
        }
        if version != 1 {
            return Err(format!(
                "the duplicate's version is {version}: a new contract starts at 1"
            ));
        }
    } else {
        if named != id {
            return Err(format!("the manifest names `{named}`, not `{id}`"));
        }
        if version <= current {
            return Err(format!(
                "the manifest's version is {version}, and the contract is at {current}: it must go up"
            ));
        }
    }
    let summary = answered.summary.trim().to_string();
    if summary.is_empty() {
        return Err("the change has no summary".to_string());
    }
    let fixture = match (answered.fixture_path, answered.fixture) {
        (Some(path), Some(value)) if !path.trim().is_empty() => {
            Some((path.trim().trim_start_matches('/').to_string(), value))
        }
        _ => None,
    };
    let title = answered.task_title.trim();
    Ok(Change {
        id: named.clone(),
        path,
        manifest: answered.manifest,
        version,
        fixture,
        task: TaskDraft {
            title: if title.is_empty() {
                format!("Build {named} v{version}")
            } else {
                title.to_string()
            },
            brief: answered.task_brief.trim().to_string(),
            architecture: answered.task_architecture.trim().to_string(),
        },
        summary,
        breaking: answered.breaking,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = r#"Here: {"path": "/crates/session/capabilities/list-scenes/capability.json",
      "manifest": {"capability": "session.listScenes", "system": "session", "version": 2, "output": {"scenes": "[]"}},
      "fixture_path": "mock/fixtures/session.listScenes.json", "fixture": {"scenes": []},
      "summary": "scenes carry their media ids", "task_title": "session.listScenes v2",
      "task_brief": "Add mediaIds to every scene.", "task_architecture": "unit: Capability\nside: harness:read-side",
      "breaking": false}"#;

    fn evolved(text: &str, id: &str, current: u32) -> Result<Change, String> {
        match parse(text, id, current)? {
            Answer::Evolve(change) => Ok(change),
            other => Err(format!("not an evolution: {other:?}")),
        }
    }

    #[test]
    fn a_change_reads_its_path_version_fixture_and_task() {
        let change = evolved(REPLY, "session.listScenes", 1).expect("a change");
        assert_eq!(change.id, "session.listScenes");
        assert_eq!(
            change.path,
            "crates/session/capabilities/list-scenes/capability.json"
        );
        assert_eq!(change.version, 2);
        assert_eq!(
            change.fixture.as_ref().map(|(p, _)| p.as_str()),
            Some("mock/fixtures/session.listScenes.json")
        );
        assert_eq!(change.task.title, "session.listScenes v2");
        assert!(change.task.architecture.contains("unit: Capability"));
        assert!(!change.breaking);
    }

    #[test]
    fn what_does_not_read_as_a_change_says_why() {
        assert!(parse("no json", "a.b", 1).is_err());
        assert!(
            evolved(REPLY, "other.id", 1)
                .expect_err("wrong id")
                .contains("not `other.id`")
        );
        assert!(
            evolved(REPLY, "session.listScenes", 2)
                .expect_err("same version")
                .contains("must go up")
        );
        let wrong_path = REPLY.replace("capability.json", "capability.rs");
        assert!(
            evolved(&wrong_path, "session.listScenes", 1)
                .expect_err("not a manifest")
                .contains("no manifest's path")
        );
        let no_summary = REPLY.replace("scenes carry their media ids", " ");
        assert!(
            evolved(&no_summary, "session.listScenes", 1)
                .expect_err("no summary")
                .contains("no summary")
        );
        let no_version = REPLY.replace(r#""version": 2, "#, "");
        assert!(
            evolved(&no_version, "session.listScenes", 1)
                .expect_err("no version")
                .contains("no integer `version`")
        );
        assert!(
            parse(
                r#"{"path":"x/capability.json","manifest":[],"summary":"s"}"#,
                "a",
                1
            )
            .expect_err("not an object")
            .contains("not a JSON object")
        );
    }

    #[test]
    fn a_fixture_needs_both_its_path_and_its_value_and_a_task_gets_a_default_title() {
        let bare = r#"{"path":"crates/x/data-capability.json","manifest":{"dataCapability":"x.do","version":3},"summary":"s","fixture_path":"mock/fixtures/x.json"}"#;
        let change = evolved(bare, "x.do", 2).expect("a change");
        assert_eq!(change.fixture, None);
        assert_eq!(change.task.title, "Build x.do v3");
        assert!(change.task.brief.is_empty());
        let blank_path = r#"{"path":"crates/x/data-capability.json","manifest":{"dataCapability":"x.do","version":3},"summary":"s","fixture_path":"  ","fixture":{"a":1}}"#;
        assert_eq!(
            evolved(blank_path, "x.do", 2).expect("a change").fixture,
            None,
            "a blank path is no path"
        );
    }

    #[test]
    fn a_duplicate_is_a_new_id_of_another_system_at_version_one() {
        let dup = r#"{"answer":"duplicate","path":"crates/media/capabilities/list-media-places/capability.json",
          "manifest":{"capability":"media.listMediaPlaces","system":"media","version":1},
          "summary":"media reads the session's places itself","task_title":"media.listMediaPlaces v1",
          "task_brief":"Build it.","task_architecture":"unit: Capability\nside: harness:read-side"}"#;
        let Answer::Duplicate(change) = parse(dup, "campaign.listPlaces", 1).expect("a duplicate")
        else {
            panic!("not a duplicate");
        };
        assert_eq!(change.id, "media.listMediaPlaces");
        assert_eq!(change.version, 1);
        let same_id = dup.replace("media.listMediaPlaces", "campaign.listPlaces");
        assert!(
            parse(&same_id, "campaign.listPlaces", 1)
                .expect_err("same id")
                .contains("a new id")
        );
        let same_system = dup.replace("media.listMediaPlaces", "campaign.listPlacesV2");
        assert!(
            parse(&same_system, "campaign.listPlaces", 1)
                .expect_err("same system")
                .contains("own system")
        );
        let v2 = dup.replace("\"version\":1", "\"version\":2");
        assert!(
            parse(&v2, "campaign.listPlaces", 1)
                .expect_err("v2")
                .contains("starts at 1")
        );
    }

    #[test]
    fn a_hand_back_carries_its_reason_and_an_unknown_answer_is_refused() {
        let back = r#"{"answer":"hand_back","reason":"a Micro-UI may read any system's Capabilities: nothing to change"}"#;
        assert_eq!(
            parse(back, "a.b", 1).expect("handed back"),
            Answer::HandBack(
                "a Micro-UI may read any system's Capabilities: nothing to change".to_string()
            )
        );
        assert!(
            parse(r#"{"answer":"hand_back"}"#, "a.b", 1)
                .expect_err("no reason")
                .contains("no `reason`")
        );
        assert!(
            parse(r#"{"answer":"maybe","path":"x","manifest":{}}"#, "a.b", 1)
                .expect_err("unknown")
                .contains("one of")
        );
    }
}
