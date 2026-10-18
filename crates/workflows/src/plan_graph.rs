//! The plan's graph written from the board: every milestone of a roadmap
//! item, as its issue declares it, against the design dossier.
//!
//! A **deterministic command, not a workflow** — like `milestone_merge`: no
//! session, no cost. The planner writes the graph as it opens milestones;
//! this writes it for a plan that exists already — one made before the
//! graph did, or moved by hand — and is what `harness plan-graph <roadmap>`
//! runs. What it derives is [`harness_architecture::graph`]'s; what it says
//! is the planner's own two warnings: a read of a later milestone, an id
//! nobody produces.

use harness_architecture::graph::{Graph, Milestone, ReadAhead};
use harness_core::domain::Outcome;
use harness_core::ports::shell::github::GitHub;

use crate::common::dossier::Dossier;
use crate::common::labels;
use crate::planner::data::plan;

/// What the write found and left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The graph, as written.
    pub graph: Graph,
    /// Every read a milestone makes of a later one.
    pub reads_ahead: Vec<ReadAhead>,
    /// The ids no milestone produces.
    pub unproduced: Vec<String>,
}

impl Written {
    /// What a report says of it, one line per fact.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "plan graph: {} milestone(s), {} id(s)",
            self.graph.milestones.len(),
            self.graph.ids.len()
        )];
        for ahead in &self.reads_ahead {
            lines.push(format!(
                "read ahead: #{} reads `{}`, which #{} produces later",
                ahead.consumer, ahead.id, ahead.producer
            ));
        }
        if !self.unproduced.is_empty() {
            lines.push(format!(
                "no milestone produces: {}",
                self.unproduced.join(", ")
            ));
        }
        lines
    }
}

/// The milestones of `roadmap`, every state, in number order — the plan as
/// the board holds it.
///
/// # Errors
/// The board cannot be read.
pub async fn milestones_of(gh: &dyn GitHub, roadmap: u64) -> Outcome<Vec<Milestone>> {
    let mut issues = gh.sub_issues(roadmap).await?;
    issues.retain(|issue| issue.has(labels::MILESTONE));
    issues.sort_by_key(|issue| issue.number);
    let components = Vec::new();
    Ok(issues
        .iter()
        .map(|issue| plan::graph_input(issue.number, &issue.title, &issue.body, &components, &[]))
        .collect())
}

/// Derives the graph of `roadmap`'s milestones against `dossier`, writes it
/// to `plan/graph.json` on `branch`, and says what it found.
///
/// # Errors
/// The board cannot be read, or GitHub refuses the write.
pub async fn write(
    gh: &dyn GitHub,
    dossier: &Dossier,
    roadmap: u64,
    branch: &str,
) -> Outcome<Written> {
    let components = dossier.components();
    let writes = dossier.write_ids();
    let mut issues = gh.sub_issues(roadmap).await?;
    issues.retain(|issue| issue.has(labels::MILESTONE));
    issues.sort_by_key(|issue| issue.number);
    let milestones: Vec<Milestone> = issues
        .iter()
        .map(|issue| {
            plan::graph_input(
                issue.number,
                &issue.title,
                &issue.body,
                &components,
                &writes,
            )
        })
        .collect();
    let graph = Graph::derive(&milestones, &components);
    let text = graph.to_json().map_err(|e| {
        harness_core::domain::Halt::Failed(format!("the graph does not render: {e}"))
    })?;
    gh.put_file(
        harness_architecture::graph::PATH,
        branch,
        &text,
        "chore(plan): the graph of the milestones (contract R)",
    )
    .await?;
    Ok(Written {
        reads_ahead: graph.reads_ahead(),
        unproduced: graph.unproduced(),
        graph,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::dossier::{MicroUi, Write};
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Issue;

    fn milestone(number: u64, title: &str, body: &str) -> Issue {
        Issue {
            number,
            title: title.to_string(),
            state: if number == 18 { "closed" } else { "open" }.to_string(),
            labels: vec![labels::MILESTONE.to_string()],
            body: body.to_string(),
            ..Issue::default()
        }
    }

    fn dossier() -> Dossier {
        Dossier {
            micro_uis: vec![
                MicroUi {
                    name: "MediaLibrary".to_string(),
                    system: "media".to_string(),
                    needs: vec![
                        "media.listMedia".to_string(),
                        "session.listScenes".to_string(),
                    ],
                    actions: vec!["media.uploadMedia".to_string()],
                    ..MicroUi::default()
                },
                MicroUi {
                    name: "SceneBoard".to_string(),
                    system: "session".to_string(),
                    needs: vec!["session.listScenes".to_string()],
                    actions: vec![
                        "session.addScene".to_string(),
                        "table.placeTokens".to_string(),
                    ],
                    ..MicroUi::default()
                },
            ],
            writes: vec![
                Write {
                    id: "media.uploadMedia".to_string(),
                    ..Write::default()
                },
                Write {
                    id: "session.addScene".to_string(),
                    ..Write::default()
                },
            ],
            ..Dossier::default()
        }
    }

    #[tokio::test]
    async fn the_board_s_milestones_make_the_graph_and_the_warnings_are_said() {
        let gh = FakeGitHub {
            subs: vec![(
                4,
                vec![
                    // Out of order on the board, a non-milestone among them.
                    milestone(
                        20,
                        "Sessions",
                        "Add `session.addScene` and the SceneBoard.\n\n## Architecture\n\nsystems: session\naggregates: Session\nside: harness:write-side",
                    ),
                    Issue {
                        number: 3,
                        labels: vec![labels::HUMAN.to_string()],
                        ..Issue::default()
                    },
                    milestone(
                        19,
                        "Media",
                        "The library: `media.uploadMedia`, MediaLibrary.\n\n## Architecture\n\nsystems: media\nside: harness:read-side",
                    ),
                    milestone(
                        18,
                        "Campaigns",
                        "Campaigns.\n\n## Architecture\n\nsystems: campaign\naggregates: Campaign\nmicro-uis: -\ncommands: -\nside: harness:write-side",
                    ),
                ],
            )],
            ..FakeGitHub::default()
        };
        let written = write(&gh, &dossier(), 4, "main_agent")
            .await
            .expect("written");
        assert_eq!(
            written
                .graph
                .milestones
                .iter()
                .map(|m| m.number)
                .collect::<Vec<_>>(),
            [18, 19, 20],
            "milestones only, in number order"
        );
        assert_eq!(written.graph.ids["session.listScenes"].producer, Some(20));
        assert_eq!(
            written.reads_ahead,
            vec![ReadAhead {
                consumer: 19,
                id: "session.listScenes".to_string(),
                producer: 20,
            }]
        );
        assert_eq!(written.unproduced, vec!["table.placeTokens".to_string()]);
        let lines = written.lines();
        assert_eq!(lines[0], "plan graph: 3 milestone(s), 9 id(s)");
        assert!(lines[1].starts_with("read ahead: #19 reads `session.listScenes`, which #20"));
        assert_eq!(lines[2], "no milestone produces: table.placeTokens");
        let writes = gh.writes();
        let Some(Wrote::PutFile(path, branch, text)) = writes.first() else {
            panic!("{writes:?}");
        };
        assert_eq!(
            (path.as_str(), branch.as_str()),
            ("plan/graph.json", "main_agent")
        );
        assert_eq!(Graph::parse(text).expect("a graph"), written.graph);
        assert_eq!(writes.len(), 1, "the graph, and nothing else");
    }

    #[tokio::test]
    async fn a_plan_without_warnings_says_only_its_size() {
        let gh = FakeGitHub {
            subs: vec![(
                4,
                vec![milestone(
                    18,
                    "Campaigns",
                    "Campaigns.\n\n## Architecture\n\nsystems: campaign\naggregates: Campaign\nside: harness:write-side",
                )],
            )],
            ..FakeGitHub::default()
        };
        let written = write(&gh, &Dossier::default(), 4, "main_agent")
            .await
            .expect("written");
        assert_eq!(
            written.lines(),
            vec!["plan graph: 1 milestone(s), 1 id(s)".to_string()]
        );
        let listed = milestones_of(&gh, 4).await.expect("read");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].aggregates, vec!["Campaign".to_string()]);
    }
}
