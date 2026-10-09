//! The capabilities list: every Capability and `DataCapability` of the
//! product, where it stands, what it exchanges and who uses it.
//!
//! Read from the manifests the architecture asks for (`capability.json`,
//! `data-capability.json`, `micro-ui.json`, `aggregate.json`,
//! `concept.json`) on every branch of the product, and from the open issues
//! of the board. Pure: the reading is the `Units` port's, the board the
//! poller's.
//!
//! | status | when |
//! | --- | --- |
//! | bug | an open issue labelled `bug` names it |
//! | available | its manifest is on `main` |
//! | building | its manifest is on another branch only |
//! | planned | an open task declares it under `## Architecture`, and no branch has it yet |

use std::collections::{BTreeMap, BTreeSet};

use harness_architecture::vocabulary::{Declaration, Unit};
use harness_core::domain::Issue;
use harness_workflows::common::sections;
use serde::Serialize;
use serde_json::Value;

use crate::ports::UnitsReading;

/// The branch on sale: what is there is available.
pub const RELEASED: &str = "main";

/// Which side of the architecture a unit sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Read-only: computes a value from persisted queries.
    Capability,
    /// A typed command behind the `DataGuard`: the only thing that writes.
    DataCapability,
}

impl Kind {
    /// The word a human reads.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Capability => "capability",
            Self::DataCapability => "data capability",
        }
    }
}

/// Where a unit stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// On `main`.
    Available,
    /// On a branch that is not `main` yet.
    Building,
    /// Declared by an open task, on no branch yet.
    Planned,
    /// An open bug names it.
    Bug,
}

/// How a Micro-UI uses a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// Reads it (`needs`).
    Reads,
    /// Triggers it (`actions`).
    Triggers,
}

/// One Micro-UI that uses a unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Caller {
    /// `Pjs`.
    pub micro_ui: String,
    /// Its system.
    pub system: String,
    /// Reads or triggers.
    pub how: How,
    /// What the screen hands it: `campagneId: ID`.
    pub props: BTreeMap<String, String>,
    /// Its manifest.
    pub path: String,
}

/// One business rule a unit carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rule {
    /// The invariant's id, or the Concept's name.
    pub id: String,
    /// The rule, as written.
    pub text: String,
    /// Where it is written: `aggregate PJ`, `concept NiveauDuGroupe@1`.
    pub from: String,
}

/// An open issue that names a unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueLink {
    /// Its number.
    pub number: u64,
    /// Its title.
    pub title: String,
    /// Labelled `bug`.
    pub bug: bool,
}

/// What a unit exchanges: what the UI hands it, what it reads or writes,
/// what it returns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Contract {
    /// A Capability's `input`, a `DataCapability`'s `payload`.
    pub input: BTreeMap<String, String>,
    /// What a Capability returns.
    pub output: Option<String>,
    /// The persisted queries a Capability reads through: its dataview.
    pub reads: Vec<String>,
    /// The Concept a Capability implements: `Risk@3`.
    pub implements: Option<String>,
    /// `insert`, `update`, `delete`, `upsert`.
    pub effect: Option<String>,
    /// The aggregate a `DataCapability` writes.
    pub target: Option<String>,
    /// The existing fields it touches.
    pub touches: Vec<String>,
    /// `relative`, `overwrite`, `confirm_on_stale`.
    pub mode: Option<String>,
    /// The permissions it asks.
    pub permissions: Vec<String>,
    /// The namespaces that may trigger it.
    pub callable_by: Vec<String>,
}

/// One unit of the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// `campagne.listerPjs`; `#12` for a planned unit, which has no id yet.
    pub id: String,
    /// Read side or write side.
    pub kind: Kind,
    /// The bounded context.
    pub system: String,
    /// The id without its system, or a planned task's title.
    pub name: String,
    /// Where it stands.
    pub status: Status,
    /// When its manifest was first committed, on any branch.
    pub created: Option<String>,
    /// When it was last committed, on any branch.
    pub modified: Option<String>,
    /// What it is for.
    pub role: String,
    /// Its manifest, relative to the product's root.
    pub path: Option<String>,
    /// Its manifest's version.
    pub version: Option<String>,
    /// The branches its manifest is on, `main` first.
    pub branches: Vec<String>,
    /// What it exchanges.
    pub contract: Contract,
    /// The Micro-UIs that use it.
    pub used_by: Vec<Caller>,
    /// The business rules it carries.
    pub rules: Vec<Rule>,
    /// The open issues that name it.
    pub issues: Vec<IssueLink>,
}

/// The list, as the page reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Catalogue {
    /// Every unit, by name.
    pub entries: Vec<Entry>,
    /// The branches read, `main` first.
    pub branches: Vec<String>,
    /// What could not be read: a manifest that is not JSON.
    pub warnings: Vec<String>,
}

/// The manifests of one kind, by id, the first branch read winning.
#[derive(Default)]
struct Found {
    manifests: BTreeMap<String, (String, Value)>,
    branches: BTreeMap<String, Vec<String>>,
}

impl Found {
    fn add(&mut self, id: String, path: &str, json: Value, branch: &str) {
        let on = self.branches.entry(id.clone()).or_default();
        if !on.iter().any(|b| b == branch) {
            on.push(branch.to_string());
        }
        self.manifests
            .entry(id)
            .or_insert_with(|| (path.to_string(), json));
    }
}

/// Everything the manifests say, across branches.
#[derive(Default)]
struct Manifests {
    capabilities: Found,
    data_capabilities: Found,
    micro_uis: Found,
    aggregates: Found,
    concepts: Found,
}

/// The file name a manifest is recognised by.
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The branches in reading order: `main`, the integration branch, then the
/// rest by name — the first that has a manifest gives its content.
fn ordered(reading: &UnitsReading, integration: &str) -> Vec<String> {
    let mut names: Vec<String> = reading.refs.iter().map(|r| r.git_ref.clone()).collect();
    names.sort_by_key(|name| (name != RELEASED, name != integration, name.clone()));
    names.dedup();
    names
}

fn text(json: &Value, key: &str) -> Option<String> {
    json.get(key).and_then(Value::as_str).map(str::to_string)
}

fn strings(json: &Value, key: &str) -> Vec<String> {
    json.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// A `{ name: type }` object; a non-string type is shown as its JSON.
fn shape(json: &Value, key: &str) -> BTreeMap<String, String> {
    json.get(key)
        .and_then(Value::as_object)
        .map(|fields| {
            fields
                .iter()
                .map(|(k, v)| {
                    let kind = v.as_str().map_or_else(|| v.to_string(), str::to_string);
                    (k.clone(), kind)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn read_manifests(
    reading: &UnitsReading,
    branches: &[String],
    warnings: &mut Vec<String>,
) -> Manifests {
    let mut all = Manifests::default();
    for branch in branches {
        let Some(at) = reading.refs.iter().find(|r| &r.git_ref == branch) else {
            continue;
        };
        for file in &at.files {
            let Ok(json) = serde_json::from_str::<Value>(&file.text) else {
                let warning = format!("{} on {branch}: not JSON", file.path);
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
                continue;
            };
            let (found, key) = match file_name(&file.path) {
                "capability.json" => (&mut all.capabilities, "capability"),
                "data-capability.json" => (&mut all.data_capabilities, "dataCapability"),
                "micro-ui.json" => (&mut all.micro_uis, "microUi"),
                "aggregate.json" => (&mut all.aggregates, "aggregate"),
                "concept.json" => (&mut all.concepts, "concept"),
                _ => continue,
            };
            let Some(mut id) = text(&json, key) else {
                continue;
            };
            if key == "concept" {
                let version = json.get("version").and_then(Value::as_u64).unwrap_or(0);
                id = format!("{id}@{version}");
            }
            if key == "microUi" {
                // Two systems may each have a `List`: the system tells them apart.
                id = format!("{}.{id}", text(&json, "system").unwrap_or_default());
            }
            found.add(id, &file.path, json, branch);
        }
    }
    all
}

/// The Micro-UIs that list `id` under `field`.
fn callers(micro_uis: &Found, id: &str, field: &str, how: How) -> Vec<Caller> {
    micro_uis
        .manifests
        .values()
        .filter(|(_, json)| strings(json, field).iter().any(|n| n == id))
        .map(|(path, json)| Caller {
            micro_ui: text(json, "microUi").unwrap_or_default(),
            system: text(json, "system").unwrap_or_default(),
            how,
            props: shape(json, "props"),
            path: path.clone(),
        })
        .collect()
}

/// The invariants a `DataCapability` may affect, as its aggregate words them.
fn invariants(aggregates: &Found, aggregate: &str, ids: &[String]) -> Vec<Rule> {
    let declared: Vec<(String, String)> = aggregates
        .manifests
        .get(aggregate)
        .and_then(|(_, json)| json.get("invariants").and_then(Value::as_array))
        .map(|items| {
            items
                .iter()
                .filter_map(|i| Some((text(i, "id")?, text(i, "rule").unwrap_or_default())))
                .collect()
        })
        .unwrap_or_default();
    ids.iter()
        .map(|id| Rule {
            id: id.clone(),
            text: declared.iter().find(|(d, _)| d == id).map_or_else(
                || "(not declared by the aggregate)".to_string(),
                |(_, r)| r.clone(),
            ),
            from: format!("aggregate {aggregate}"),
        })
        .collect()
}

/// The Concept a Capability implements, as its definition says it.
fn concept_rule(concepts: &Found, implements: &str) -> Rule {
    let definition = concepts
        .manifests
        .get(implements)
        .and_then(|(_, json)| text(json, "definition"))
        .unwrap_or_else(|| "(the Concept is not documented)".to_string());
    Rule {
        id: implements.to_string(),
        text: definition,
        from: format!("concept {implements}"),
    }
}

fn output_of(json: &Value) -> Option<String> {
    json.get("output")
        .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
}

fn split_id(id: &str) -> (String, String) {
    id.split_once('.').map_or_else(
        || (String::new(), id.to_string()),
        |(system, name)| (system.to_string(), name.to_string()),
    )
}

fn capability(id: &str, path: &str, json: &Value, all: &Manifests) -> Entry {
    let (system, name) = split_id(id);
    let implements = text(json, "implements");
    Entry {
        id: id.to_string(),
        kind: Kind::Capability,
        system: text(json, "system").unwrap_or(system),
        name,
        status: Status::Available,
        created: None,
        modified: None,
        role: text(json, "description").unwrap_or_default(),
        path: Some(path.to_string()),
        version: text(json, "version"),
        branches: Vec::new(),
        rules: implements
            .iter()
            .map(|c| concept_rule(&all.concepts, c))
            .collect(),
        contract: Contract {
            input: shape(json, "input"),
            output: output_of(json),
            reads: strings(json, "reads"),
            implements,
            ..Contract::default()
        },
        used_by: callers(&all.micro_uis, id, "needs", How::Reads),
        issues: Vec::new(),
    }
}

fn data_capability(id: &str, path: &str, json: &Value, all: &Manifests) -> Entry {
    let (system, name) = split_id(id);
    let target = json
        .get("target")
        .and_then(|t| text(t, "aggregate"))
        .unwrap_or_default();
    let effect = text(json, "effect").unwrap_or_default();
    let touches = strings(json, "touches");
    let role = if touches.is_empty() {
        format!("{effect} a {target}")
    } else {
        format!("{effect} a {target}: {}", touches.join(", "))
    };
    Entry {
        id: id.to_string(),
        kind: Kind::DataCapability,
        system,
        name,
        status: Status::Available,
        created: None,
        modified: None,
        role,
        path: Some(path.to_string()),
        version: json
            .get("version")
            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string)),
        branches: Vec::new(),
        rules: invariants(&all.aggregates, &target, &strings(json, "invariants")),
        contract: Contract {
            input: shape(json, "payload"),
            effect: Some(effect),
            target: Some(target),
            touches,
            mode: text(json, "mode"),
            permissions: strings(json, "permissions"),
            callable_by: strings(json, "callableBy"),
            ..Contract::default()
        },
        used_by: callers(&all.micro_uis, id, "actions", How::Triggers),
        issues: Vec::new(),
    }
}

fn is_bug(issue: &Issue) -> bool {
    issue
        .labels
        .iter()
        .any(|l| l.to_lowercase().contains("bug"))
}

/// Whether an issue names a unit: its full id anywhere, or its name as a
/// word of the title.
fn names(issue: &Issue, entry: &Entry) -> bool {
    let id = entry.id.to_lowercase();
    let title = issue.title.to_lowercase();
    if title.contains(&id) || issue.body.to_lowercase().contains(&id) {
        return true;
    }
    let name = entry.name.to_lowercase();
    name.len() >= 4
        && title
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word == name)
}

/// A planned unit: an open task that declares a Capability or a
/// `DataCapability` no branch has yet.
fn planned(issue: &Issue) -> Option<Entry> {
    let found = sections::parse(&issue.body);
    let declaration = Declaration::parse(found.get(sections::ARCHITECTURE)?)?;
    let kind = match declaration.unit {
        Unit::Capability | Unit::PersistedQuery => Kind::Capability,
        Unit::DataCapability => Kind::DataCapability,
        _ => return None,
    };
    let brief = found.get(sections::SCOPE).cloned().unwrap_or_default();
    Some(Entry {
        id: format!("#{}", issue.number),
        kind,
        system: declaration.system,
        name: issue.title.clone(),
        status: Status::Planned,
        created: None,
        modified: None,
        role: brief.lines().next().unwrap_or_default().to_string(),
        path: None,
        version: None,
        branches: Vec::new(),
        contract: Contract {
            implements: declaration.concept,
            effect: declaration.effect.map(|e| e.key().to_string()),
            touches: declaration.touches,
            ..Contract::default()
        },
        used_by: Vec::new(),
        rules: Vec::new(),
        issues: vec![IssueLink {
            number: issue.number,
            title: issue.title.clone(),
            bug: false,
        }],
    })
}

/// The list: every unit on a branch, plus what open tasks plan.
///
/// `issues` are the open issues the board read — tasks and bugs; a closed
/// one is ignored.
#[must_use]
pub fn catalogue(reading: &UnitsReading, integration: &str, issues: &[Issue]) -> Catalogue {
    let branches = ordered(reading, integration);
    let mut warnings = Vec::new();
    let all = read_manifests(reading, &branches, &mut warnings);
    let dates: BTreeMap<&str, (&str, &str)> = reading
        .dates
        .iter()
        .map(|d| (d.path.as_str(), (d.created.as_str(), d.modified.as_str())))
        .collect();
    let open: Vec<&Issue> = issues.iter().filter(|i| i.is_open()).collect();

    let mut entries: Vec<Entry> = Vec::new();
    for (found, build) in [
        (
            &all.capabilities,
            capability as fn(&str, &str, &Value, &Manifests) -> Entry,
        ),
        (&all.data_capabilities, data_capability),
    ] {
        for (id, (path, json)) in &found.manifests {
            let mut entry = build(id, path, json, &all);
            entry.branches = found.branches.get(id).cloned().unwrap_or_default();
            if let Some((created, modified)) = dates.get(path.as_str()) {
                entry.created = Some((*created).to_string());
                entry.modified = Some((*modified).to_string());
            }
            entry.issues = open
                .iter()
                .filter(|i| names(i, &entry))
                .map(|i| IssueLink {
                    number: i.number,
                    title: i.title.clone(),
                    bug: is_bug(i),
                })
                .collect();
            entry.status = if entry.issues.iter().any(|i| i.bug) {
                Status::Bug
            } else if entry.branches.iter().any(|b| b == RELEASED) {
                Status::Available
            } else {
                Status::Building
            };
            entries.push(entry);
        }
    }

    let linked: BTreeSet<u64> = entries
        .iter()
        .flat_map(|e| e.issues.iter().map(|i| i.number))
        .collect();
    entries.extend(
        open.iter()
            .filter(|i| !is_bug(i) && !linked.contains(&i.number))
            .filter_map(|i| planned(i)),
    );
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    Catalogue {
        entries,
        branches,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{FileDates, RefUnits, UnitFile};

    fn file(path: &str, text: &str) -> UnitFile {
        UnitFile {
            path: path.to_string(),
            text: text.to_string(),
        }
    }

    const CAP: &str = r#"{"capability":"campagne.niveauDuGroupe","system":"campagne","version":"0.1.0","description":"Party level.","implements":"NiveauDuGroupe@1","input":{"campagneId":"ID"},"output":"NiveauDuGroupe@1","reads":["queries/niveaux.graphql"]}"#;
    const DC: &str = r#"{"dataCapability":"campagne.ajouterPj","owner":"dataguard","version":1,"effect":"insert","target":{"aggregate":"PJ"},"touches":[],"payload":{"nom":"string"},"mode":"relative","invariants":["pc-name-required","pc-ghost"],"permissions":[],"callableBy":["campagne"],"idempotencyKey":"required"}"#;
    const UI: &str = r#"{"microUi":"Pjs","system":"campagne","description":"PCs","needs":["campagne.niveauDuGroupe"],"actions":["campagne.ajouterPj"],"props":{"campagneId":"ID"}}"#;
    const AGG: &str = r#"{"aggregate":"PJ","invariants":[{"id":"pc-name-required","rule":"The name is not empty."}]}"#;
    const CONCEPT: &str =
        r#"{"concept":"NiveauDuGroupe","version":1,"definition":"The mean of the levels."}"#;

    fn reading() -> UnitsReading {
        UnitsReading {
            refs: vec![
                RefUnits {
                    git_ref: "feat/dc".to_string(),
                    files: vec![
                        file(
                            "crates/dataguard/data-capabilities/ajouter-pj/data-capability.json",
                            DC,
                        ),
                        file("crates/dataguard/aggregates/PJ/aggregate.json", AGG),
                        file("apps/campagne/pjs/micro-ui.json", UI),
                    ],
                },
                RefUnits {
                    git_ref: RELEASED.to_string(),
                    files: vec![
                        file("crates/campagne/capabilities/niveau/capability.json", CAP),
                        file("concepts/NiveauDuGroupe/v1/concept.json", CONCEPT),
                        file("broken/capability.json", "{not json"),
                    ],
                },
            ],
            dates: vec![FileDates {
                path: "crates/campagne/capabilities/niveau/capability.json".to_string(),
                created: "2026-09-01T10:00:00Z".to_string(),
                modified: "2026-09-03T10:00:00Z".to_string(),
            }],
        }
    }

    fn issue(number: u64, title: &str, body: &str, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: title.to_string(),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: body.to_string(),
            blocked_by: Vec::new(),
        }
    }

    fn find<'a>(c: &'a Catalogue, id: &str) -> &'a Entry {
        c.entries.iter().find(|e| e.id == id).expect("entry listed")
    }

    #[test]
    fn a_unit_on_main_is_available_and_one_elsewhere_is_building() {
        let c = catalogue(&reading(), "main_agent", &[]);
        assert_eq!(
            find(&c, "campagne.niveauDuGroupe").status,
            Status::Available
        );
        assert_eq!(find(&c, "campagne.ajouterPj").status, Status::Building);
        assert_eq!(c.branches, vec!["main", "feat/dc"]);
        assert_eq!(c.warnings.len(), 1, "{:?}", c.warnings);
    }

    #[test]
    fn a_capability_carries_its_contract_its_callers_its_concept_and_dates() {
        let c = catalogue(&reading(), "main_agent", &[]);
        let cap = find(&c, "campagne.niveauDuGroupe");
        assert_eq!(
            cap.contract.input.get("campagneId").map(String::as_str),
            Some("ID")
        );
        assert_eq!(cap.contract.reads, vec!["queries/niveaux.graphql"]);
        assert_eq!(cap.used_by.len(), 1);
        assert_eq!(cap.used_by[0].how, How::Reads);
        assert_eq!(cap.rules[0].text, "The mean of the levels.");
        assert_eq!(cap.created.as_deref(), Some("2026-09-01T10:00:00Z"));
        assert_eq!(cap.modified.as_deref(), Some("2026-09-03T10:00:00Z"));
    }

    #[test]
    fn a_data_capability_carries_its_aggregate_s_invariants_by_id() {
        let c = catalogue(&reading(), "main_agent", &[]);
        let dc = find(&c, "campagne.ajouterPj");
        assert_eq!(dc.kind, Kind::DataCapability);
        assert_eq!(dc.rules[0].text, "The name is not empty.");
        assert!(dc.rules[1].text.contains("not declared"));
        assert_eq!(dc.used_by[0].how, How::Triggers);
        assert_eq!(dc.role, "insert a PJ");
    }

    #[test]
    fn an_open_bug_naming_a_unit_turns_it_red() {
        let bugs = [issue(7, "niveauDuGroupe rounds .5 down", "", &["bug"])];
        let c = catalogue(&reading(), "main_agent", &bugs);
        let cap = find(&c, "campagne.niveauDuGroupe");
        assert_eq!(cap.status, Status::Bug);
        assert_eq!(cap.issues[0].number, 7);
    }

    #[test]
    fn an_open_task_declaring_a_capability_no_branch_has_is_planned() {
        let body = "## Architecture\nunit: capability\nsystem: campagne\n";
        let c = catalogue(
            &reading(),
            "main_agent",
            &[issue(9, "List the encounters", body, &["harness:agent"])],
        );
        let plan = find(&c, "#9");
        assert_eq!(plan.status, Status::Planned);
        assert_eq!(plan.system, "campagne");
        assert_eq!(plan.kind, Kind::Capability);
    }

    #[test]
    fn a_task_that_names_an_existing_unit_is_not_planned_twice() {
        let body = "Builds `campagne.ajouterPj`.\n## Architecture\nunit: data-capability\nsystem: campagne\n";
        let c = catalogue(&reading(), "main_agent", &[issue(4, "Add a PC", body, &[])]);
        assert!(c.entries.iter().all(|e| e.id != "#4"));
        assert_eq!(find(&c, "campagne.ajouterPj").issues[0].number, 4);
    }
}
