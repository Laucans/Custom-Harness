//! What an evolution reads for free, and what it asks of its one session.

use std::rc::Rc;

use async_trait::async_trait;
use harness_architecture::graph::{self, Graph};
use harness_core::domain::prompts::splice;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::common::labels;
use crate::contract_manager::data::change;
use crate::contract_manager::data::request::Request;
use crate::contract_manager::data::state::ManagerState;

/// Reads, for free, what the session evolves from: the request, the plan's
/// graph — who produces the id, who reads it, its version — and the
/// milestone the producer's task goes under.
pub struct ReadRequest {
    /// What reads the issue and the graph.
    pub gh: Rc<dyn GitHub>,
    /// The integration branch the graph is read off.
    pub base_branch: String,
}

#[async_trait(?Send)]
impl Action<ManagerState> for ReadRequest {
    async fn run(&self, ctx: &mut Context<ManagerState>) -> Outcome<Verdict> {
        let issue = ctx.state.issue().clone();
        let Some(request) = Request::parse(&issue.body) else {
            return Err(Halt::Halted(format!(
                "{} does not read as a contract-change request: no `id:` and `task:` lines — \
                 a person wrote it, and a person says what it asks",
                issue.reference()
            )));
        };
        let graph = self
            .gh
            .file_text(graph::PATH, &self.base_branch)
            .await?
            .and_then(|text| Graph::parse(&text).ok());
        if let Some(graph) = &graph {
            ctx.state.known = graph
                .ids
                .get(&request.id)
                .filter(|interface| interface.producer.is_some())
                .map(|interface| (interface.kind, interface.version));
            ctx.state.producer = graph
                .ids
                .get(&request.id)
                .and_then(|interface| interface.producer);
            ctx.state.consumers = graph
                .milestones
                .iter()
                .filter(|node| node.consumes.contains(&request.id))
                .map(|node| node.number)
                .collect();
        }
        if ctx.state.known.is_some() {
            let open = self.gh.issues_labelled(labels::MILESTONE, "open").await?;
            let producer_open = ctx
                .state
                .producer
                .filter(|producer| open.iter().any(|m| m.number == *producer && m.is_open()));
            ctx.state.target_milestone = producer_open.or(request.milestone);
        }
        ctx.traces.say(&format!(
            "{}: `{}` for #{} — {}",
            issue.reference(),
            request.id,
            request.task,
            match ctx.state.known {
                Some((kind, version)) => format!(
                    "{} v{version}, produced by {}, read by {}",
                    kind.name(),
                    ctx.state
                        .producer
                        .map_or_else(|| "nobody".to_string(), |p| format!("#{p}")),
                    if ctx.state.consumers.is_empty() {
                        "nobody".to_string()
                    } else {
                        ctx.state
                            .consumers
                            .iter()
                            .map(|n| format!("#{n}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                ),
                None => "no milestone's contract: the requester's own".to_string(),
            }
        ));
        ctx.state.graph = graph;
        ctx.state.request = Some(request);
        Ok(Verdict::Continue)
    }
}

/// Sends the prompt that evolves the contract, and asks once more when the
/// answer does not read as a change — the gate after it judges the second.
pub struct AskManager {
    /// The stage name, for the journal and the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: String,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// The integration branch, named to the session.
    pub base_branch: String,
}

/// The values the template is spliced with.
pub(crate) fn values(state: &ManagerState, base: &str) -> Vec<(&'static str, String)> {
    let request = state.request();
    let list = |numbers: &[u64]| {
        if numbers.is_empty() {
            "(none)".to_string()
        } else {
            numbers
                .iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    vec![
        ("num", state.issue().number.to_string()),
        ("id", request.id.clone()),
        (
            "kind",
            state
                .known
                .map_or("interface", |(kind, _)| kind.name())
                .to_string(),
        ),
        ("version", state.version().to_string()),
        (
            "producer",
            state
                .producer
                .map_or_else(|| "(none)".to_string(), |p| format!("#{p}")),
        ),
        ("consumers", list(&state.consumers)),
        ("task", request.task.to_string()),
        (
            "milestone",
            request
                .milestone
                .map_or_else(|| "(unknown)".to_string(), |m| format!("#{m}")),
        ),
        (
            "need",
            if request.need.trim().is_empty() {
                "(unsaid — read the task)".to_string()
            } else {
                request.need.clone()
            },
        ),
        ("base", base.to_string()),
    ]
}

#[async_trait(?Send)]
impl SessionAction<ManagerState> for AskManager {
    async fn run(&self, open: &mut Open<'_, ManagerState>) -> Outcome<Verdict> {
        let owned = values(&open.state, &self.base_branch);
        let pairs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let prompt = splice(&self.template, &pairs);
        let task = open.state.issue().reference();
        let id = open.state.request().id.clone();
        let version = open.state.version();
        let reply =
            ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        if let Err(why) = change::parse(&reply.text, &id, version) {
            let retry = format!(
                "Your answer does not read as a change: {why}. Answer again with ONLY the JSON \
                 object — path, manifest (version above {version}), fixture_path, fixture, \
                 summary, task_title, task_brief, task_goal, task_acceptance, breaking — no prose."
            );
            ask_and_record(open, &retry, &self.stage, 1, &task, self.spending.as_ref()).await?;
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use harness_architecture::graph::Kind;
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;
    use std::collections::HashMap;

    const GRAPH: &str = r#"{"version":1,"milestones":[
      {"number":19,"title":"media","systems":["media"],"produces":["media.listMedia"],"consumes":["session.listScenes"],"depends_on":[20]},
      {"number":20,"title":"sessions","systems":["session"],"produces":["session.listScenes"],"consumes":[],"depends_on":[]},
      {"number":21,"title":"sound","systems":["sound"],"produces":[],"consumes":["session.listScenes"],"depends_on":[20]}
    ],"ids":{
      "media.listMedia":{"kind":"capability","producer":19,"state":"planned","version":1},
      "session.listScenes":{"kind":"capability","producer":20,"state":"mocked","version":2},
      "legacy.thing":{"kind":"capability","producer":null,"state":"built","version":1}
    }}"#;

    fn ctx(body: &str) -> Context<ManagerState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            ManagerState {
                issue: Some(Issue {
                    number: 70,
                    body: body.to_string(),
                    ..Issue::default()
                }),
                ..ManagerState::default()
            },
            Logbook::null(),
        )
    }

    fn gh(open_milestones: &[u64]) -> Rc<FakeGitHub> {
        Rc::new(FakeGitHub {
            issues: open_milestones
                .iter()
                .map(|n| Issue {
                    number: *n,
                    state: "open".to_string(),
                    labels: vec![labels::MILESTONE.to_string()],
                    ..Issue::default()
                })
                .collect(),
            files: HashMap::from([(
                ("plan/graph.json".to_string(), "main_agent".to_string()),
                GRAPH.to_string(),
            )]),
            ..FakeGitHub::default()
        })
    }

    fn read(gh: &Rc<FakeGitHub>) -> ReadRequest {
        ReadRequest {
            gh: Rc::clone(gh) as Rc<dyn GitHub>,
            base_branch: "main_agent".to_string(),
        }
    }

    #[tokio::test]
    async fn the_request_is_read_with_its_producer_consumers_and_version() {
        let gh = gh(&[19, 20, 21]);
        let mut context = ctx("id: session.listScenes\ntask: #61\nmilestone: #19\nneed: media ids");
        read(&gh).run(&mut context).await.expect("read");
        let state = &context.state;
        assert_eq!(state.known, Some((Kind::Capability, 2)));
        assert_eq!(state.producer, Some(20));
        assert_eq!(state.consumers, [19, 21]);
        assert_eq!(state.target_milestone, Some(20), "the producer is open");
        assert_eq!(state.version(), 2);
        let spliced = values(state, "main_agent");
        let get = |k: &str| {
            spliced
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert_eq!(get("kind"), "Capability");
        assert_eq!(get("consumers"), "#19, #21");
        assert_eq!(get("producer"), "#20");
        assert_eq!(get("need"), "media ids");
        assert_eq!(get("task"), "61");
    }

    #[tokio::test]
    async fn a_closed_producer_sends_the_task_to_the_requester_s_milestone() {
        let gh = gh(&[19, 21]);
        let mut context = ctx("id: session.listScenes\ntask: #61\nmilestone: #19");
        read(&gh).run(&mut context).await.expect("read");
        assert_eq!(context.state.target_milestone, Some(19));
        let spliced = values(&context.state, "main_agent");
        assert!(
            spliced
                .iter()
                .any(|(k, v)| *k == "need" && v.contains("unsaid"))
        );
    }

    #[tokio::test]
    async fn an_id_the_graph_does_not_produce_is_nobody_else_s() {
        let gh = gh(&[19]);
        for id in ["legacy.thing", "unknown.id"] {
            let mut context = ctx(&format!("id: {id}\ntask: #61"));
            read(&gh).run(&mut context).await.expect("read");
            assert_eq!(context.state.known, None, "{id}");
            assert!(context.state.nobody_else_s(), "{id}");
            assert_eq!(context.state.target_milestone, None);
        }
        let no_graph = Rc::new(FakeGitHub::default());
        let mut context = ctx("id: session.listScenes\ntask: #61");
        read(&no_graph).run(&mut context).await.expect("read");
        assert!(context.state.nobody_else_s() && context.state.graph.is_none());
    }

    #[tokio::test]
    async fn a_body_that_is_no_request_halts_and_says_so() {
        let gh = gh(&[]);
        let mut context = ctx("Please change the API.");
        let err = read(&gh).run(&mut context).await.expect_err("halts");
        assert!(
            matches!(err, Halt::Halted(ref why) if why.contains("`id:`")),
            "{err:?}"
        );
    }
}
