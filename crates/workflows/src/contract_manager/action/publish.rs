//! What an evolution writes on GitHub.
//!
//! The new version on the integration branch, the graph bumped, the
//! producer's task opened, the consumers told, the request closed — or, for
//! an id nobody else produces, the requester told it is its own to change.

use std::fmt::Write as _;
use std::rc::Rc;

use async_trait::async_trait;
use harness_architecture::graph::{self, Kind, State};
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Event;

use crate::common::{labels, sections};
use crate::contract_manager::data::change::{self, Change};
use crate::contract_manager::data::state::ManagerState;

/// Writes the new version and carries it to everyone it concerns.
pub struct Apply {
    /// What writes on GitHub.
    pub gh: Rc<dyn GitHub>,
    /// The stage whose reply holds the change.
    pub evolve_stage: String,
    /// The integration branch the contracts and the graph are written on.
    pub base_branch: String,
}

impl Apply {
    /// The change as the session answered it.
    fn change(&self, ctx: &Context<ManagerState>) -> Outcome<Change> {
        let Some(reply) = ctx.results.get(&self.evolve_stage) else {
            return Err(Halt::Failed(format!(
                "no reply under {} — the evolve stage did not run",
                self.evolve_stage
            )));
        };
        change::parse(&reply.text, &ctx.state.request().id, ctx.state.version()).map_err(|why| {
            Halt::Failed(format!(
                "the change did not read, even after one retry: {why} — ChangeParses should have \
                 stopped the round"
            ))
        })
    }

    /// An id nobody else produces: the requester changes it itself.
    async fn requester_s_own(&self, ctx: &Context<ManagerState>) -> Outcome<()> {
        let issue = ctx.state.issue();
        let request = ctx.state.request();
        self.gh
            .post_issue_comment(
                request.task,
                &format!(
                    "The contract of `{}` is no other milestone's (the plan's graph names no \
                     producer): change it in this task, manifest first, then the code. {} is \
                     closed.",
                    request.id,
                    issue.reference()
                ),
            )
            .await?;
        self.gh
            .post_issue_comment(
                issue.number,
                &format!(
                    "`{}` is nobody else's contract: #{} changes it itself.",
                    request.id, request.task
                ),
            )
            .await?;
        self.gh.close_issue(issue.number).await?;
        ctx.traces.say(&format!(
            "{}: `{}` is the requester's own — #{} told, nothing paid",
            issue.reference(),
            request.id,
            request.task
        ));
        Ok(())
    }

    /// The manifest, the fixture, the graph: written on the integration
    /// branch, one commit each.
    async fn write_contract(&self, ctx: &ManagerState, change: &Change) -> Outcome<()> {
        let id = &ctx.request().id;
        let pretty = |value: &serde_json::Value| {
            serde_json::to_string_pretty(value).map_or_else(|_| value.to_string(), |t| t + "\n")
        };
        self.gh
            .put_file(
                &change.path,
                &self.base_branch,
                &pretty(&change.manifest),
                &format!("contract({id}): v{} — {}", change.version, change.summary),
            )
            .await?;
        if let Some((path, fixture)) = &change.fixture {
            self.gh
                .put_file(
                    path,
                    &self.base_branch,
                    &pretty(fixture),
                    &format!("mock({id}): the fixture of v{}", change.version),
                )
                .await?;
        }
        if let Some(mut graph) = ctx.graph.clone()
            && let Some(interface) = graph.ids.get_mut(id)
        {
            interface.version = change.version;
            interface.state = State::Evolving;
            let text = graph
                .to_json()
                .map_err(|e| Halt::Failed(format!("the plan's graph cannot be written: {e}")))?;
            self.gh
                .put_file(
                    graph::PATH,
                    &self.base_branch,
                    &text,
                    &format!("chore(plan): {id} evolves to v{}", change.version),
                )
                .await?;
        }
        Ok(())
    }

    /// The task that builds the new version, under the producer's milestone
    /// while it is open, else the requester's; its tap open.
    async fn producer_task(&self, ctx: &ManagerState, change: &Change) -> Outcome<u64> {
        let id = &ctx.request().id;
        let kind = ctx.known.map_or(Kind::Capability, |(kind, _)| kind);
        let slug: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let mut body = format!(
            "## {}\n\nbranch: feat/{}-v{}\n\n{}\n\nThe contract of `{id}` moved to v{} \
             (`{}`): {}. Build the new version; its consumers read it already, from the \
             fixture that stands for it.",
            sections::heading_of(sections::SCOPE),
            slug.trim_matches('-'),
            change.version,
            change.task.brief,
            change.version,
            change.path,
            change.summary
        );
        if !change.task.architecture.is_empty() {
            let _ = write!(
                body,
                "\n\n## {}\n\n{}",
                sections::heading_of(sections::ARCHITECTURE),
                change.task.architecture
            );
        }
        let side = if kind == Kind::DataCapability {
            vec![
                labels::AGENT,
                labels::WRITE_SIDE,
                labels::DATA_LAYER,
                labels::READY,
            ]
        } else {
            vec![labels::AGENT, labels::READ_SIDE, labels::READY]
        };
        let task = self
            .gh
            .create_issue(&change.task.title, &body, &side)
            .await?;
        if let Some(milestone) = ctx.target_milestone {
            self.gh.create_sub_issue_link(milestone, task).await?;
        }
        Ok(task)
    }

    /// Every consumer told, the requester told, the request closed.
    async fn tell(&self, ctx: &ManagerState, change: &Change, producer_task: u64) -> Outcome<()> {
        let request = ctx.request();
        let issue = ctx.issue();
        let note = format!(
            "The contract of `{}` moved to v{} (`{}`): {}.{} #{producer_task} builds it; until \
             then its fixture{} stands for it.",
            request.id,
            change.version,
            change.path,
            change.summary,
            if change.breaking {
                " **Breaking**: what was built against the old version must follow."
            } else {
                ""
            },
            change
                .fixture
                .as_ref()
                .map(|(path, _)| format!(" (`{path}`)"))
                .unwrap_or_default()
        );
        for consumer in ctx
            .consumers
            .iter()
            .filter(|c| Some(**c) != ctx.target_milestone)
        {
            self.gh.post_issue_comment(*consumer, &note).await?;
        }
        self.gh
            .post_issue_comment(
                request.task,
                &format!(
                    "{note} Asked by this task ({}): build against v{} now.",
                    issue.reference(),
                    change.version
                ),
            )
            .await?;
        self.gh
            .post_issue_comment(
                issue.number,
                &format!(
                    "Evolved to v{}: {} — manifest `{}`, built by #{producer_task}, told to {}.",
                    change.version,
                    change.summary,
                    change.path,
                    if ctx.consumers.is_empty() {
                        "no consumer".to_string()
                    } else {
                        ctx.consumers
                            .iter()
                            .map(|n| format!("#{n}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                ),
            )
            .await?;
        self.gh.close_issue(issue.number).await
    }
}

#[async_trait(?Send)]
impl Action<ManagerState> for Apply {
    async fn run(&self, ctx: &mut Context<ManagerState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        if ctx.state.nobody_else_s() {
            self.requester_s_own(ctx).await?;
            return Ok(Verdict::Continue);
        }
        let change = self.change(ctx)?;
        self.write_contract(&ctx.state, &change).await?;
        let producer_task = self.producer_task(&ctx.state, &change).await?;
        self.tell(&ctx.state, &change, producer_task).await?;
        ctx.traces.event(&Event::ContractEvolved {
            id: ctx.state.request().id.clone(),
            version: change.version,
            task: ctx.state.request().task,
            producer_task,
            summary: change.summary.clone(),
        });
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::contract_manager::data::request::Request;
    use harness_architecture::graph::Graph;
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    const GRAPH: &str = r#"{"version":1,"milestones":[
      {"number":19,"title":"media","systems":["media"],"produces":[],"consumes":["session.listScenes"],"depends_on":[20]},
      {"number":20,"title":"sessions","systems":["session"],"produces":["session.listScenes"],"consumes":[],"depends_on":[]},
      {"number":21,"title":"sound","systems":["sound"],"produces":[],"consumes":["session.listScenes"],"depends_on":[20]}
    ],"ids":{"session.listScenes":{"kind":"capability","producer":20,"state":"mocked","version":1}}}"#;

    const REPLY: &str = r#"{"path": "crates/session/capabilities/list-scenes/capability.json",
      "manifest": {"capability": "session.listScenes", "system": "session", "version": 2},
      "fixture_path": "mock/fixtures/session.listScenes.json", "fixture": {"scenes": []},
      "summary": "scenes carry their media ids", "task_title": "session.listScenes v2: media ids",
      "task_brief": "Add mediaIds.", "task_architecture": "unit: Capability\nside: harness:read-side", "breaking": true}"#;

    fn ctx(known: bool, reply: Option<&str>) -> Context<ManagerState> {
        let mut ctx = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            ManagerState {
                issue: Some(Issue {
                    number: 70,
                    ..Issue::default()
                }),
                request: Some(Request {
                    id: "session.listScenes".to_string(),
                    task: 61,
                    milestone: Some(19),
                    need: "media ids".to_string(),
                }),
                graph: known.then(|| Graph::parse(GRAPH).expect("a graph")),
                known: known.then_some((Kind::Capability, 1)),
                producer: known.then_some(20),
                consumers: if known { vec![19, 21] } else { Vec::new() },
                target_milestone: known.then_some(20),
            },
            Logbook::null(),
        );
        if let Some(text) = reply {
            ctx.results.insert(
                "evolve".to_string(),
                Reply {
                    text: text.to_string(),
                    stop_line: None,
                    spend: Spend::default(),
                },
            );
        }
        ctx
    }

    fn apply(gh: &Rc<FakeGitHub>) -> Apply {
        Apply {
            gh: Rc::clone(gh) as Rc<dyn GitHub>,
            evolve_stage: "evolve".to_string(),
            base_branch: "main_agent".to_string(),
        }
    }

    #[tokio::test]
    async fn the_new_version_is_written_bumped_built_by_a_task_and_told_to_everyone() {
        let gh = Rc::new(FakeGitHub {
            next_issue_number: std::cell::Cell::new(100),
            ..FakeGitHub::default()
        });
        let mut context = ctx(true, Some(REPLY));
        apply(&gh).run(&mut context).await.expect("applied");
        let writes = gh.writes();
        let puts: Vec<(&str, &str)> = writes
            .iter()
            .filter_map(|w| match w {
                Wrote::PutFile(path, branch, _) => Some((path.as_str(), branch.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            puts,
            [
                (
                    "crates/session/capabilities/list-scenes/capability.json",
                    "main_agent"
                ),
                ("mock/fixtures/session.listScenes.json", "main_agent"),
                ("plan/graph.json", "main_agent"),
            ]
        );
        let graph_text = writes
            .iter()
            .find_map(|w| match w {
                Wrote::PutFile(path, _, text) if path == "plan/graph.json" => Some(text.clone()),
                _ => None,
            })
            .expect("the graph");
        let graph = Graph::parse(&graph_text).expect("parses");
        assert_eq!(graph.ids["session.listScenes"].version, 2);
        assert_eq!(graph.ids["session.listScenes"].state, State::Evolving);
        let created = writes
            .iter()
            .find_map(|w| match w {
                Wrote::CreatedIssue(title, body, labels) => Some((title, body, labels)),
                _ => None,
            })
            .expect("the producer's task");
        assert_eq!(created.0, "session.listScenes v2: media ids");
        assert!(
            created.1.contains("branch: feat/session-listscenes-v2"),
            "{}",
            created.1
        );
        assert!(created.1.contains("## Architecture") && created.1.contains("unit: Capability"));
        assert_eq!(
            created.2,
            &[labels::AGENT, labels::READ_SIDE, labels::READY]
        );
        assert!(writes.contains(&Wrote::SubIssueLink(20, 101)));
        let told: Vec<u64> = writes
            .iter()
            .filter_map(|w| match w {
                Wrote::Comment(n, _) => Some(*n),
                _ => None,
            })
            .collect();
        assert_eq!(
            told,
            [19, 21, 61, 70],
            "consumers, the requester, the request"
        );
        assert!(
            writes
                .iter()
                .any(|w| matches!(w, Wrote::Comment(21, said) if said.contains("Breaking") && said.contains("#101")))
        );
        assert!(writes.contains(&Wrote::Closed(70)));
    }

    #[tokio::test]
    async fn a_write_side_contract_opens_a_data_layer_task() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(
            true,
            Some(
                &REPLY
                    .replace("capability.json", "data-capability.json")
                    .replace(
                        r#""capability": "session.listScenes""#,
                        r#""dataCapability": "session.listScenes""#,
                    ),
            ),
        );
        context.state.known = Some((Kind::DataCapability, 1));
        apply(&gh).run(&mut context).await.expect("applied");
        let labels_written = gh
            .writes()
            .into_iter()
            .find_map(|w| match w {
                Wrote::CreatedIssue(_, _, labels) => Some(labels),
                _ => None,
            })
            .expect("a task");
        assert_eq!(
            labels_written,
            [
                labels::AGENT,
                labels::WRITE_SIDE,
                labels::DATA_LAYER,
                labels::READY
            ]
        );
    }

    #[tokio::test]
    async fn an_id_nobody_else_produces_is_handed_back_and_nothing_is_written_on_the_branch() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, None);
        apply(&gh).run(&mut context).await.expect("handed back");
        let writes = gh.writes();
        assert!(writes.iter().any(
            |w| matches!(w, Wrote::Comment(61, said) if said.contains("change it in this task"))
        ));
        assert!(writes.contains(&Wrote::Closed(70)));
        assert!(
            !writes
                .iter()
                .any(|w| matches!(w, Wrote::PutFile(..) | Wrote::CreatedIssue(..)))
        );
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing_and_a_missing_reply_fails() {
        let gh = Rc::new(FakeGitHub::default());
        let mut dry = ctx(true, None);
        dry.settings.dry_run = true;
        apply(&gh).run(&mut dry).await.expect("dry");
        assert!(gh.writes().is_empty());
        let err = apply(&gh)
            .run(&mut ctx(true, None))
            .await
            .expect_err("no reply");
        assert!(err.reason().contains("did not run"));
    }
}
