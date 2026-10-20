//! The train's order and its bookkeeping, decided from the plan's graph
//! (contract R) with no I/O: which milestone may land, which ids a landing
//! builds, which branches it reaches.
//!
//! Several milestones build at once, each against the contracts the others
//! produce — mocked until built. The train keeps them honest: a consumer
//! lands after its producers, every landing marks the ids it built in the
//! graph (the mock server stops standing in for them) and is brought into
//! every branch still building, so the next merge meets no surprise.

use harness_architecture::graph::{Graph, Kind, State};
use harness_core::domain::Issue;

use crate::common::labels;

/// The producers of what `milestone` consumes that have not landed yet.
///
/// Open and not `harness:waiting-merge`, in plan order. Empty when the
/// milestone may land: no graph, no producers, or every one landed.
#[must_use]
pub fn producers_landed(graph: Option<&Graph>, milestone: u64, milestones: &[Issue]) -> Vec<u64> {
    let Some(graph) = graph else {
        return Vec::new();
    };
    graph
        .waits_on(milestone)
        .into_iter()
        .filter(|producer| {
            milestones
                .iter()
                .find(|m| m.number == *producer)
                .is_some_and(|m| m.is_open() && !m.has(labels::WAITING_MERGE))
        })
        .collect()
}

/// Whether a changed file is a manifest the graph may know an id of.
#[must_use]
pub fn is_manifest(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "capability.json" | "data-capability.json" | "micro-ui.json" | "aggregate.json"
    )
}

/// The id a manifest's text declares, with its kind: `capability`,
/// `dataCapability`, `microUi` or `aggregate`.
#[must_use]
pub fn declared(text: &str) -> Option<(Kind, String)> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let read = |key: &str| {
        json.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    read("capability")
        .map(|id| (Kind::Capability, id))
        .or_else(|| read("dataCapability").map(|id| (Kind::DataCapability, id)))
        .or_else(|| read("microUi").map(|id| (Kind::MicroUi, id)))
        .or_else(|| read("aggregate").map(|id| (Kind::Aggregate, id)))
}

/// Marks `built` every id the manifests declare that the graph knows under
/// the same kind and does not hold built already; returns those marked, in
/// the manifests' order.
pub fn mark_built(graph: &mut Graph, manifests: &[(Kind, String)]) -> Vec<String> {
    let mut marked = Vec::new();
    for (kind, id) in manifests {
        if let Some(interface) = graph.ids.get_mut(id)
            && interface.kind == *kind
            && interface.state != State::Built
        {
            interface.state = State::Built;
            marked.push(id.clone());
        }
    }
    marked
}

/// The milestones a landing reaches: every other one building —
/// `harness:triggered`, open, not `harness:waiting-merge` — by number.
#[must_use]
pub fn building(milestones: &[Issue], landed: u64) -> Vec<&Issue> {
    let mut found: Vec<&Issue> = milestones
        .iter()
        .filter(|m| {
            m.number != landed
                && m.is_open()
                && m.has(labels::TRIGGERED)
                && !m.has(labels::WAITING_MERGE)
        })
        .collect();
    found.sort_by_key(|m| m.number);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64, labels: &[&str], state: &str) -> Issue {
        Issue {
            number,
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn graph() -> Graph {
        Graph::parse(
            r#"{"version":1,"milestones":[
              {"number":20,"title":"sessions","systems":["session"],"produces":["session.listScenes","session.addScene"],"consumes":[],"depends_on":[]},
              {"number":19,"title":"media","systems":["media"],"produces":["media.listMedia"],"consumes":["session.listScenes"],"depends_on":[20]}
            ],"ids":{
              "media.listMedia":{"kind":"capability","producer":19,"state":"planned","version":1},
              "session.listScenes":{"kind":"capability","producer":20,"state":"mocked","version":1},
              "session.addScene":{"kind":"data-capability","producer":20,"state":"planned","version":1}
            }}"#,
        )
        .expect("a graph")
    }

    #[test]
    fn a_consumer_waits_on_its_open_producers_and_no_one_else() {
        let graph = graph();
        let open = [
            issue(19, &[labels::MILESTONE, labels::TRIGGERED], "open"),
            issue(20, &[labels::MILESTONE, labels::TRIGGERED], "open"),
        ];
        assert_eq!(producers_landed(Some(&graph), 19, &open), [20]);
        assert!(
            producers_landed(Some(&graph), 20, &open).is_empty(),
            "producers_landed(Some(&graph), 20, &open) should be empty"
        );
        let landed = [
            issue(19, &[labels::MILESTONE, labels::TRIGGERED], "open"),
            issue(
                20,
                &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
                "open",
            ),
        ];
        assert!(
            producers_landed(Some(&graph), 19, &landed).is_empty(),
            "producers_landed(Some(&graph), 19, &landed) should be empty"
        );
        assert!(
            producers_landed(Some(&graph), 19, &[open[0].clone()]).is_empty(),
            "a producer gone from the open list is closed"
        );
        assert!(producers_landed(None, 19, &open).is_empty(), "no graph");
        assert!(
            producers_landed(Some(&graph), 7, &open).is_empty(),
            "unknown"
        );
        // A read of a later milestone is a plan error, never a wait.
        let ahead = Graph::parse(
            r#"{"version":1,"milestones":[
              {"number":19,"title":"media","systems":["media"],"produces":[],"consumes":["session.listScenes"],"depends_on":[20]},
              {"number":20,"title":"sessions","systems":["session"],"produces":["session.listScenes"],"consumes":[],"depends_on":[]}
            ],"ids":{}}"#,
        )
        .expect("a graph");
        assert!(
            producers_landed(Some(&ahead), 19, &open).is_empty(),
            "producers_landed(Some(&ahead), 19, &open) should be empty"
        );
    }

    #[test]
    fn a_landing_marks_what_its_manifests_declare_under_the_same_kind() {
        let mut graph = graph();
        let manifests = vec![
            (Kind::Capability, "session.listScenes".to_string()),
            (Kind::Capability, "session.addScene".to_string()),
            (Kind::Capability, "nobody.knows".to_string()),
        ];
        assert_eq!(mark_built(&mut graph, &manifests), ["session.listScenes"]);
        assert_eq!(graph.ids["session.listScenes"].state, State::Built);
        assert_eq!(
            graph.ids["session.addScene"].state,
            State::Planned,
            "the kind counts"
        );
        assert!(mark_built(&mut graph, &manifests).is_empty(), "once");
    }

    #[test]
    fn a_manifest_is_known_by_its_name_and_read_by_its_key() {
        assert!(is_manifest(
            "crates/session/capabilities/list/capability.json"
        ));
        assert!(is_manifest("data-capability.json"));
        assert!(
            is_manifest("apps/x/micro-ui.json") && is_manifest("data/aggregates/A/aggregate.json")
        );
        assert!(!is_manifest("mock/pages.json") && !is_manifest("src/capability.rs"));
        assert_eq!(
            declared(r#"{"capability":"session.listScenes","system":"session"}"#),
            Some((Kind::Capability, "session.listScenes".to_string()))
        );
        assert_eq!(
            declared(r#"{"dataCapability":"session.addScene"}"#),
            Some((Kind::DataCapability, "session.addScene".to_string()))
        );
        assert_eq!(
            declared(r#"{"microUi":"SceneBoard"}"#),
            Some((Kind::MicroUi, "SceneBoard".to_string()))
        );
        assert_eq!(
            declared(r#"{"aggregate":"Session"}"#),
            Some((Kind::Aggregate, "Session".to_string()))
        );
        assert_eq!(declared("{"), None);
        assert_eq!(declared(r#"{"other":1}"#), None);
    }

    #[test]
    fn the_landing_reaches_every_other_milestone_building() {
        let milestones = [
            issue(21, &[labels::MILESTONE, labels::TRIGGERED], "open"),
            issue(19, &[labels::MILESTONE, labels::TRIGGERED], "open"),
            issue(20, &[labels::MILESTONE, labels::TRIGGERED], "open"),
            issue(22, &[labels::MILESTONE, labels::READY], "open"),
            issue(
                18,
                &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
                "open",
            ),
            issue(17, &[labels::MILESTONE, labels::TRIGGERED], "closed"),
        ];
        let reached: Vec<u64> = building(&milestones, 20).iter().map(|m| m.number).collect();
        assert_eq!(reached, [19, 21]);
    }
}
