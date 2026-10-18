//! The design dossier the factory's creation leaves in the product, read
//! back for the planner and `split`: what was decided, so that a plan and
//! its tasks copy it rather than guess it.
//!
//! The files, all under the checkout's root:
//!
//! - `mock/proof-of-concept.json` — the record: every micro-frontend with
//!   what it reads (`needs`), triggers (`actions`) and its fixtures, every
//!   write with its effect and the fields it touches;
//! - `mock/data/aggregates/<Aggregate>.aggregate.json` — each aggregate as
//!   contract I has it, its invariants;
//! - `rules.json` — the rules (contract P), their criteria.
//!
//! A file missing or unreadable reads as nothing: a roadmap item written by
//! hand has no dossier, and its plan is made without one, as before.

use std::path::Path;

use harness_core::ports::shell::disk::Disk;
use serde_json::Value;

/// The record the creation leaves beside the mock-up.
pub const RECORD: &str = "mock/proof-of-concept.json";
/// Where the drafted aggregates lie, one file each.
pub const AGGREGATES_DIR: &str = "mock/data/aggregates";
/// The rules (contract P).
pub const RULES: &str = "rules.json";
/// The ports (contract Q).
pub const PORTS: &str = "ports.json";

/// How long the dossier's block may run, in characters: past it, the lines
/// stop and the block says so.
pub const BUDGET: usize = 12_000;

/// One micro-frontend as the creation cut it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MicroUi {
    /// Its name, `PascalCase`.
    pub name: String,
    /// Its system.
    pub system: String,
    /// The Capabilities it reads.
    pub needs: Vec<String>,
    /// The `DataCapabilities` it triggers.
    pub actions: Vec<String>,
    /// Its fixtures, one per Capability it needs.
    pub fixtures: Vec<String>,
    /// Its fragment under `mock/`.
    pub fragment: String,
}

/// One write as the creation decided it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Write {
    /// Its identifier, `<system>.<verbThing>`.
    pub id: String,
    /// `insert`, `update`, `delete`, `upsert`; empty when not judged.
    pub effect: String,
    /// The fields it changes, `table.field`.
    pub touches: Vec<String>,
}

/// One aggregate as drafted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Aggregate {
    /// Its name.
    pub name: String,
    /// Its fields, by key.
    pub fields: Vec<String>,
    /// Its invariants: `id — rule`.
    pub invariants: Vec<String>,
}

/// One rule as `rules.json` holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rule {
    /// `R3`.
    pub id: String,
    /// `journey` or `business`.
    pub family: String,
    /// What it is, in a few words.
    pub title: String,
    /// Its criterion, `given …, when …, then …`.
    pub criterion: String,
    /// The micro-frontends that carry it.
    pub components: Vec<String>,
}

/// One port as `ports.json` holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Port {
    /// Its business function.
    pub id: String,
    /// Its provider.
    pub provider: String,
    /// Whether the provider keeps state.
    pub stateful: bool,
    /// Its adapter's crate.
    pub adapter: String,
    /// The units that go through it.
    pub used_by: Vec<String>,
}

/// Everything the creation decided, as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dossier {
    /// The micro-frontends.
    pub micro_uis: Vec<MicroUi>,
    /// The writes.
    pub writes: Vec<Write>,
    /// The aggregates.
    pub aggregates: Vec<Aggregate>,
    /// The rules.
    pub rules: Vec<Rule>,
    /// The ports.
    pub ports: Vec<Port>,
}

/// What part of the dossier a block shows: all of it when every list is
/// empty, else what touches one of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Systems (bounded contexts).
    pub systems: Vec<String>,
    /// Aggregates.
    pub aggregates: Vec<String>,
    /// Rules, `R3`.
    pub rules: Vec<String>,
}

impl Filter {
    /// A milestone's slice of the dossier, from its body: the `systems:` and
    /// `aggregates:` lines of its `## Architecture`, the rules of its
    /// `## Rules`.
    #[must_use]
    pub fn of_milestone(body: &str) -> Self {
        let found = crate::common::sections::parse(body);
        let list = |key: &str| -> Vec<String> {
            found
                .get(crate::common::sections::ARCHITECTURE)
                .map(|text| {
                    text.lines()
                        .filter_map(|line| line.trim().strip_prefix(key))
                        .flat_map(|value| value.split(','))
                        .map(|v| v.trim().trim_matches('`').to_string())
                        .filter(|v| !v.is_empty() && v != "-")
                        .collect()
                })
                .unwrap_or_default()
        };
        let rules = found
            .get(crate::common::sections::RULES)
            .map(|text| rule_ids(text))
            .unwrap_or_default();
        Self {
            systems: list("systems:"),
            aggregates: list("aggregates:"),
            rules,
        }
    }

    const fn is_all(&self) -> bool {
        self.systems.is_empty() && self.aggregates.is_empty() && self.rules.is_empty()
    }
}

/// The rule ids a text names, `R` and a number, each once, in order.
#[must_use]
pub fn rule_ids(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        let Some(digits) = word.strip_prefix('R') else {
            continue;
        };
        if !digits.is_empty()
            && !digits.starts_with('0')
            && digits.bytes().all(|b| b.is_ascii_digit())
            && !found.iter().any(|f| f == word)
        {
            found.push(word.to_string());
        }
    }
    found
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

impl Dossier {
    /// The dossier under a checkout's root; what is missing reads as empty.
    #[must_use]
    pub fn read(disk: &dyn Disk, root: &Path) -> Self {
        let json = |path: &Path| -> Option<Value> {
            disk.read_to_string(path)
                .and_then(|t| serde_json::from_str(&t).ok())
        };
        let mut dossier = Self::default();
        if let Some(record) = json(&root.join(RECORD)) {
            for c in record
                .get("components")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                dossier.micro_uis.push(MicroUi {
                    name: text(c.get("name")),
                    system: text(c.get("system")),
                    needs: strings(c.get("needs")),
                    actions: strings(c.get("actions")),
                    fixtures: strings(c.get("fixtures")),
                    fragment: text(c.get("fragment")),
                });
            }
            for w in record
                .get("writes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                dossier.writes.push(Write {
                    id: text(w.get("dataCapability")),
                    effect: text(w.get("effect")),
                    touches: strings(w.get("touches")),
                });
            }
        }
        let dir = root.join(AGGREGATES_DIR);
        for name in disk.file_names(&dir) {
            if !name.ends_with(".aggregate.json") {
                continue;
            }
            let Some(a) = json(&dir.join(&name)) else {
                continue;
            };
            dossier.aggregates.push(Aggregate {
                name: text(a.get("aggregate")),
                fields: a
                    .get("fields")
                    .and_then(Value::as_object)
                    .map(|f| f.keys().cloned().collect())
                    .unwrap_or_default(),
                invariants: a
                    .get("invariants")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|i| format!("{} — {}", text(i.get("id")), text(i.get("rule"))))
                    .collect(),
            });
        }
        if let Some(rules) = json(&root.join(RULES)) {
            for r in rules
                .get("rules")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let criterion = r.get("criterion");
                let part = |key: &str| text(criterion.and_then(|c| c.get(key)));
                dossier.rules.push(Rule {
                    id: text(r.get("id")),
                    family: text(r.get("family")),
                    title: text(r.get("title")),
                    criterion: format!(
                        "given {}, when {}, then {}",
                        part("given"),
                        part("when"),
                        part("then")
                    ),
                    components: strings(r.get("components")),
                });
            }
        }
        if let Some(ports) = json(&root.join(PORTS)) {
            for p in ports
                .get("ports")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                dossier.ports.push(Port {
                    id: text(p.get("id")),
                    provider: text(p.get("provider")),
                    stateful: p.get("stateful").and_then(Value::as_bool) == Some(true),
                    adapter: text(p.get("adapter")),
                    used_by: strings(p.get("usedBy")),
                });
            }
        }
        dossier
    }

    /// Nothing was read.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.micro_uis.is_empty()
            && self.writes.is_empty()
            && self.aggregates.is_empty()
            && self.rules.is_empty()
    }

    /// The ports the units kept by `in_system` use — all of them when
    /// `all` — under their heading; nothing when none.
    fn port_lines(&self, in_system: impl Fn(&str) -> bool, all: bool) -> Vec<String> {
        let ports: Vec<&Port> = self
            .ports
            .iter()
            .filter(|p| all || p.used_by.iter().any(|u| in_system(u)))
            .collect();
        if ports.is_empty() {
            return Vec::new();
        }
        let mut lines = vec![
            "Ports (ports.json, contract Q: everything outside the product goes through one):"
                .to_string(),
        ];
        for p in ports {
            lines.push(format!(
                "- {}: {}{}, adapter {}; used by {}",
                p.id,
                p.provider,
                if p.stateful {
                    " (keeps state: DataCapabilities only)"
                } else {
                    ""
                },
                p.adapter,
                or_none(&p.used_by)
            ));
        }
        lines
    }

    /// The micro-frontends as the plan's graph reads them: what each reads
    /// and triggers.
    #[must_use]
    pub fn components(&self) -> Vec<harness_architecture::graph::Component> {
        self.micro_uis
            .iter()
            .map(|ui| harness_architecture::graph::Component {
                name: ui.name.clone(),
                system: ui.system.clone(),
                needs: ui.needs.clone(),
                actions: ui.actions.clone(),
            })
            .collect()
    }

    /// The ids of every write, as the graph's fallback reads a milestone
    /// written before the plan named its commands.
    #[must_use]
    pub fn write_ids(&self) -> Vec<String> {
        self.writes.iter().map(|w| w.id.clone()).collect()
    }

    /// The dossier as a prompt reads it — one line per micro-frontend, write,
    /// aggregate and rule — kept to what `filter` names, cut at [`BUDGET`].
    #[must_use]
    pub fn block(&self, filter: &Filter) -> String {
        if self.is_empty() && self.ports.is_empty() {
            return "No design dossier: this roadmap item was written by hand.".to_string();
        }
        let all = filter.is_all();
        let in_system = |id_or_system: &str| {
            all || filter
                .systems
                .iter()
                .any(|s| id_or_system == s || id_or_system.starts_with(&format!("{s}.")))
        };
        let mut lines: Vec<String> = Vec::new();
        let micro_uis: Vec<&MicroUi> = self
            .micro_uis
            .iter()
            .filter(|m| in_system(&m.system))
            .collect();
        if !micro_uis.is_empty() {
            lines.push("Micro-frontends (mock/components/, contract D once built):".to_string());
            for m in micro_uis {
                lines.push(format!(
                    "- {} ({}): needs {}; actions {}; fixtures {}; fragment mock/{}",
                    m.name,
                    m.system,
                    or_none(&m.needs),
                    or_none(&m.actions),
                    or_none(&m.fixtures),
                    m.fragment
                ));
            }
        }
        let writes: Vec<&Write> = self.writes.iter().filter(|w| in_system(&w.id)).collect();
        if !writes.is_empty() {
            lines.push("Writes (contract F: effect, touches):".to_string());
            for w in writes {
                lines.push(format!(
                    "- {}: {}, touches {}",
                    w.id,
                    if w.effect.is_empty() {
                        "effect not judged"
                    } else {
                        &w.effect
                    },
                    or_none(&w.touches)
                ));
            }
        }
        let aggregates: Vec<&Aggregate> = self
            .aggregates
            .iter()
            .filter(|a| {
                all || filter
                    .aggregates
                    .iter()
                    .any(|f| f.eq_ignore_ascii_case(&a.name))
            })
            .collect();
        if !aggregates.is_empty() {
            lines.push(format!(
                "Aggregates (drafted under {AGGREGATES_DIR}/, contract I):"
            ));
            for a in aggregates {
                lines.push(format!(
                    "- {}: fields {}; invariants {}",
                    a.name,
                    or_none(&a.fields),
                    or_none(&a.invariants)
                ));
            }
        }
        let rules: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| all || filter.rules.contains(&r.id))
            .collect();
        lines.extend(self.port_lines(|u| in_system(u), all));
        if !rules.is_empty() {
            lines.push("Rules (rules.json, their tests already under tests/rules/):".to_string());
            for r in rules {
                lines.push(format!(
                    "- {} ({}) {}: {}; carried by {}",
                    r.id,
                    r.family,
                    r.title,
                    r.criterion,
                    or_none(&r.components)
                ));
            }
        }
        let mut block = String::new();
        for line in lines {
            if block.chars().count() + line.chars().count() > BUDGET {
                block.push_str("(the dossier goes on: read the files named above)\n");
                break;
            }
            block.push_str(&line);
            block.push('\n');
        }
        block
    }
}

fn or_none(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_disk::FakeDisk;
    use std::path::PathBuf;

    fn disk() -> FakeDisk {
        let root = PathBuf::from("/repo");
        let mut disk = FakeDisk::default();
        disk.existing.insert(
            root.join(RECORD),
            r#"{"components":[
                {"name":"Pjs","system":"campagne","needs":["campagne.listerPjs"],"actions":["campagne.ajouterPj"],"fixtures":["fixtures/campagne.listerPjs.json"],"fragment":"components/Pjs.html"},
                {"name":"Map","system":"monde","needs":["monde.lireCarte"],"actions":[]}],
              "writes":[{"dataCapability":"campagne.ajouterPj","effect":"insert","touches":[]},
                        {"dataCapability":"monde.renommer","effect":null,"touches":["region.name"]}]}"#
                .to_string(),
        );
        disk.existing.insert(
            root.join(AGGREGATES_DIR).join("PJ.aggregate.json"),
            r#"{"aggregate":"PJ","fields":{"name":{}},"invariants":[{"id":"nom-unique","rule":"one name"}]}"#
                .to_string(),
        );
        disk.existing.insert(
            root.join(AGGREGATES_DIR).join("README.md"),
            "not a draft".to_string(),
        );
        disk.existing.insert(
            root.join(RULES),
            r#"{"version":1,"rules":[
                {"id":"R3","family":"business","title":"Unique names","criterion":{"given":"a PJ","when":"another takes its name","then":"refused"},"components":["Pjs"]},
                {"id":"R4","family":"journey","title":"See the map","criterion":{"given":"g","when":"w","then":"t"}}]}"#
                .to_string(),
        );
        disk
    }

    #[test]
    fn the_dossier_is_read_whole_and_a_missing_one_reads_empty() {
        let read = Dossier::read(&disk(), Path::new("/repo"));
        assert_eq!(read.micro_uis.len(), 2);
        assert_eq!(
            read.micro_uis[0].fixtures,
            ["fixtures/campagne.listerPjs.json"]
        );
        assert_eq!(read.writes[1].effect, "", "not judged");
        assert_eq!(read.writes[1].touches, ["region.name"]);
        assert_eq!(read.aggregates.len(), 1, "only the drafts");
        assert_eq!(read.aggregates[0].invariants, ["nom-unique — one name"]);
        assert_eq!(
            read.rules[0].criterion,
            "given a PJ, when another takes its name, then refused"
        );
        let none = Dossier::read(&FakeDisk::default(), Path::new("/repo"));
        assert!(none.is_empty());
        assert!(none.block(&Filter::default()).contains("written by hand"));
    }

    #[test]
    fn a_milestone_sees_its_systems_aggregates_and_rules_only() {
        let read = Dossier::read(&disk(), Path::new("/repo"));
        let body = "Goal.\n\n## Architecture\n\nsystems: campagne\nconcepts: -\naggregates: `pj`\nside: harness:write-side\n\n## Rules\n\nR3";
        let filter = Filter::of_milestone(body);
        assert_eq!(filter.systems, ["campagne"]);
        assert_eq!(filter.aggregates, ["pj"]);
        assert_eq!(filter.rules, ["R3"]);
        let block = read.block(&filter);
        assert!(
            block.contains("- Pjs (campagne): needs campagne.listerPjs"),
            "{block}"
        );
        assert!(!block.contains("Map (monde)"), "{block}");
        assert!(block.contains("- campagne.ajouterPj: insert"), "{block}");
        assert!(!block.contains("monde.renommer"), "{block}");
        assert!(block.contains("- PJ: fields name"), "{block}");
        assert!(block.contains("- R3 (business) Unique names"), "{block}");
        assert!(!block.contains("R4"), "{block}");
        let whole = read.block(&Filter::default());
        assert!(
            whole.contains("Map (monde)")
                && whole.contains("R4")
                && whole.contains("effect not judged")
        );
    }

    #[test]
    fn the_ports_are_read_and_a_milestone_sees_those_its_units_use() {
        let mut disk = disk();
        disk.existing.insert(
            PathBuf::from("/repo").join(PORTS),
            r#"{"version":1,"ports":[
                {"id":"database","provider":"PostgreSQL 16","stateful":true,"adapter":"crates/data","usedBy":[]},
                {"id":"payments","provider":"Stripe","stateful":true,"adapter":"crates/ports/payments/stripe","usedBy":["campagne.ajouterPj"]},
                {"id":"geocoding","provider":"Mapbox","stateful":false,"adapter":"crates/ports/geocoding/mapbox","usedBy":["monde.lireCarte"]}]}"#
                .to_string(),
        );
        let read = Dossier::read(&disk, Path::new("/repo"));
        assert_eq!(read.ports.len(), 3);
        let block = read.block(&Filter {
            systems: vec!["campagne".to_string()],
            ..Filter::default()
        });
        assert!(
            block.contains("- payments: Stripe (keeps state: DataCapabilities only)"),
            "{block}"
        );
        assert!(!block.contains("geocoding"), "{block}");
        let only_ports = Dossier {
            ports: read.ports,
            ..Dossier::default()
        };
        assert!(
            !only_ports
                .block(&Filter::default())
                .contains("written by hand")
        );
    }

    #[test]
    fn rule_ids_are_read_off_any_text_each_once() {
        assert_eq!(rule_ids("R3, R5 — R3; R0 r4 R12x R40"), ["R3", "R5", "R40"]);
    }

    #[test]
    fn the_block_stops_at_its_budget() {
        let mut big = Dossier::default();
        for n in 0..2_000 {
            big.rules.push(Rule {
                id: format!("R{}", n + 1),
                title: "a rule with a long enough title".to_string(),
                ..Rule::default()
            });
        }
        let block = big.block(&Filter::default());
        assert!(block.chars().count() <= BUDGET + 60);
        assert!(block.ends_with("(the dossier goes on: read the files named above)\n"));
    }
}
