//! The train's order and its bookkeeping, decided from the plan's graph
//! (contract R) with no I/O: which milestone may land, which ids a landing
//! builds.
//!
//! Several milestones build at once, each against the contracts the others
//! produce — mocked until built. The train keeps them honest: a consumer
//! lands after its producers, every landing marks the ids it built in the
//! graph (the mock server stops standing in for them), and every branch
//! still building takes it through the catch-up route (`common::catch_up`),
//! so the next merge meets no surprise.

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
        .filter(|producer| unlanded(*producer, milestones))
        .collect()
}

/// The milestones before `milestone` in plan order that have not landed:
/// open and not `harness:waiting-merge`, in plan order.
///
/// What a task may wait on while it builds
/// ([`crate::dev_loop::data::tasks::runs_ahead`]): every id a task needs is
/// judged against its own producer, so a producer the graph's edges forgot
/// still holds it — #28's #219 needed #24's `OralBrief` and #28 named no
/// edge to #24. Later milestones never count: a read of one is a plan
/// error, never a wait. Empty without a graph, or for a milestone the
/// graph does not know.
#[must_use]
pub fn unlanded_before(graph: Option<&Graph>, milestone: u64, milestones: &[Issue]) -> Vec<u64> {
    let Some(graph) = graph else {
        return Vec::new();
    };
    graph
        .before(milestone)
        .into_iter()
        .filter(|earlier| unlanded(*earlier, milestones))
        .collect()
}

/// The producers of what `tasks` need that have not landed, before
/// `milestone` in plan order, each once, in plan order.
///
/// The edges the graph forgot, read from the tasks themselves: #29's #465
/// needed `SuggestionPill`, which #27 produces, and #29's `depends_on`
/// named no #27. Every task counts, closed or not — what a landed task
/// built on must have landed before its milestone does.
#[must_use]
pub fn need_producers(
    graph: Option<&Graph>,
    milestone: u64,
    milestones: &[Issue],
    tasks: &[Issue],
) -> Vec<u64> {
    let Some(graph) = graph else {
        return Vec::new();
    };
    let needed: Vec<u64> = tasks
        .iter()
        .filter_map(crate::dev_loop::data::tasks::declaration)
        .flat_map(|declaration| declaration.needs)
        .filter_map(|id| graph.ids.get(&id).and_then(|interface| interface.producer))
        .collect();
    graph
        .before(milestone)
        .into_iter()
        .filter(|earlier| needed.contains(earlier) && unlanded(*earlier, milestones))
        .collect()
}

/// What a milestone's landing waits on: the producers the graph names
/// ([`producers_landed`]) and those of what its tasks need
/// ([`need_producers`]), each once, in plan order. Empty when it may land.
#[must_use]
pub fn landing_waits(
    graph: Option<&Graph>,
    milestone: u64,
    milestones: &[Issue],
    tasks: &[Issue],
) -> Vec<u64> {
    let named = producers_landed(graph, milestone, milestones);
    let needed = need_producers(graph, milestone, milestones, tasks);
    let Some(graph) = graph else {
        return Vec::new();
    };
    graph
        .before(milestone)
        .into_iter()
        .filter(|earlier| named.contains(earlier) || needed.contains(earlier))
        .collect()
}

/// Whether `number` is an open milestone not landed yet: one gone from the
/// open list is closed.
fn unlanded(number: u64, milestones: &[Issue]) -> bool {
    milestones
        .iter()
        .find(|m| m.number == number)
        .is_some_and(|m| m.is_open() && !m.has(labels::WAITING_MERGE))
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

    /// #28 needs #24's `OralBrief` and #27's `SuggestionPill` with no edge
    /// to either; #29 comes after #28 and depends on it.
    fn forgotten_edges() -> Graph {
        Graph::parse(
            r#"{"version":1,"milestones":[
              {"number":24,"title":"brief","systems":["session"],"produces":["OralBrief"],"consumes":[],"depends_on":[]},
              {"number":27,"title":"pill","systems":["ai"],"produces":["SuggestionPill"],"consumes":[],"depends_on":[]},
              {"number":28,"title":"screens","systems":["session"],"produces":[],"consumes":[],"depends_on":[]},
              {"number":29,"title":"later","systems":["ai"],"produces":["ai.late"],"consumes":["x"],"depends_on":[28]}
            ],"ids":{
              "OralBrief":{"kind":"micro-ui","producer":24,"state":"planned","version":1},
              "SuggestionPill":{"kind":"micro-ui","producer":27,"state":"mocked","version":1},
              "ai.late":{"kind":"capability","producer":29,"state":"planned","version":1},
              "ai.there":{"kind":"capability","producer":null,"state":"built","version":1}
            }}"#,
        )
        .expect("a graph")
    }

    fn needing(number: u64, needs: &str, state: &str) -> Issue {
        Issue {
            body: format!("## Architecture\n\nunit: composition\nsystem: session\nneeds: {needs}"),
            ..issue(number, &[labels::AGENT], state)
        }
    }

    #[test]
    fn a_task_need_produced_earlier_holds_the_task_and_the_landing() {
        let graph = forgotten_edges();
        let open = [
            issue(24, &[labels::MILESTONE, labels::TRIGGERED], "open"),
            issue(
                27,
                &[labels::MILESTONE, labels::TRIGGERED, labels::WAITING_MERGE],
                "open",
            ),
            issue(28, &[labels::MILESTONE, labels::TRIGGERED], "open"),
            issue(29, &[labels::MILESTONE, labels::TRIGGERED], "open"),
        ];
        // What #28's tasks may wait on: every milestone before it not landed.
        assert_eq!(unlanded_before(Some(&graph), 28, &open), [24]);
        assert_eq!(unlanded_before(Some(&graph), 29, &open), [24, 28]);
        assert!(unlanded_before(Some(&graph), 24, &open).is_empty(), "first");
        assert!(unlanded_before(None, 28, &open).is_empty(), "no graph");
        assert!(
            unlanded_before(Some(&graph), 7, &open).is_empty(),
            "unknown"
        );
        // #219, delivered and closed, still holds #28's landing on #24: the
        // graph names no edge, the need does.
        let screen = needing(219, "OralBrief, SuggestionPill, ai.there, wiring", "closed");
        let tasks = [screen];
        assert!(
            producers_landed(Some(&graph), 28, &open).is_empty(),
            "should be empty"
        );
        assert_eq!(need_producers(Some(&graph), 28, &open, &tasks), [24]);
        assert_eq!(landing_waits(Some(&graph), 28, &open, &tasks), [24]);
        assert!(
            landing_waits(None, 28, &open, &tasks).is_empty(),
            "should be empty"
        );
        // Every producer landed: nothing holds it.
        let landed: Vec<Issue> = open.iter().filter(|m| m.number != 24).cloned().collect();
        assert!(
            landing_waits(Some(&graph), 28, &landed, &tasks).is_empty(),
            "should be empty"
        );
        // A graph edge and a need meet in plan order, each once.
        let later = [
            needing(465, "OralBrief", "open"),
            needing(466, "OralBrief", "open"),
        ];
        assert_eq!(landing_waits(Some(&graph), 29, &open, &later), [24, 28]);
        assert_eq!(need_producers(Some(&graph), 29, &open, &later), [24]);
        // A task that needs nothing adds nothing to the graph's edges.
        let plain = [issue(470, &[labels::AGENT], "open")];
        assert_eq!(landing_waits(Some(&graph), 29, &open, &plain), [28]);
    }

    #[test]
    fn a_need_produced_later_in_plan_order_holds_nothing() {
        let graph = forgotten_edges();
        let open = [
            issue(28, &[labels::MILESTONE, labels::TRIGGERED], "open"),
            issue(29, &[labels::MILESTONE, labels::TRIGGERED], "open"),
        ];
        // #28 reading what #29 produces is a plan error, never a wait: two
        // milestones reading each other would wait forever.
        let reads_ahead = [needing(300, "ai.late", "open")];
        assert!(
            need_producers(Some(&graph), 28, &open, &reads_ahead).is_empty(),
            "should be empty"
        );
        assert!(
            landing_waits(Some(&graph), 28, &open, &reads_ahead).is_empty(),
            "should be empty"
        );
        // Its own milestone's id holds nothing either.
        let own = [needing(301, "ai.late", "open")];
        assert!(
            need_producers(Some(&graph), 29, &open, &own).is_empty(),
            "should be empty"
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
}
