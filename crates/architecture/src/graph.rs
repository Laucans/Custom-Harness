//! The plan's graph: what every milestone produces and consumes, and the
//! dependencies that follow — kept in the product at [`PATH`] (contract R).
//!
//! Why it exists: the planner chained milestones only on the aggregates they
//! shared, and left what one milestone *reads* of another unsaid. A screen
//! that lists scenes was planned with no dependency on the milestone that
//! adds them, and found them missing mid-task. The graph says it ahead:
//! every id — an aggregate, a `DataCapability`, a Capability, a Micro-UI, a
//! Concept — with the milestone that produces it, and every milestone with
//! what it consumes. From it come the read-ahead warnings, the critical
//! path a free lane serves first, and — once interfaces are mocked until
//! built — the milestones that may run side by side.
//!
//! Pure: the dossier's components and the milestones' declarations come in
//! as values; what is written where is the workflow's.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Where the product keeps its graph.
pub const PATH: &str = "plan/graph.json";

/// The version this module writes.
pub const VERSION: u32 = 1;

/// What kind of unit an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// An aggregate the write side shapes (`Campaign`).
    Aggregate,
    /// A write (`campaign.createCampaign`).
    DataCapability,
    /// A read (`campaign.listCharacters`).
    Capability,
    /// A micro-frontend (`SceneBoard`).
    MicroUi,
    /// A versioned definition (`Risk@3`).
    Concept,
}

/// Where an interface stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Planned, nothing written yet.
    #[default]
    Planned,
    /// Its contract and fixture exist; consumers build against them.
    Mocked,
    /// Its producer delivered it.
    Built,
    /// A change to its contract is under way.
    Evolving,
}

/// One id of the graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interface {
    /// What it is.
    pub kind: Kind,
    /// The milestone that produces it; `None` when no milestone of the plan
    /// does — already in the repository, or forgotten by the plan.
    pub producer: Option<u64>,
    /// Where it stands.
    #[serde(default)]
    pub state: State,
    /// The version of its contract, from 1.
    #[serde(default = "one")]
    pub version: u32,
}

const fn one() -> u32 {
    1
}

/// One milestone of the graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// Its issue number.
    pub number: u64,
    /// Its title.
    pub title: String,
    /// The systems it works in.
    pub systems: Vec<String>,
    /// The ids it produces, sorted.
    pub produces: Vec<String>,
    /// The ids it consumes and does not produce, sorted.
    pub consumes: Vec<String>,
    /// The milestones that produce what it consumes, sorted.
    pub depends_on: Vec<u64>,
}

/// The graph, as `plan/graph.json` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graph {
    /// The format's version.
    pub version: u32,
    /// The milestones, in plan order.
    pub milestones: Vec<Node>,
    /// Every id, by name.
    pub ids: BTreeMap<String, Interface>,
}

/// What a milestone declares, as the planner wrote it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Milestone {
    /// Its issue number.
    pub number: u64,
    /// Its title.
    pub title: String,
    /// The systems it works in.
    pub systems: Vec<String>,
    /// The aggregates it shapes or writes.
    pub aggregates: Vec<String>,
    /// The Concepts it defines or implements.
    pub concepts: Vec<String>,
    /// The micro-frontends it builds, by name.
    pub micro_uis: Vec<String>,
    /// The writes it adds, by id.
    pub commands: Vec<String>,
}

/// A micro-frontend of the design dossier: what it reads and triggers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Component {
    /// Its name.
    pub name: String,
    /// Its system.
    pub system: String,
    /// The Capabilities it reads.
    pub needs: Vec<String>,
    /// The `DataCapabilities` it triggers.
    pub actions: Vec<String>,
}

/// A milestone that consumes what a later milestone of the plan produces:
/// built in this order, its tasks find the id missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAhead {
    /// The milestone that reads.
    pub consumer: u64,
    /// What it reads.
    pub id: String,
    /// The later milestone that produces it.
    pub producer: u64,
}

/// The system an id belongs to: what comes before its dot.
fn system_of(id: &str) -> Option<&str> {
    id.split_once('.').map(|(system, _)| system)
}

/// An aggregate's name as the graph already spells it — `pj` after `PJ` is
/// the same aggregate, as the milestones' chaining reads them.
fn canonical(ids: &BTreeMap<String, Interface>, name: &str) -> String {
    ids.keys()
        .find(|known| known.eq_ignore_ascii_case(name))
        .map_or_else(|| name.to_string(), Clone::clone)
}

/// The first milestone to name an id produces it.
fn produce(ids: &mut BTreeMap<String, Interface>, id: &str, kind: Kind, number: u64) {
    ids.entry(id.to_string()).or_insert(Interface {
        kind,
        producer: Some(number),
        state: State::Planned,
        version: 1,
    });
}

fn sorted(set: BTreeSet<String>) -> Vec<String> {
    set.into_iter().collect()
}

/// The micro-frontend of the dossier named `name`, if any.
fn component<'a>(components: &'a [Component], name: &str) -> Option<&'a Component> {
    components.iter().find(|c| c.name == *name)
}

/// Whether `m` works in the system of `id`.
fn in_system(m: &Milestone, id: &str) -> bool {
    system_of(id).is_some_and(|system| m.systems.iter().any(|s| s == system))
}

/// What the micro-frontends of `m` read and trigger.
fn read_by<'a>(m: &'a Milestone, components: &'a [Component]) -> impl Iterator<Item = &'a String> {
    m.micro_uis
        .iter()
        .filter_map(move |ui| component(components, ui))
        .flat_map(|c| c.needs.iter().chain(c.actions.iter()))
}

/// Places every id a micro-frontend reads or triggers that no milestone
/// named: a Capability goes to the first milestone whose own
/// micro-frontend reads it, working in its system; else to the first
/// milestone of its system; else to nobody.
fn place_reads(
    ids: &mut BTreeMap<String, Interface>,
    milestones: &[Milestone],
    components: &[Component],
) {
    for m in milestones {
        for ui in &m.micro_uis {
            for need in component(components, ui).map_or(&[][..], |c| c.needs.as_slice()) {
                if !ids.contains_key(need) && in_system(m, need) {
                    produce(ids, need, Kind::Capability, m.number);
                }
            }
        }
    }
    let read: BTreeSet<String> = milestones
        .iter()
        .flat_map(|m| read_by(m, components))
        .cloned()
        .collect();
    let triggered = |id: &str| {
        milestones
            .iter()
            .flat_map(|m| m.micro_uis.iter())
            .filter_map(|ui| component(components, ui))
            .any(|c| c.actions.iter().any(|a| a == id))
    };
    for id in &read {
        if ids.contains_key(id) {
            continue;
        }
        let kind = if triggered(id) {
            Kind::DataCapability
        } else {
            Kind::Capability
        };
        match milestones.iter().find(|m| in_system(m, id)) {
            Some(m) => produce(ids, id, kind, m.number),
            None => {
                ids.insert(
                    id.clone(),
                    Interface {
                        kind,
                        producer: None,
                        state: State::Planned,
                        version: 1,
                    },
                );
            }
        }
    }
}

/// One milestone's node: what it produces, what it consumes of the rest,
/// and whose work that is.
fn node_of(m: &Milestone, ids: &BTreeMap<String, Interface>, components: &[Component]) -> Node {
    let produces: BTreeSet<String> = ids
        .iter()
        .filter(|(_, i)| i.producer == Some(m.number))
        .map(|(id, _)| id.clone())
        .collect();
    let mut consumes: BTreeSet<String> = read_by(m, components).cloned().collect();
    consumes.extend(m.aggregates.iter().map(|a| canonical(ids, a)));
    let consumes: BTreeSet<String> = consumes.difference(&produces).cloned().collect();
    let depends_on: BTreeSet<u64> = consumes
        .iter()
        .filter_map(|id| ids.get(id).and_then(|i| i.producer))
        .filter(|producer| *producer != m.number)
        .collect();
    Node {
        number: m.number,
        title: m.title.clone(),
        systems: m.systems.clone(),
        produces: sorted(produces),
        consumes: sorted(consumes),
        depends_on: depends_on.into_iter().collect(),
    }
}

impl Graph {
    /// Derives the graph from the milestones, in plan order, and the
    /// dossier's components.
    ///
    /// An aggregate, a Concept, a micro-frontend or a write is produced by
    /// the first milestone that names it; a Capability by the first milestone
    /// whose micro-frontends read it and whose systems hold its system, else
    /// by the first milestone working in that system; an id no milestone
    /// produces is kept with no producer. A milestone consumes the ids its
    /// micro-frontends read or trigger and the aggregates it names, minus
    /// what it produces, and depends on their producers.
    #[must_use]
    pub fn derive(milestones: &[Milestone], components: &[Component]) -> Self {
        let mut ids: BTreeMap<String, Interface> = BTreeMap::new();
        for m in milestones {
            for a in &m.aggregates {
                let name = canonical(&ids, a);
                produce(&mut ids, &name, Kind::Aggregate, m.number);
            }
            for c in &m.concepts {
                produce(&mut ids, c, Kind::Concept, m.number);
            }
            for ui in &m.micro_uis {
                produce(&mut ids, ui, Kind::MicroUi, m.number);
            }
            for cmd in &m.commands {
                produce(&mut ids, cmd, Kind::DataCapability, m.number);
            }
        }
        place_reads(&mut ids, milestones, components);
        let nodes = milestones
            .iter()
            .map(|m| node_of(m, &ids, components))
            .collect();
        Self {
            version: VERSION,
            milestones: nodes,
            ids,
        }
    }

    /// Marks where every id stands: `built` when the checkout declares it,
    /// `mocked` when a fixture stands for it, `planned` otherwise. An id
    /// already `evolving` keeps that.
    pub fn mark_states(&mut self, built: &dyn Fn(&str) -> bool, mocked: &dyn Fn(&str) -> bool) {
        for (id, interface) in &mut self.ids {
            if interface.state == State::Evolving {
                continue;
            }
            interface.state = if built(id) {
                State::Built
            } else if mocked(id) {
                State::Mocked
            } else {
                State::Planned
            };
        }
    }

    /// The position of `number` in plan order.
    fn position(&self, number: u64) -> Option<usize> {
        self.milestones.iter().position(|n| n.number == number)
    }

    /// Every read a milestone makes of what a later milestone produces, in
    /// plan order.
    #[must_use]
    pub fn reads_ahead(&self) -> Vec<ReadAhead> {
        let mut found = Vec::new();
        for (at, node) in self.milestones.iter().enumerate() {
            for id in &node.consumes {
                let Some(producer) = self.ids.get(id).and_then(|i| i.producer) else {
                    continue;
                };
                if self.position(producer).is_some_and(|p| p > at) {
                    found.push(ReadAhead {
                        consumer: node.number,
                        id: id.clone(),
                        producer,
                    });
                }
            }
        }
        found
    }

    /// The ids no milestone of the plan produces, sorted.
    #[must_use]
    pub fn unproduced(&self) -> Vec<String> {
        self.ids
            .iter()
            .filter(|(_, i)| i.producer.is_none())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The critical path: the chain of milestones, in plan order, whose
    /// remaining work — `remaining(number)`, the open tasks still chained in
    /// it — sums highest along the dependencies. A free lane serves it first;
    /// what is off it is only taken with the resources left.
    ///
    /// Only the dependencies that run forward in plan order count: a read of
    /// a later milestone is a plan error ([`Self::reads_ahead`]), not a path.
    #[must_use]
    pub fn critical_path(&self, remaining: &dyn Fn(u64) -> u64) -> Vec<u64> {
        let count = self.milestones.len();
        // longest[i]: the heaviest chain from i onward, and the next step of it.
        let mut longest: Vec<(u64, Option<usize>)> = vec![(0, None); count];
        for i in (0..count).rev() {
            let number = self.milestones[i].number;
            let best = self
                .milestones
                .iter()
                .enumerate()
                .skip(i + 1)
                .filter(|(_, later)| later.depends_on.contains(&number))
                .map(|(j, _)| (longest[j].0, j))
                .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
            longest[i] = (
                remaining(number).saturating_add(best.map_or(0, |(weight, _)| weight)),
                best.map(|(_, j)| j),
            );
        }
        let Some(mut at) = (0..count)
            .filter(|i| longest[*i].0 > 0)
            .max_by(|a, b| longest[*a].0.cmp(&longest[*b].0).then(b.cmp(a)))
        else {
            return Vec::new();
        };
        let mut path = vec![self.milestones[at].number];
        while let Some(next) = longest[at].1 {
            path.push(self.milestones[next].number);
            at = next;
        }
        path
    }

    /// The graph as `plan/graph.json` holds it.
    ///
    /// # Errors
    /// `serde_json`'s, which a graph of strings and numbers never raises.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }

    /// A graph read back from its text.
    ///
    /// # Errors
    /// The text is not a graph.
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    fn component(name: &str, system: &str, needs: &[&str], actions: &[&str]) -> Component {
        Component {
            name: name.to_string(),
            system: system.to_string(),
            needs: strings(needs),
            actions: strings(actions),
        }
    }

    /// Three milestones of a tabletop product: campaigns, then
    /// media, then sessions — the media library reading scenes the
    /// sessions milestone adds.
    fn plan() -> (Vec<Milestone>, Vec<Component>) {
        let milestones = vec![
            Milestone {
                number: 18,
                title: "Campaigns".to_string(),
                systems: strings(&["campaign"]),
                aggregates: strings(&["Campaign"]),
                concepts: strings(&["Versioning@1"]),
                micro_uis: strings(&["CampaignOverview"]),
                commands: strings(&["campaign.createCampaign"]),
            },
            Milestone {
                number: 19,
                title: "Media".to_string(),
                systems: strings(&["media"]),
                micro_uis: strings(&["MediaLibrary"]),
                commands: strings(&["media.uploadMedia"]),
                ..Milestone::default()
            },
            Milestone {
                number: 20,
                title: "Sessions".to_string(),
                systems: strings(&["session", "campaign"]),
                aggregates: strings(&["Session", "Campaign"]),
                micro_uis: strings(&["SceneBoard"]),
                commands: strings(&["session.addScene"]),
                ..Milestone::default()
            },
        ];
        let components = vec![
            component(
                "CampaignOverview",
                "campaign",
                &["campaign.getCampaign"],
                &["campaign.createCampaign"],
            ),
            component(
                "MediaLibrary",
                "media",
                &["media.listMedia", "session.listScenes"],
                &["media.uploadMedia"],
            ),
            component(
                "SceneBoard",
                "session",
                &[
                    "session.listScenes",
                    "campaign.listCharacters",
                    "media.listMedia",
                ],
                &["session.addScene", "table.placeTokens"],
            ),
        ];
        (milestones, components)
    }

    #[test]
    fn what_a_milestone_produces_and_consumes_follows_the_dossier() {
        let (milestones, components) = plan();
        let graph = Graph::derive(&milestones, &components);
        let node = |n: u64| {
            graph
                .milestones
                .iter()
                .find(|m| m.number == n)
                .expect("a node")
        };
        assert_eq!(
            node(18).produces,
            strings(&[
                "Campaign",
                "CampaignOverview",
                "Versioning@1",
                "campaign.createCampaign",
                "campaign.getCampaign"
            ])
        );
        assert_eq!(node(18).consumes, Vec::<String>::new());
        assert_eq!(
            node(19).consumes,
            strings(&["session.listScenes"]),
            "media.listMedia is its own"
        );
        assert_eq!(
            node(19).depends_on,
            vec![20],
            "a read ahead, still a dependency"
        );
        // Sessions shares Campaign with #18 and reads from #18 and #19;
        // campaign.listCharacters, which its own SceneBoard reads and no
        // earlier micro-frontend does, is its to build: it works in
        // `campaign` too.
        assert_eq!(
            node(20).consumes,
            strings(&["Campaign", "media.listMedia", "table.placeTokens"])
        );
        assert_eq!(node(20).depends_on, vec![18, 19]);
        assert_eq!(
            graph.ids["campaign.listCharacters"].producer,
            Some(20),
            "the first milestone whose micro-frontend reads it, in its system"
        );
        assert!(
            node(20)
                .produces
                .contains(&"campaign.listCharacters".to_string())
        );
        assert_eq!(graph.ids["table.placeTokens"].kind, Kind::DataCapability);
        assert_eq!(graph.unproduced(), strings(&["table.placeTokens"]));
        assert_eq!(
            graph.reads_ahead(),
            vec![ReadAhead {
                consumer: 19,
                id: "session.listScenes".to_string(),
                producer: 20,
            }]
        );
    }

    #[test]
    fn a_read_no_milestone_of_its_system_reads_goes_to_that_system_s_first() {
        // #2 reads campaign.listPlayers; #1 works in campaign, no
        // micro-frontend of #1 reads it: #1 builds it — a read, not a write.
        let milestones = vec![
            Milestone {
                number: 1,
                title: "Campaigns".to_string(),
                systems: strings(&["campaign"]),
                ..Milestone::default()
            },
            Milestone {
                number: 2,
                title: "Media".to_string(),
                systems: strings(&["media"]),
                micro_uis: strings(&["MediaLibrary"]),
                ..Milestone::default()
            },
        ];
        let components = vec![component(
            "MediaLibrary",
            "media",
            &["campaign.listPlayers"],
            &["media.uploadMedia"],
        )];
        let graph = Graph::derive(&milestones, &components);
        assert_eq!(graph.ids["campaign.listPlayers"].producer, Some(1));
        assert_eq!(graph.ids["campaign.listPlayers"].kind, Kind::Capability);
        assert_eq!(graph.ids["media.uploadMedia"].kind, Kind::DataCapability);
        assert_eq!(graph.milestones[1].depends_on, vec![1]);
    }

    #[test]
    fn states_follow_the_checkout_and_the_fixtures_but_an_evolution_stays() {
        let (milestones, components) = plan();
        let mut graph = Graph::derive(&milestones, &components);
        graph.ids.get_mut("media.listMedia").expect("an id").state = State::Evolving;
        graph.mark_states(&|id| id == "campaign.getCampaign", &|id| {
            id == "session.listScenes" || id == "campaign.getCampaign"
        });
        assert_eq!(
            graph.ids["campaign.getCampaign"].state,
            State::Built,
            "built before mocked"
        );
        assert_eq!(graph.ids["session.listScenes"].state, State::Mocked);
        assert_eq!(graph.ids["session.addScene"].state, State::Planned);
        assert_eq!(graph.ids["media.listMedia"].state, State::Evolving);
    }

    #[test]
    fn an_aggregate_is_one_whatever_its_case() {
        let milestones = vec![
            Milestone {
                number: 1,
                title: "A".to_string(),
                aggregates: strings(&["PJ"]),
                ..Milestone::default()
            },
            Milestone {
                number: 3,
                title: "C".to_string(),
                aggregates: strings(&["pj", "Campagne"]),
                ..Milestone::default()
            },
        ];
        let graph = Graph::derive(&milestones, &[]);
        assert_eq!(
            graph.ids.keys().cloned().collect::<Vec<_>>(),
            strings(&["Campagne", "PJ"])
        );
        assert_eq!(graph.milestones[1].consumes, strings(&["PJ"]));
        assert_eq!(graph.milestones[1].depends_on, vec![1]);
    }

    #[test]
    fn the_critical_path_is_the_heaviest_forward_chain() {
        let (milestones, components) = plan();
        let graph = Graph::derive(&milestones, &components);
        // #20 depends on #18 and #19; #19's read of #20 runs backward and
        // is no path. Weights: 18 → 20 is 5 + 2, 19 → 20 is 1 + 2.
        let remaining = |n: u64| match n {
            18 => 5,
            19 => 1,
            20 => 2,
            _ => 0,
        };
        assert_eq!(graph.critical_path(&remaining), vec![18, 20]);
        let media_heavy = |n: u64| match n {
            19 => 9,
            20 => 2,
            _ => 1,
        };
        assert_eq!(graph.critical_path(&media_heavy), vec![19, 20]);
        assert_eq!(
            graph.critical_path(&|_| 0),
            Vec::<u64>::new(),
            "nothing left"
        );
        let tie = |_: u64| 1;
        assert_eq!(
            graph.critical_path(&tie),
            vec![18, 20],
            "a tie goes to the earliest"
        );
    }

    #[test]
    fn the_graph_reads_back_as_written_and_defaults_what_an_older_one_lacks() {
        let (milestones, components) = plan();
        let graph = Graph::derive(&milestones, &components);
        let text = graph.to_json().expect("json");
        assert!(text.ends_with('\n'));
        assert_eq!(Graph::parse(&text).expect("parses"), graph);
        let older =
            r#"{"version":1,"milestones":[],"ids":{"x.y":{"kind":"capability","producer":null}}}"#;
        let read = Graph::parse(older).expect("parses");
        assert_eq!(read.ids["x.y"].state, State::Planned);
        assert_eq!(read.ids["x.y"].version, 1);
    }

    proptest! {
        #[test]
        fn any_graph_survives_a_round_trip(
            numbers in proptest::collection::vec(1..50u64, 0..5),
            ids in proptest::collection::vec("[a-z]{1,5}\\.[a-zA-Z]{1,6}", 0..5),
        ) {
            let milestones: Vec<Milestone> = numbers
                .iter()
                .enumerate()
                .map(|(i, n)| Milestone {
                    number: *n,
                    title: format!("m{i}"),
                    systems: strings(&["s"]),
                    commands: ids.iter().skip(i).take(1).cloned().collect(),
                    ..Milestone::default()
                })
                .collect();
            let graph = Graph::derive(&milestones, &[]);
            let text = graph.to_json().expect("json");
            prop_assert_eq!(Graph::parse(&text).expect("parses"), graph);
        }
    }
}
