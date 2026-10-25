//! A contract change asked by a session that stopped on it — the dev loop's
//! code session, or a pull request's repair — written the same way for
//! both.
//!
//! A session stops with `AGENT_LOOP_STOP: contract <id> — <need>` on a
//! contract another milestone produces: it may not edit it (gate
//! `contracts-untouched`). [`ask`] decides whose it is and opens the
//! request:
//!
//! - **the requester's own** ([`Asked::TasksOwn`]): an aggregate — its
//!   invariants are the writer's (contract I) — or a contract of the
//!   requester's own milestone. Nothing is opened; the caller answers.
//! - **one open request already asks the same id** ([`Asked::Existing`]):
//!   matched by the id the request's body declares, whoever asked it — a
//!   task PR and its landing PR, two siblings, stopping on one id never make
//!   the manager evolve the contract twice.
//! - **otherwise** ([`Asked::Opened`]): a `harness:contract-change` issue,
//!   the contract manager's.
//!
//! **A landed producer's contract is not the requester's own**: a consumer
//! editing a landed manifest is refused by `contracts-untouched` and by its
//! closure (`ForgeDesMondes`' #482 was told to, and was refused). The
//! manager opens the producer task under the requester's milestone instead.
//!
//! A task requester gets the stop under its `## Decision` (`kind:
//! contract`) and a blocker on the request; a milestone requester — its
//! landing PR's repair stopped — gets neither: its blockers are its split's,
//! and its PR's own marker holds it (`common::contest::CONTRACT_WAIT`).

use harness_architecture::graph::{self, Graph, Kind};
use harness_core::domain::Outcome;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook};

use crate::common::decision::Question;
use crate::common::{labels, sections};
use crate::contract_manager::data::request::Request;

/// Who stopped on the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requester {
    /// The task, or the milestone whose landing PR's repair stopped.
    pub issue: u64,
    /// Its milestone: the task's, or the milestone itself.
    pub milestone: Option<u64>,
    /// Its body, as read: a task's `## Decision` is written over it.
    pub body: String,
    /// Whether it is a task — a milestone gets no `## Decision` and no
    /// blocker.
    pub task: bool,
}

/// What became of the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// A request opened, this issue.
    Opened(u64),
    /// An open request already asks the same id: this one.
    Existing(u64),
    /// The requester's own to evolve, and why.
    TasksOwn(String),
}

impl Asked {
    /// The request the requester waits on, when it waits on one.
    #[must_use]
    pub const fn request(&self) -> Option<u64> {
        match self {
            Self::Opened(issue) | Self::Existing(issue) => Some(*issue),
            Self::TasksOwn(_) => None,
        }
    }
}

/// Why the contract `id` is the requester's own to evolve, if it is.
///
/// An aggregate, or a contract of its own milestone (`milestone`). `None` when
/// another milestone builds it — landed or not — or the graph on
/// `integration` does not know it.
pub async fn tasks_own(
    gh: &dyn GitHub,
    milestone: Option<u64>,
    id: &str,
    integration: &str,
) -> Option<String> {
    let graph = gh
        .file_text(graph::PATH, integration)
        .await
        .ok()
        .flatten()
        .and_then(|text| Graph::parse(&text).ok())?;
    let entry = graph.ids.get(id)?;
    if entry.kind == Kind::Aggregate {
        return Some(format!(
            "`{id}` is an aggregate: its invariants are the writer's"
        ));
    }
    let producer = entry.producer?;
    (milestone == Some(producer)).then(|| format!("`{id}` is this milestone's own (#{producer})"))
}

/// The open `harness:contract-change` issue that already asks `id`.
async fn existing(gh: &dyn GitHub, id: &str) -> Outcome<Option<u64>> {
    Ok(gh
        .issues_labelled(labels::CONTRACT_CHANGE, "open")
        .await?
        .iter()
        .filter(|issue| issue.is_open())
        .find(|issue| Request::parse(&issue.body).is_some_and(|asked| asked.id == id))
        .map(|issue| issue.number))
}

/// Asks the change of contract `id` for `requester`, which needs `need`.
///
/// # Errors
/// [`harness_core::domain::Halt`] from a read or a write.
pub async fn ask(
    gh: &dyn GitHub,
    requester: &Requester,
    (id, need): (&str, &str),
    integration: &str,
    log: &Logbook,
) -> Outcome<Asked> {
    if let Some(why) = tasks_own(gh, requester.milestone, id, integration).await {
        return Ok(Asked::TasksOwn(why));
    }
    if requester.task {
        let question = Question {
            kind: Some("contract".to_string()),
            blocks: format!("contract {id} — {need}"),
            ..Question::default()
        };
        let kept = sections::replace(&requester.body, &[(sections::DECISION, &question.render())]);
        gh.set_body(requester.issue, &kept).await?;
    }
    let asked = if let Some(issue) = existing(gh, id).await? {
        log.say(&format!(
            "#{}: contract change #{issue} already asks `{id}` — waiting on it",
            requester.issue
        ));
        Asked::Existing(issue)
    } else {
        Asked::Opened(open(gh, requester, id, need, log).await?)
    };
    if requester.task
        && let Some(issue) = asked.request()
        && !gh
            .blocked_by(requester.issue)
            .await?
            .iter()
            .any(|blocker| blocker.number == issue)
    {
        gh.add_blocked_by(requester.issue, issue).await?;
    }
    Ok(asked)
}

/// Opens the request: the label created on a board that lacks it, then the
/// issue the contract manager takes.
async fn open(
    gh: &dyn GitHub,
    requester: &Requester,
    id: &str,
    need: &str,
    log: &Logbook,
) -> Outcome<u64> {
    if !gh
        .labels()
        .await?
        .iter()
        .any(|l| l == labels::CONTRACT_CHANGE)
    {
        let label = labels::ALL
            .iter()
            .find(|l| l.name == labels::CONTRACT_CHANGE);
        gh.create_label(
            labels::CONTRACT_CHANGE,
            label.map_or("bfdadc", |l| l.color),
            label.map_or("", |l| l.description),
        )
        .await?;
    }
    let request = Request {
        id: id.to_string(),
        task: requester.issue,
        milestone: requester.milestone,
        need: need.to_string(),
    };
    let issue = gh
        .create_issue(
            &request.title(),
            &request.render(),
            &[labels::CONTRACT_CHANGE],
        )
        .await?;
    log.event(&Event::ContractChangeAsked {
        issue,
        task: requester.issue,
        id: id.to_string(),
    });
    Ok(issue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Issue;

    /// #20 produces `session.listScenes`, landed (`built`), and the
    /// `Session` aggregate; the requester's milestone is #4.
    const GRAPH: &str = r#"{"version":1,"milestones":[
      {"number":20,"title":"sessions","systems":["session"],"produces":["Session"],"consumes":[],"depends_on":[]},
      {"number":4,"title":"pay","systems":["pay"],"produces":["pay.charge"],"consumes":["Session"],"depends_on":[20]}
    ],"ids":{
      "Session":{"kind":"aggregate","producer":20,"state":"built","version":1},
      "session.listScenes":{"kind":"capability","producer":20,"state":"built","version":1},
      "pay.charge":{"kind":"capability","producer":4,"state":"planned","version":1}
    }}"#;

    fn gh(requests: Vec<Issue>) -> FakeGitHub {
        FakeGitHub {
            files: std::collections::HashMap::from([(
                ("plan/graph.json".to_string(), "main_agent".to_string()),
                GRAPH.to_string(),
            )]),
            issues: requests,
            labels: vec![labels::CONTRACT_CHANGE.to_string()],
            ..FakeGitHub::default()
        }
    }

    fn task() -> Requester {
        Requester {
            issue: 11,
            milestone: Some(4),
            body: "## Scope\n\nbranch: feat/pay".to_string(),
            task: true,
        }
    }

    #[tokio::test]
    async fn an_aggregate_or_its_own_milestone_s_contract_is_the_task_s_own() {
        for id in ["Session", "pay.charge"] {
            let gh = gh(Vec::new());
            let asked = ask(
                &gh,
                &task(),
                (id, "a field"),
                "main_agent",
                &Logbook::null(),
            )
            .await
            .expect("asked");
            assert!(matches!(asked, Asked::TasksOwn(_)), "{id}: {asked:?}");
            assert!(gh.writes().is_empty(), "{id}");
        }
    }

    #[tokio::test]
    async fn a_landed_producer_s_capability_is_managed_not_the_task_s_own() {
        let gh = gh(Vec::new());
        let asked = ask(
            &gh,
            &task(),
            ("session.listScenes", "the scenes' media ids"),
            "main_agent",
            &Logbook::null(),
        )
        .await
        .expect("asked");
        assert_eq!(asked, Asked::Opened(1));
        let writes = gh.writes();
        assert!(matches!(&writes[0], Wrote::Body(11, body) if body.contains("kind: contract")));
        assert!(writes.contains(&Wrote::BlockedByLink(11, 1)));
    }

    #[test]
    fn a_request_is_waited_on_its_own_number_and_the_task_s_own_on_none() {
        assert_eq!(Asked::Opened(42).request(), Some(42));
        assert_eq!(Asked::Existing(503).request(), Some(503));
        assert_eq!(Asked::TasksOwn("own".to_string()).request(), None);
    }

    #[tokio::test]
    async fn a_task_blocked_by_another_issue_still_gets_the_request_s_blocker() {
        let blocked = |by: u64| Issue {
            number: 11,
            blocked_by: vec![Issue {
                number: by,
                ..Issue::default()
            }],
            ..Issue::default()
        };
        // Blocked by #9: the request #1 is linked too.
        let elsewhere = FakeGitHub {
            subs: vec![(4, vec![blocked(9)])],
            ..gh(Vec::new())
        };
        let asked = ask(
            &elsewhere,
            &task(),
            ("session.listScenes", "x"),
            "main_agent",
            &Logbook::null(),
        )
        .await
        .expect("asked");
        assert_eq!(asked, Asked::Opened(1));
        assert!(elsewhere.writes().contains(&Wrote::BlockedByLink(11, 1)));
        // Blocked by the request already: never linked twice.
        let already = FakeGitHub {
            subs: vec![(4, vec![blocked(1)])],
            ..gh(Vec::new())
        };
        ask(
            &already,
            &task(),
            ("session.listScenes", "x"),
            "main_agent",
            &Logbook::null(),
        )
        .await
        .expect("asked");
        assert!(
            !already
                .writes()
                .iter()
                .any(|wrote| matches!(wrote, Wrote::BlockedByLink(..)))
        );
    }

    #[tokio::test]
    async fn a_second_stop_on_the_same_id_reuses_the_open_request_whoever_asked() {
        let open = Issue {
            number: 503,
            state: "open".to_string(),
            labels: vec![labels::CONTRACT_CHANGE.to_string()],
            body: Request {
                id: "session.listScenes".to_string(),
                task: 482,
                milestone: Some(22),
                need: "a source field".to_string(),
            }
            .render(),
            ..Issue::default()
        };
        let gh = gh(vec![open]);
        // The landing PR of milestone #4: no Decision, no blocker on it.
        let landing = Requester {
            issue: 4,
            milestone: Some(4),
            body: "the milestone".to_string(),
            task: false,
        };
        let asked = ask(
            &gh,
            &landing,
            ("session.listScenes", "x"),
            "main_agent",
            &Logbook::null(),
        )
        .await
        .expect("asked");
        assert_eq!(asked, Asked::Existing(503));
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    }
}
