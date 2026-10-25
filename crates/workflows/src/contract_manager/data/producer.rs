//! The task that builds a contract's new version, as split writes a slice —
//! pure.
//!
//! The manager used to write its own body: `## Scope` and the session's
//! `## Architecture` lines, no `## Business Goal`, no `## Acceptance
//! Criteria`, no `produces:`. The task then halted on `TaskHasASpec`, and
//! its closure did not hold the unit (`ForgeDesMondes`' #434, #503, #362:
//! each patched by hand). [`item`] makes it a slice instead — the unit from
//! the graph's kind, the system the id's namespace (2026-10-18: system = id
//! namespace), `produces` the id, the goal and the criteria the session
//! drafted — and `publish` renders it with split's own renderer, so the task
//! passes the same measure every split task does (`plan::spec_gaps`).

use harness_architecture::graph::Kind;

use crate::contract_manager::data::change::Change;
use crate::split::data::plan::TaskItem;

/// The slug of an id, as a branch carries it: `session.listScenes` →
/// `session-listscenes`.
#[must_use]
pub fn slug(id: &str) -> String {
    let dashed: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    dashed.trim_matches('-').to_string()
}

/// The slice that builds `change`, a contract of kind `kind`, with `why`
/// (what moved, for its consumers) after the session's brief.
///
/// A draft with no goal or no criterion — a reply from an older prompt the
/// factory overrides — gets defaults that say what the version must do: it
/// answers as its manifest declares, and as its fixture stands for it.
#[must_use]
pub fn item(change: &Change, kind: Kind, why: &str) -> TaskItem {
    let system = change
        .id
        .split_once('.')
        .map_or_else(String::new, |(system, _)| system.to_string());
    let unit = if kind == Kind::DataCapability {
        "data-capability"
    } else {
        "capability"
    };
    let goal = if change.task.goal.trim().is_empty() {
        format!(
            "Consumers read `{}` v{}: {}.",
            change.id, change.version, change.summary
        )
    } else {
        change.task.goal.trim().to_string()
    };
    let mut acceptance: Vec<String> = change
        .task
        .acceptance
        .iter()
        .map(|criterion| criterion.trim().to_string())
        .filter(|criterion| !criterion.is_empty())
        .collect();
    if acceptance.is_empty() {
        acceptance.push(format!(
            "given the v{} manifest at `{}`, when `{}` is called, then it answers as the \
             manifest declares",
            change.version, change.path, change.id
        ));
        if let Some((fixture, _)) = &change.fixture {
            acceptance.push(format!(
                "given its fixture `{fixture}`, then the built {} answers the same shape",
                if kind == Kind::DataCapability {
                    "DataCapability"
                } else {
                    "Capability"
                }
            ));
        }
    }
    TaskItem {
        title: change.task.title.clone(),
        brief: format!("{}\n\n{why}", change.task.brief.trim())
            .trim()
            .to_string(),
        branch: format!("feat/{}-v{}", slug(&change.id), change.version),
        needs_human: false,
        unit: unit.to_string(),
        system,
        concept: None,
        effect: None,
        touches: Vec::new(),
        aggregate: None,
        produces: vec![change.id.clone()],
        needs: Vec::new(),
        depends_on: Vec::new(),
        rules: Vec::new(),
        goal,
        acceptance,
        business_rules: Vec::new(),
        technical: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_manager::data::change::TaskDraft;
    use crate::split::data::plan;
    use proptest::prelude::*;

    fn change(id: &str, goal: &str, acceptance: &[&str]) -> Change {
        Change {
            id: id.to_string(),
            path: "crates/session/capabilities/list-scenes/capability.json".to_string(),
            manifest: serde_json::json!({}),
            version: 2,
            fixture: Some((
                "mock/fixtures/session.listScenes.json".to_string(),
                serde_json::json!({}),
            )),
            summary: "scenes carry their media ids".to_string(),
            task: TaskDraft {
                title: format!("{id} v2: media ids"),
                brief: "Add mediaIds to every scene.".to_string(),
                goal: goal.to_string(),
                acceptance: acceptance.iter().map(ToString::to_string).collect(),
            },
            breaking: false,
        }
    }

    proptest! {
        #[test]
        fn every_producer_item_is_a_whole_slice(
            system in "[a-z]{1,8}",
            name in "[a-z][a-zA-Z]{0,10}",
            summary in "[a-z ]{1,30}",
            data in any::<bool>(),
        ) {
            let id = format!("{system}.{name}");
            let mut drafted = change(&id, "", &[]);
            drafted.summary = summary;
            let kind = if data { Kind::DataCapability } else { Kind::Capability };
            let slice = item(&drafted, kind, "why");
            prop_assert!(plan::spec_gaps(std::slice::from_ref(&slice)).is_empty());
            prop_assert_eq!(&slice.produces, &vec![id]);
            prop_assert_eq!(&slice.system, &system);
            prop_assert!(slice.branch.starts_with("feat/") && slice.branch.ends_with("-v2"));
        }
    }

    #[test]
    fn the_session_s_goal_and_criteria_are_kept_and_an_old_reply_gets_the_defaults() {
        let drafted = change(
            "session.listScenes",
            "The DM sees each scene's media.",
            &["given a scene with media, when listed, then mediaIds holds them"],
        );
        let slice = item(&drafted, Kind::Capability, "moved to v2");
        assert_eq!(slice.goal, "The DM sees each scene's media.");
        assert_eq!(slice.acceptance.len(), 1);
        assert_eq!(slice.unit, "capability");
        assert_eq!(slice.branch, "feat/session-listscenes-v2");
        assert!(slice.brief.ends_with("moved to v2"));
        let old = item(
            &change("session.listScenes", " ", &[]),
            Kind::DataCapability,
            "",
        );
        assert!(
            old.goal
                .starts_with("Consumers read `session.listScenes` v2")
        );
        assert_eq!(old.acceptance.len(), 2, "{:?}", old.acceptance);
        assert!(old.acceptance[1].contains("DataCapability"));
        assert_eq!(old.unit, "data-capability");
    }
}
