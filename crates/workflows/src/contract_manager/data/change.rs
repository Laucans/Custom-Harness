//! What the session answers: the new version of a contract, read and
//! checked here with no I/O.

use serde::Deserialize;
use serde_json::Value;

use crate::common::json_reply::{self, ParseError};

/// What the session answers.
#[derive(Debug, Clone, Deserialize)]
struct Answered {
    path: String,
    manifest: Value,
    #[serde(default)]
    fixture_path: Option<String>,
    #[serde(default)]
    fixture: Option<Value>,
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

/// The new version of a contract, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
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

/// The change the session answered, for the id at this version.
///
/// # Errors
///
/// No JSON object; a path that is not a manifest's; a manifest that is not
/// an object, that names another id, or whose version does not go up; an
/// empty summary.
pub fn parse(text: &str, id: &str, current: u32) -> Result<Change, String> {
    let answered: Answered =
        json_reply::parse_object(text).map_err(|e: ParseError| e.to_string())?;
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
        .unwrap_or_default();
    if named != id {
        return Err(format!("the manifest names `{named}`, not `{id}`"));
    }
    let version = object
        .get("version")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| "the manifest has no integer `version`".to_string())?;
    if version <= current {
        return Err(format!(
            "the manifest's version is {version}, and the contract is at {current}: it must go up"
        ));
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
        path,
        manifest: answered.manifest,
        version,
        fixture,
        task: TaskDraft {
            title: if title.is_empty() {
                format!("Build {id} v{version}")
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

    #[test]
    fn a_change_reads_its_path_version_fixture_and_task() {
        let change = parse(REPLY, "session.listScenes", 1).expect("a change");
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
            parse(REPLY, "other.id", 1)
                .expect_err("wrong id")
                .contains("not `other.id`")
        );
        assert!(
            parse(REPLY, "session.listScenes", 2)
                .expect_err("same version")
                .contains("must go up")
        );
        let wrong_path = REPLY.replace("capability.json", "capability.rs");
        assert!(
            parse(&wrong_path, "session.listScenes", 1)
                .expect_err("not a manifest")
                .contains("no manifest's path")
        );
        let no_summary = REPLY.replace("scenes carry their media ids", " ");
        assert!(
            parse(&no_summary, "session.listScenes", 1)
                .expect_err("no summary")
                .contains("no summary")
        );
        let no_version = REPLY.replace(r#""version": 2, "#, "");
        assert!(
            parse(&no_version, "session.listScenes", 1)
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
        let change = parse(bare, "x.do", 2).expect("a change");
        assert_eq!(change.fixture, None);
        assert_eq!(change.task.title, "Build x.do v3");
        assert!(change.task.brief.is_empty());
    }
}
