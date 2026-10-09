//! What the plant is made of before any run is read: the rooms, the models
//! that get a chimney, and one production line per workflow, station by
//! station.
//!
//! Declared here rather than derived from `harness-workflows`' stage tables.
//! A table is built against ports and boxed actions, and the only thing it
//! says about itself is a name and whether a stage pays. Which station a stage
//! *looks like* — a scanner, a robot with a wrench, a printer — is a fact of
//! the view, so the view declares it. The stage names here are the ones the
//! runs log under `[stage]`, which is what lets `assemble` place the product.

use serde::Serialize;

/// How a station looks, and what kind of step it stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A gate: a deterministic check. A scanner arch over the belt.
    Scanner,
    /// A paid session that writes. A humanoid robot holding a wrench.
    Builder,
    /// A paid session that reads and judges. A humanoid robot with a clipboard.
    Inspector,
    /// A free step that gathers context. A printer feeding sheets.
    Printer,
    /// A free step that acts on the code or on GitHub. A robotic arm.
    Arm,
}

/// One gate a scanner stands for, named as the workflow names it — the
/// name every `gate_checked` event of a run carries, which is what lets the
/// view pair a verdict with its arch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateSpec {
    /// Unique within its line; the name, dashed.
    pub id: String,
    /// `code requires` — the `Gate::name` in `harness-workflows`.
    pub name: String,
    /// What the gate checks, in a sentence.
    pub purpose: String,
}

/// One station along a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Station {
    /// Unique within its line; what the front-end addresses.
    pub id: String,
    /// What the sign over the station says.
    pub label: String,
    /// What it looks like.
    pub kind: Kind,
    /// The stage whose progress this station reflects, as `run.log` names
    /// it. A gate carries none: it belongs to the stage beside it.
    pub stage: Option<String>,
    /// The model a paid station opens by default.
    pub model: Option<String>,
    /// What it is for, in a sentence — what the pane's `?` says.
    pub purpose: String,
    /// The gates a scanner stands for: one, or several when gates follow
    /// each other on the belt (a stage's `post` and the next one's `pre`)
    /// and are shown as one arch. Empty for every other kind.
    pub gates: Vec<GateSpec>,
}

/// One production line: a workflow, as the plant shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Line {
    /// The folder under `.llocal/logs/` this workflow writes its runs to.
    pub id: String,
    /// The name on the line.
    pub title: String,
    /// What starts it.
    pub trigger: String,
    /// In belt order.
    pub stations: Vec<Station>,
    /// What the whole workflow is for, in a sentence — what the pane's `?`
    /// says on the line and its start belt.
    pub purpose: String,
}

/// How far along a room is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomStatus {
    /// Shows real data.
    Live,
    /// Shows real data, but only part of what the room is meant to.
    Draft,
    /// Barriers and cones.
    Construction,
}

/// One of the six rooms inside the plant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Room {
    /// 1 to 6, the order on the floor plan.
    pub id: u8,
    /// What the front-end addresses.
    pub key: &'static str,
    /// The name on the door.
    pub name: &'static str,
    /// One sentence on what the room is for.
    pub blurb: &'static str,
    /// How far along it is.
    pub status: RoomStatus,
}

/// The dev loop's first free station: it has no `[stage]` line of its own, so
/// `assemble` marks it done once the run named its task.
pub const PICK: &str = "pick";

/// The dev loop's last free station: done once the run reported a delivery.
pub const DELIVER: &str = "deliver";

/// The six rooms, in floor-plan order.
pub const ROOMS: [Room; 6] = [
    Room {
        id: 1,
        key: "lines",
        name: "Assembly lines",
        blurb: "One production line per workflow; the product advances stage by stage.",
        status: RoomStatus::Live,
    },
    Room {
        id: 2,
        key: "office",
        name: "Architecture office",
        blurb: "The issues in detail, the mock-up, the feature list, the data sources — and where the need gets built with Claude Code.",
        status: RoomStatus::Draft,
    },
    Room {
        id: 3,
        key: "store",
        name: "Distribution",
        blurb: "The current version, the agents' integration branch, each milestone's branch.",
        status: RoomStatus::Live,
    },
    Room {
        id: 4,
        key: "infirmary",
        name: "Infirmary",
        blurb: "The doctor: a check-up of the plant, and the repair `harness doctor` performs when asked.",
        status: RoomStatus::Live,
    },
    Room {
        id: 5,
        key: "control",
        name: "Control room",
        blurb: "Spending, tokens, quota windows and the last stops.",
        status: RoomStatus::Live,
    },
    Room {
        id: 6,
        key: "construction",
        name: "Under construction",
        blurb: "Nothing here yet.",
        status: RoomStatus::Construction,
    },
];

fn free(stage: &str, label: &str, kind: Kind, purpose: &str) -> Station {
    Station {
        id: stage.to_string(),
        label: label.to_string(),
        kind,
        stage: Some(stage.to_string()),
        model: None,
        purpose: purpose.to_string(),
        gates: Vec::new(),
    }
}

/// A gate, as the workflow names it.
fn gate(name: &str, purpose: &str) -> GateSpec {
    GateSpec {
        id: name.replace(' ', "-"),
        name: name.to_string(),
        purpose: purpose.to_string(),
    }
}

/// The arch one gate stands for.
fn scanner(gate: GateSpec) -> Station {
    Station {
        id: gate.id.clone(),
        label: gate.name.clone(),
        kind: Kind::Scanner,
        stage: None,
        model: None,
        purpose: gate.purpose.clone(),
        gates: vec![gate],
    }
}

/// A paid stage as the belt shows it: the gate before it when the stage
/// declares one, the robot, the gate after it when it declares one. Only
/// the gates `harness-workflows` really builds are drawn: an arch nobody
/// walks through would show idle forever.
fn paid(
    stage: &str,
    label: &str,
    kind: Kind,
    model: &str,
    purpose: &str,
    pre: Option<GateSpec>,
    post: Option<GateSpec>,
) -> Vec<Station> {
    let mut stations = Vec::new();
    if let Some(pre) = pre {
        stations.push(scanner(pre));
    }
    stations.push(Station {
        id: stage.to_string(),
        label: label.to_string(),
        kind,
        stage: Some(stage.to_string()),
        model: Some(model.to_string()),
        purpose: purpose.to_string(),
        gates: Vec::new(),
    });
    if let Some(post) = post {
        stations.push(scanner(post));
    }
    stations
}

/// Two gates that follow each other on the belt — a stage's `post`, the next
/// stage's `pre` — are one arch: the product walks through both before it
/// moves on, and two arches a hand apart said nothing more than one.
fn merge_adjacent_gates(flat: Vec<Station>) -> Vec<Station> {
    let mut stations: Vec<Station> = Vec::new();
    for station in flat {
        let gated = !station.gates.is_empty();
        if gated && let Some(last) = stations.last_mut().filter(|last| !last.gates.is_empty()) {
            last.id = format!("{}+{}", last.id, station.id);
            last.gates.extend(station.gates);
            continue;
        }
        stations.push(station);
    }
    // A merged arch is named by its neighbours, and its purpose lists its
    // gates: a reader of the sign knows what stands between the two stages.
    for i in 0..stations.len() {
        if stations[i].gates.len() < 2 {
            continue;
        }
        let before = stations[..i]
            .iter()
            .rev()
            .find(|s| s.gates.is_empty())
            .map(|s| s.label.clone());
        let after = stations[i + 1..]
            .iter()
            .find(|s| s.gates.is_empty())
            .map(|s| s.label.clone());
        stations[i].label = match (before, after) {
            (Some(b), Some(a)) => format!("{b} → {a}"),
            (None, Some(a)) => format!("before {a}"),
            (Some(b), None) => format!("after {b}"),
            (None, None) => "gates".to_string(),
        };
        let names: Vec<&str> = stations[i].gates.iter().map(|g| g.name.as_str()).collect();
        stations[i].purpose = format!(
            "{} gates in a row — {}. The product walks through all of them before it moves on.",
            names.len(),
            names.join(", ")
        );
    }
    stations
}

fn line(id: &str, title: &str, trigger: &str, purpose: &str, parts: Vec<Vec<Station>>) -> Line {
    Line {
        id: id.to_string(),
        title: title.to_string(),
        trigger: trigger.to_string(),
        stations: merge_adjacent_gates(parts.into_iter().flatten().collect()),
        purpose: purpose.to_string(),
    }
}

/// Every line of the plant, in the order they hang on the wall.
#[must_use]
pub fn lines() -> Vec<Line> {
    vec![
        agent_loop_line(),
        refinement_line(),
        planner_line(),
        split_line(),
        pr_review_line(),
        pr_fix_line(),
        milestone_merge_line(),
        main_agent_merge_line(),
    ]
}

fn milestone_merge_line() -> Line {
    line(
        "milestone-merge",
        "Milestone merge",
        "a task's PR reviewed, CI green",
        "Folds one task into its milestone: once its pull request has its review and every \
         check is green, merges its branch into the milestone branch and marks it delivered.",
        vec![
            vec![free(
                "reviewed",
                "reviewed",
                Kind::Scanner,
                "The last agent review lets it through: its verdict is clean, on the latest \
                 repair.",
            )],
            vec![free(
                "ci-green",
                "CI is green",
                Kind::Scanner,
                "At least one check succeeded and none failed — a gate not implemented yet \
                 reports skipped and does not hold the merge back.",
            )],
            vec![free(
                "merge",
                "merge it",
                Kind::Arm,
                "Merges the task's branch into the milestone branch.",
            )],
            vec![free(
                "deliver",
                "mark delivered",
                Kind::Arm,
                "Takes review-pending off the task and marks it waiting for the milestone's \
                 own merge.",
            )],
        ],
    )
}

fn agent_loop_line() -> Line {
    line(
        "agent-loop",
        "Dev loop",
        "harness:ready on a task",
        "Builds one task end to end: plans it against the code, writes it, tests it, and \
         delivers it as a pull request on its milestone's branch.",
        vec![
            vec![scanner(gate(
                "dev_loop preflight",
                "Before anything is paid for: the quota window has room, the labels and an \
                 open milestone are on GitHub, the skills and the tools are installed.",
            ))],
            vec![free(
                PICK,
                "pick the task",
                Kind::Printer,
                "Takes the next ready, unblocked task of the current milestone and mounts \
                 its branch.",
            )],
            paid(
                "technical-refinement",
                "technical refinement",
                Kind::Builder,
                "opus",
                "Reads the code the task touches and writes its Technical section and its \
                 implementation plan into the issue.",
                Some(gate(
                    "technical-refinement requires",
                    "The stage is in this run and not already done, the issue has no technical \
                     sections yet, and it carries its business sections.",
                )),
                Some(gate(
                    "technical-refinement must achieve",
                    "The issue body, re-read from GitHub, is not empty.",
                )),
            ),
            paid(
                "code",
                "code",
                Kind::Builder,
                "sonnet",
                "Builds the plan on the task's branch, runs the checks, and opens the pull \
                 request.",
                Some(gate(
                    "code requires",
                    "The stage is in this run, not already done or delivered, and the issue \
                     has a SPEC to build.",
                )),
                Some(gate(
                    "code must keep the architecture",
                    "Static analysis of the checkout: the agent-native architecture still \
                     holds, and every Concept a Capability implements is documented.",
                )),
            ),
            paid(
                "create-test",
                "create test",
                Kind::Builder,
                "sonnet",
                "Adds the tests the change actually warrants, each proven red then green.",
                Some(gate(
                    "create-test requires",
                    "The stage is in this run and not already done.",
                )),
                None,
            ),
            vec![scanner(gate(
                "the round must deliver",
                "A merged pull request on the integration branch closes the task.",
            ))],
            vec![free(
                DELIVER,
                "mark delivered",
                Kind::Arm,
                "Marks the task delivered once its PR is merged — or review-pending while a \
                 write-side PR waits for its merge.",
            )],
        ],
    )
}

fn refinement_line() -> Line {
    line(
        "refinement",
        "Refinement",
        "harness:refinement on an issue",
        "Turns an issue into its spec: the business sections first (a task or a milestone), \
         the technical ones later (a task only).",
        vec![
            vec![free(
                "ground",
                "ground the map",
                Kind::Printer,
                "Builds the repository map once, shared by every stage of the round.",
            )],
            paid(
                "explore",
                "explore the repo",
                Kind::Inspector,
                "sonnet",
                "Reads the repository so the sections rest on facts, not guesses.",
                None,
                None,
            ),
            paid(
                "router",
                "route the sections",
                Kind::Inspector,
                "sonnet",
                "From the second round on, picks which sections this round rewrites.",
                Some(gate(
                    "router requires",
                    "A second round or later, with a context to route on — the first round \
                     writes every section and skips the router.",
                )),
                Some(gate(
                    "router must achieve",
                    "The router's reply names known sections.",
                )),
            ),
            paid(
                "sections",
                "write the sections",
                Kind::Builder,
                "sonnet",
                "Writes each section of the phase: Business Goal, Acceptance Criteria, \
                 Business Rules — or Technical and its plan.",
                Some(gate(
                    "section requires",
                    "This round writes the section: the router named it, or the first round \
                     writes them all.",
                )),
                Some(gate(
                    "section must sit in the layout",
                    "The section's design and plan place their unit in the repository layout \
                     rather than waiving it.",
                )),
            ),
            paid(
                "coherence",
                "check coherence",
                Kind::Inspector,
                "sonnet",
                "Reads the round's sections together and fixes what contradicts.",
                None,
                None,
            ),
            paid(
                "human-advice",
                "advise on a human",
                Kind::Inspector,
                "sonnet",
                "Says whether the technical half of a task needs a human decision first.",
                Some(gate(
                    "advice requires",
                    "The issue is a task, not a milestone — only a task gets the advice.",
                )),
                None,
            ),
            vec![scanner(gate(
                "publish requires",
                "Not a dry run: the body is really written back.",
            ))],
            vec![free(
                "publish",
                "publish the body",
                Kind::Arm,
                "Writes the body back into the issue and swaps the labels.",
            )],
        ],
    )
}

fn planner_line() -> Line {
    line(
        "planner",
        "Planner",
        "harness:ready on a roadmap item",
        "Turns a ready roadmap item into the chain of milestones that delivers it.",
        vec![
            vec![free(
                "ground",
                "ground the map",
                Kind::Printer,
                "Builds the repository map once, shared by every stage.",
            )],
            paid(
                "explore",
                "explore the repo",
                Kind::Inspector,
                "sonnet",
                "Reads the repository to see what already exists.",
                None,
                None,
            ),
            vec![free(
                "context",
                "gather the context",
                Kind::Printer,
                "Reads the roadmap item and the milestones already open under it.",
            )],
            paid(
                "plan",
                "plan the milestones",
                Kind::Builder,
                "sonnet",
                "Draws the milestones, write side before the readers that need it, in \
                 delivery order.",
                None,
                Some(gate(
                    "plan must parse",
                    "The reply parses as a JSON array of milestones.",
                )),
            ),
            vec![free(
                "publish",
                "open the milestones",
                Kind::Arm,
                "Opens the milestones, links them to the roadmap item and chains them with \
                 blocked_by.",
            )],
        ],
    )
}

fn split_line() -> Line {
    line(
        "split",
        "Split",
        "harness:ready on a milestone",
        "Turns a ready, refined milestone into its tasks — once the milestone before it is \
         delivered.",
        vec![
            vec![free(
                "context",
                "gather the context",
                Kind::Printer,
                "Reads the milestone, its roadmap item and the repository's architecture.",
            )],
            paid(
                "slice",
                "slice into tasks",
                Kind::Builder,
                "sonnet",
                "Cuts the milestone into tasks, one unit of the architecture each, chained \
                 where one builds on another.",
                None,
                Some(gate(
                    "slice must parse",
                    "The reply parses as a JSON array of task slices.",
                )),
            ),
            vec![free(
                "publish",
                "open the tasks",
                Kind::Arm,
                "Opens the tasks with their branch and side, chained with blocked_by.",
            )],
        ],
    )
}

fn pr_review_line() -> Line {
    line(
        "pr-review",
        "PR review",
        "harness:to-review on a pull request",
        "Gives a pull request an agent review before it is merged, ending on a verdict: \
         a blocking one sends a repair, a clean one lets the merge happen.",
        vec![
            paid(
                "inline",
                "review inline",
                Kind::Inspector,
                "sonnet",
                "Reads the diff and comments where the code is wrong or risky.",
                Some(gate(
                    "inline requires",
                    "The inline pass is not switched off (--no-inline).",
                )),
                None,
            ),
            paid(
                "brief",
                "write the brief",
                Kind::Inspector,
                "sonnet",
                "Writes the review's summary, and its last line: VERDICT blocking (a correctness \
                 or security defect, a bypassable gate) or clean.",
                None,
                None,
            ),
            vec![scanner(gate(
                "publish requires",
                "Not a dry run: the review is really posted.",
            ))],
            vec![free(
                "publish",
                "post the review",
                Kind::Arm,
                "Posts the review on the pull request.",
            )],
        ],
    )
}

fn pr_fix_line() -> Line {
    line(
        "pr-fix",
        "PR fix",
        "harness:pr-fix on a red pull request",
        "Makes one repair attempt on a pull request whose CI went red, whose agent review \
         blocks it, or whose branch no longer merges into its milestone — two per PR at \
         most, then a human decides.",
        vec![
            vec![free(
                "context",
                "read what broke",
                Kind::Printer,
                "Reads the failing checks and their logs.",
            )],
            paid(
                "fix",
                "repair and push",
                Kind::Builder,
                "sonnet",
                "Repairs the branch and pushes once — no second attempt without a human.",
                Some(gate(
                    "fix requires",
                    "Something is really broken on the pull request: a red check, a blocking \
                     review, or a branch that no longer merges.",
                )),
                None,
            ),
        ],
    )
}

fn main_agent_merge_line() -> Line {
    line(
        "main-agent-merge",
        "main_agent merge",
        "every task delivered, CI green",
        "Closes a milestone: once every task is merged into its branch and CI is green, \
         merges that branch into the integration branch.",
        vec![
            vec![free(
                "tasks-delivered",
                "every task delivered",
                Kind::Scanner,
                "Asks git, not the labels: every task of the milestone has its PR merged.",
            )],
            vec![free(
                "ci-green",
                "CI is green",
                Kind::Scanner,
                "Every check on the milestone's pull request succeeded.",
            )],
            vec![free(
                "open-pr",
                "open the PR",
                Kind::Arm,
                "Opens the pull request from the milestone branch to the integration branch.",
            )],
            vec![free(
                "merge",
                "merge it",
                Kind::Arm,
                "Merges it and marks the milestone waiting for its merge to main.",
            )],
        ],
    )
}

/// The models the lines open by default, in order of first appearance — one
/// chimney each. A model only seen in a trace gets its chimney from `assemble`.
#[must_use]
pub fn models() -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for station in lines().iter().flat_map(|line| line.stations.iter()) {
        if let Some(model) = &station.model
            && !seen.contains(model)
        {
            seen.push(model.clone());
        }
    }
    seen
}

/// The model a stage opens by default, looked up across every line.
///
/// The cost ledger carries no model column, so a row's model is read from the
/// stage it names — an estimate, and labelled as one where it is shown.
#[must_use]
pub fn model_of_stage(lines: &[Line], stage: &str) -> Option<String> {
    lines
        .iter()
        .flat_map(|line| line.stations.iter())
        .find(|station| station.stage.as_deref() == Some(stage))
        .and_then(|station| station.model.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::traces::workflow_of_route;

    #[test]
    fn every_line_and_station_says_what_it_is_for() {
        for line in lines() {
            assert!(
                !line.purpose.trim().is_empty(),
                "{} has no purpose",
                line.id
            );
            for station in &line.stations {
                assert!(
                    !station.purpose.trim().is_empty(),
                    "{}/{} has no purpose",
                    line.id,
                    station.id
                );
            }
        }
    }

    #[test]
    fn station_ids_are_unique_within_a_line() {
        for line in lines() {
            let mut ids: Vec<&str> = line.stations.iter().map(|s| s.id.as_str()).collect();
            let before = ids.len();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), before, "duplicate station id in {}", line.id);
        }
    }

    #[test]
    fn every_route_the_router_can_take_has_a_line() {
        let ids: Vec<String> = lines().into_iter().map(|line| line.id).collect();
        for route in [
            "DevLoop { milestone: 17 }",
            "Refinement { issue: 66 }",
            "TechRefinement { issue: 66 }",
            "Split { milestone: 15 }",
            "Planner { roadmap: 12 }",
            "PrReview { pr: \"71\", base: \"main_agent\" }",
            "PrFix { pr: \"71\" }",
            "MergeMainAgent { milestone: 16 }",
        ] {
            let workflow = workflow_of_route(route).expect(route);
            assert!(ids.iter().any(|id| id == workflow), "{route} -> {workflow}");
        }
    }

    #[test]
    fn a_paid_stage_draws_only_the_gates_it_declares() {
        let both = paid(
            "code",
            "code",
            Kind::Builder,
            "sonnet",
            "builds",
            Some(gate("code requires", "a spec")),
            Some(gate("code must keep the architecture", "holds")),
        );
        let kinds: Vec<Kind> = both.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [Kind::Scanner, Kind::Builder, Kind::Scanner]);
        assert_eq!(both[1].stage.as_deref(), Some("code"));
        assert!(both[0].stage.is_none() && both[2].stage.is_none());
        assert_eq!(both[0].id, "code-requires");
        assert_eq!(both[0].gates[0].name, "code requires");
        let none = paid(
            "brief",
            "brief",
            Kind::Inspector,
            "sonnet",
            "writes",
            None,
            None,
        );
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].gates.len(), 0);
    }

    #[test]
    fn two_gates_in_a_row_are_one_arch_named_by_its_neighbours() {
        let dev = lines()
            .into_iter()
            .find(|l| l.id == "agent-loop")
            .expect("agent-loop");
        let arch = dev
            .stations
            .iter()
            .find(|s| s.id == "technical-refinement-must-achieve+code-requires")
            .expect("the merged arch");
        assert_eq!(arch.kind, Kind::Scanner);
        assert_eq!(arch.label, "technical refinement → code");
        let names: Vec<&str> = arch.gates.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            ["technical-refinement must achieve", "code requires"]
        );
        assert!(arch.purpose.contains("2 gates"));
        // A lone gate keeps its own name on the sign.
        let lone = dev
            .stations
            .iter()
            .find(|s| s.id == "dev_loop-preflight")
            .expect("the preflight arch");
        assert_eq!(lone.label, "dev_loop preflight");
        assert_eq!(lone.gates.len(), 1);
        // Two free scanners that are stages of their own are never merged.
        let merge = lines()
            .into_iter()
            .find(|l| l.id == "main-agent-merge")
            .expect("main-agent-merge");
        assert!(merge.stations.iter().any(|s| s.id == "tasks-delivered"));
        assert!(merge.stations.iter().any(|s| s.id == "ci-green"));
    }

    #[test]
    fn every_gate_name_is_unique_within_its_line() {
        for line in lines() {
            let mut names: Vec<&str> = line
                .stations
                .iter()
                .flat_map(|s| s.gates.iter().map(|g| g.name.as_str()))
                .collect();
            let before = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), before, "duplicate gate name in {}", line.id);
        }
    }

    #[test]
    fn the_dev_loop_logs_the_three_stages_the_blueprint_names() {
        let dev = lines()
            .into_iter()
            .find(|l| l.id == "agent-loop")
            .expect("agent-loop");
        let staged: Vec<String> = dev.stations.into_iter().filter_map(|s| s.stage).collect();
        assert_eq!(
            staged,
            [PICK, "technical-refinement", "code", "create-test", DELIVER]
        );
    }

    #[test]
    fn two_models_get_a_chimney_by_default() {
        assert_eq!(models(), ["opus", "sonnet"]);
        assert_eq!(model_of_stage(&lines(), "code").as_deref(), Some("sonnet"));
        assert_eq!(
            model_of_stage(&lines(), "technical-refinement").as_deref(),
            Some("opus")
        );
        assert_eq!(model_of_stage(&lines(), "publish"), None);
    }

    #[test]
    fn the_six_rooms_are_numbered_in_order() {
        let ids: Vec<u8> = ROOMS.iter().map(|room| room.id).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5, 6]);
    }
}
