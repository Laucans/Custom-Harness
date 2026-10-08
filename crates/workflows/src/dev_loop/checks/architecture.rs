//! Does the checkout still follow the agent-native architecture once `code`
//! is done? Two gates on the round, both deterministic and both local.
//!
//! [`ArchitectureHolds`] reads the manifests the architecture asks for and
//! checks the rules a diff is reviewed against: a Capability depends on no
//! Capability and on nothing of the write side; its manifest names its
//! system and invokes nothing; a Micro-UI needs Capabilities that exist, in
//! its own system, and triggers `DataCapabilities` that exist; a `DataCapability`
//! belongs to the `DataGuard` and declares its effect; and the unit the task
//! declared under `## Architecture` is actually there, on the side it said.
//!
//! [`ConceptsDocumented`] checks that every Concept a Capability implements
//! (`implements: Risk@3`) is documented — `concepts/Risk/v3/concept.json`,
//! naming that concept and that version. Meaning is what is shared between
//! systems, and a Concept nobody wrote down is a copy nobody can check.
//!
//! The CI gates of the repository (`.github/workflows/gates.yml`) run the
//! same rules; these are the loop's own reading, so a round that drifted
//! stops here with the list rather than at the next pull request's checks.
//! A checkout without `contracts/` is not under the architecture: both gates
//! say so and let the round through.
//!
//! The reading is pure over an in-memory [`Tree`]; only [`Tree::read`]
//! touches the disk, through the `Disk` port.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};
use harness_core::ports::shell::disk::Disk;
use serde_json::Value;

use crate::common::architecture::{Declaration, Effect, Side, Unit};
use crate::common::sections;
use crate::dev_loop::data::state::Loop;

/// One manifest, where it was read and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The path, relative to the checkout.
    pub path: String,
    /// Its JSON, or `None` when the file is not JSON at all.
    pub json: Option<Value>,
}

/// A Capability crate: its manifest and its `Cargo.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityCrate {
    /// `crates/<system>/capabilities/<name>`.
    pub dir: String,
    /// The system folder it sits under.
    pub system: String,
    /// `capability.json`, if present.
    pub manifest: Option<Manifest>,
    /// `Cargo.toml`, if present.
    pub cargo_toml: Option<String>,
}

/// What the gates read of the checkout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    /// The repository carries `contracts/`: it is under the architecture.
    pub under_architecture: bool,
    /// Every Capability crate.
    pub capabilities: Vec<CapabilityCrate>,
    /// Every `micro-ui.json`, with the system folder it sits under.
    pub micro_uis: Vec<(String, Manifest)>,
    /// Every `data-capability.json`.
    pub data_capabilities: Vec<Manifest>,
    /// Every `aggregate.json`.
    pub aggregates: Vec<Manifest>,
    /// Every documented Concept, as `(name, version)`.
    pub concepts: Vec<(String, u64)>,
    /// `queries/registry.json` exists.
    pub has_query_registry: bool,
    /// The folders under `crates/`: the systems that have Rust code.
    pub systems: Vec<String>,
}

impl Tree {
    /// Reads the checkout at `root`. Nothing here fails: an unreadable or
    /// absent file is a manifest that is not there, which the rules report.
    #[must_use]
    pub fn read(disk: &dyn Disk, root: &Path) -> Self {
        let text = |rel: &str| disk.read_to_string(&root.join(rel));
        let manifest = |rel: &str| {
            text(rel).map(|content| Manifest {
                path: rel.to_string(),
                json: serde_json::from_str(&content).ok(),
            })
        };
        let mut tree = Self {
            under_architecture: disk.exists(&root.join("contracts")),
            has_query_registry: disk.exists(&root.join("queries/registry.json")),
            ..Self::default()
        };
        for system in disk.dir_names(&root.join("crates")) {
            tree.systems.push(system.clone());
            for name in disk.dir_names(&root.join("crates").join(&system).join("capabilities")) {
                let dir = format!("crates/{system}/capabilities/{name}");
                tree.capabilities.push(CapabilityCrate {
                    manifest: manifest(&format!("{dir}/capability.json")),
                    cargo_toml: text(&format!("{dir}/Cargo.toml")),
                    system: system.clone(),
                    dir,
                });
            }
        }
        for system in disk.dir_names(&root.join("apps")) {
            for name in disk.dir_names(&root.join("apps").join(&system)) {
                if let Some(found) = manifest(&format!("apps/{system}/{name}/micro-ui.json")) {
                    tree.micro_uis.push((system.clone(), found));
                }
            }
        }
        for name in disk.dir_names(&root.join("crates/dataguard/data-capabilities")) {
            if let Some(found) = manifest(&format!(
                "crates/dataguard/data-capabilities/{name}/data-capability.json"
            )) {
                tree.data_capabilities.push(found);
            }
        }
        for name in disk.dir_names(&root.join("crates/dataguard/aggregates")) {
            if let Some(found) = manifest(&format!(
                "crates/dataguard/aggregates/{name}/aggregate.json"
            )) {
                tree.aggregates.push(found);
            }
        }
        for concept in disk.dir_names(&root.join("concepts")) {
            for version in disk.dir_names(&root.join("concepts").join(&concept)) {
                let Some(number) = version
                    .strip_prefix('v')
                    .and_then(|n| n.parse::<u64>().ok())
                else {
                    continue;
                };
                let rel = format!("concepts/{concept}/{version}/concept.json");
                let documented = text(&rel)
                    .and_then(|content| serde_json::from_str::<Value>(&content).ok())
                    .is_some_and(|json| {
                        str_of(&json, "concept") == Some(concept.as_str())
                            && json.get("version").and_then(Value::as_u64) == Some(number)
                    });
                if documented {
                    tree.concepts.push((concept.clone(), number));
                }
            }
        }
        tree
    }
}

fn str_of<'a>(json: &'a Value, key: &str) -> Option<&'a str> {
    json.get(key).and_then(Value::as_str)
}

fn strings_of(json: &Value, key: &str) -> Vec<String> {
    json.get(key)
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

/// `<system>.<name>` → `<system>`.
fn system_of_id(id: &str) -> Option<&str> {
    id.split_once('.').map(|(system, _)| system)
}

/// The side a `DataCapability` manifest declares.
fn manifest_side(json: &Value) -> Side {
    let effect = str_of(json, "effect").and_then(Effect::parse);
    let touches = strings_of(json, "touches");
    if effect == Some(Effect::Insert) && touches.is_empty() {
        Side::Read
    } else {
        Side::Write
    }
}

/// Every rule the tree breaks, one line each. Empty: the architecture holds.
// One rule per block, in the order the architecture states them; splitting
// them into functions would hide the list this gate is.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn violations(tree: &Tree, task: Option<&Declaration>) -> Vec<String> {
    let mut found = Vec::new();
    let capability_ids: Vec<&str> = tree
        .capabilities
        .iter()
        .filter_map(|c| c.manifest.as_ref()?.json.as_ref())
        .filter_map(|json| str_of(json, "capability"))
        .collect();
    let data_capability_ids: Vec<&str> = tree
        .data_capabilities
        .iter()
        .filter_map(|m| m.json.as_ref())
        .filter_map(|json| str_of(json, "dataCapability"))
        .collect();

    for crate_ in &tree.capabilities {
        match &crate_.cargo_toml {
            None => found.push(format!(
                "{}: no Cargo.toml — a Capability is a crate",
                crate_.dir
            )),
            Some(toml) => {
                let offending: Vec<&str> = toml
                    .lines()
                    .filter(|line| !line.trim_start().starts_with('#'))
                    .filter(|line| {
                        ["capabilities/", "dataguard", "dataqueue", "resolver"]
                            .iter()
                            .any(|needle| line.contains(needle))
                    })
                    .collect();
                if !offending.is_empty() {
                    found.push(format!(
                        "{}/Cargo.toml depends on another Capability or on the write side: {}",
                        crate_.dir,
                        offending.join(" | ").trim()
                    ));
                }
            }
        }
        match &crate_.manifest {
            None => found.push(format!("{}: no capability.json (contract B)", crate_.dir)),
            Some(Manifest { json: None, path }) => found.push(format!("{path} is not JSON")),
            Some(Manifest {
                json: Some(json),
                path,
            }) => {
                for forbidden in ["invokes", "needs", "actions", "writes"] {
                    if json.get(forbidden).is_some() {
                        found.push(format!(
                            "{path} has a `{forbidden}` field — a Capability only reads"
                        ));
                    }
                }
                match str_of(json, "capability") {
                    Some(id) if system_of_id(id) != Some(crate_.system.as_str()) => {
                        found.push(format!(
                            "{path}: `{id}` is not in the `{}` namespace its folder says",
                            crate_.system
                        ));
                    }
                    None => found.push(format!("{path} names no `capability`")),
                    _ => {}
                }
                if str_of(json, "system") != Some(crate_.system.as_str()) {
                    found.push(format!(
                        "{path}: `system` is not `{}`, the folder it sits under",
                        crate_.system
                    ));
                }
                if strings_of(json, "reads").is_empty() {
                    found.push(format!(
                        "{path} declares no persisted query under `reads` — every read is declared"
                    ));
                }
            }
        }
    }

    for (system, manifest) in &tree.micro_uis {
        let Some(json) = &manifest.json else {
            found.push(format!("{} is not JSON", manifest.path));
            continue;
        };
        for need in strings_of(json, "needs") {
            if !capability_ids.contains(&need.as_str()) {
                found.push(format!(
                    "{}: needs `{need}`, a Capability no manifest declares",
                    manifest.path
                ));
            } else if system_of_id(&need) != Some(system.as_str()) {
                found.push(format!(
                    "{}: needs `{need}`, outside its own system `{system}`",
                    manifest.path
                ));
            }
        }
        for action in strings_of(json, "actions") {
            if !data_capability_ids.contains(&action.as_str()) {
                found.push(format!(
                    "{}: triggers `{action}`, a DataCapability no manifest declares",
                    manifest.path
                ));
            }
        }
    }

    for manifest in &tree.data_capabilities {
        let Some(json) = &manifest.json else {
            found.push(format!("{} is not JSON", manifest.path));
            continue;
        };
        if str_of(json, "owner") != Some("dataguard") {
            found.push(format!(
                "{}: `owner` is not `dataguard` — a DataCapability belongs to the DataGuard",
                manifest.path
            ));
        }
        if str_of(json, "effect").and_then(Effect::parse).is_none() {
            found.push(format!(
                "{}: `effect` is not one of insert, update, delete, upsert",
                manifest.path
            ));
        }
    }

    if let Some(task) = task {
        found.extend(unit_present(tree, task));
    }
    found
}

/// The unit the task declared is there, on the side it declared.
fn unit_present(tree: &Tree, task: &Declaration) -> Vec<String> {
    let system = task.system.as_str();
    let missing = |what: &str| {
        vec![format!(
            "the task declared a {what} and the checkout has none"
        )]
    };
    match task.unit {
        Unit::Capability => {
            let found = tree.capabilities.iter().any(|c| {
                c.manifest
                    .as_ref()
                    .and_then(|m| m.json.as_ref())
                    .is_some_and(|json| {
                        (system.is_empty() || str_of(json, "system") == Some(system))
                            && task
                                .concept
                                .as_deref()
                                .is_none_or(|concept| str_of(json, "implements") == Some(concept))
                    })
            });
            if found {
                Vec::new()
            } else {
                missing(&format!(
                    "Capability of system `{system}`{}",
                    task.concept
                        .as_deref()
                        .map_or_else(String::new, |c| format!(" implementing `{c}`"))
                ))
            }
        }
        Unit::MicroUi => {
            let found = tree
                .micro_uis
                .iter()
                .any(|(s, _)| system.is_empty() || s == system);
            if found {
                Vec::new()
            } else {
                missing(&format!("Micro-UI of system `{system}`"))
            }
        }
        Unit::DataCapability => {
            let declared = task.side();
            let found = tree
                .data_capabilities
                .iter()
                .filter_map(|m| m.json.as_ref())
                .filter(|json| {
                    system.is_empty()
                        || str_of(json, "dataCapability")
                            .and_then(system_of_id)
                            .is_some_and(|s| s == system)
                })
                .collect::<Vec<_>>();
            if found.is_empty() {
                return missing(&format!("DataCapability of system `{system}`"));
            }
            if declared == Side::Read && found.iter().all(|json| manifest_side(json) == Side::Write)
            {
                return vec![format!(
                    "the task was declared a pure insert (read side) but every DataCapability of \
                     `{system}` mutates existing data — that is the write side, and a human reviews it"
                )];
            }
            Vec::new()
        }
        Unit::Concept => {
            let wanted = task.concept.as_deref().and_then(parse_concept_ref);
            match wanted {
                Some((name, version)) if !tree.concepts.contains(&(name.clone(), version)) => {
                    vec![format!(
                        "the task declared Concept `{name}@{version}` and concepts/{name}/v{version}/concept.json does not document it"
                    )]
                }
                _ => Vec::new(),
            }
        }
        Unit::Invariant if tree.aggregates.is_empty() => {
            missing("invariant, and no aggregate.json carries one")
        }
        Unit::PersistedQuery if !tree.has_query_registry => {
            missing("persisted query, and queries/registry.json does not exist")
        }
        Unit::Infrastructure if !system.is_empty() && !tree.systems.iter().any(|s| s == system) => {
            missing(&format!(
                "infrastructure unit of the system `{system}`, and crates/{system}/ does not \
                 exist — the stack builds it in Rust, there"
            ))
        }
        Unit::Invariant
        | Unit::PersistedQuery
        | Unit::Composition
        | Unit::Migration
        | Unit::Infrastructure => Vec::new(),
    }
}

/// `Risk@3` → `("Risk", 3)`.
fn parse_concept_ref(text: &str) -> Option<(String, u64)> {
    let (name, version) = text.trim().split_once('@')?;
    Some((name.to_string(), version.parse().ok()?))
}

/// Every Concept a Capability implements that no `concepts/` entry documents.
#[must_use]
pub fn undocumented_concepts(tree: &Tree) -> Vec<String> {
    let mut found = Vec::new();
    for crate_ in &tree.capabilities {
        let Some(json) = crate_.manifest.as_ref().and_then(|m| m.json.as_ref()) else {
            continue;
        };
        let Some(implements) = str_of(json, "implements") else {
            continue;
        };
        match parse_concept_ref(implements) {
            Some((name, version)) if tree.concepts.contains(&(name.clone(), version)) => {}
            Some((name, version)) => found.push(format!(
                "{} implements `{name}@{version}`, which concepts/{name}/v{version}/concept.json does not document",
                crate_.dir
            )),
            None => found.push(format!(
                "{} implements `{implements}`, which is not a `Concept@version` reference",
                crate_.dir
            )),
        }
    }
    found
}

/// The task's own declaration, read from its body.
fn declaration_of(ctx: &Context<Loop>) -> Option<Declaration> {
    sections::parse(&ctx.state.task.body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
}

/// The round kept the architecture: see the module.
pub struct ArchitectureHolds {
    /// What reads the checkout.
    pub disk: Rc<dyn Disk>,
    /// The checkout's root.
    pub root: PathBuf,
}

#[async_trait(?Send)]
impl Verification<Loop> for ArchitectureHolds {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let tree = Tree::read(self.disk.as_ref(), &self.root);
        if !tree.under_architecture {
            ctx.traces
                .say("architecture: no contracts/ in this checkout — not under the architecture, gate skipped");
            return Ok(Verdict::Continue);
        }
        let found = violations(&tree, declaration_of(ctx).as_ref());
        if found.is_empty() {
            ctx.traces.say(&format!(
                "architecture holds: {} Capability crate(s), {} Micro-UI(s), {} DataCapabilit(ies)",
                tree.capabilities.len(),
                tree.micro_uis.len(),
                tree.data_capabilities.len()
            ));
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "the architecture no longer holds after #{} — fix these before anything else builds \
             on them (the repository's gates check the same rules):\n  - {}",
            ctx.state.task.number,
            found.join("\n  - ")
        )))
    }
}

/// Every Concept a Capability implements is documented: see the module.
pub struct ConceptsDocumented {
    /// What reads the checkout.
    pub disk: Rc<dyn Disk>,
    /// The checkout's root.
    pub root: PathBuf,
}

#[async_trait(?Send)]
impl Verification<Loop> for ConceptsDocumented {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let tree = Tree::read(self.disk.as_ref(), &self.root);
        if !tree.under_architecture {
            return Ok(Verdict::Continue);
        }
        let found = undocumented_concepts(&tree);
        if found.is_empty() {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "a Capability implements a Concept nobody documented — write \
             concepts/<Concept>/v<n>/concept.json (contract A) and its fixtures first:\n  - {}",
            found.join("\n  - ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_disk::FakeDisk;
    use std::collections::HashMap;

    fn manifest(path: &str, json: &str) -> Manifest {
        Manifest {
            path: path.to_string(),
            json: serde_json::from_str(json).ok(),
        }
    }

    fn capability(system: &str, name: &str, json: &str, toml: &str) -> CapabilityCrate {
        let dir = format!("crates/{system}/capabilities/{name}");
        CapabilityCrate {
            manifest: Some(manifest(&format!("{dir}/capability.json"), json)),
            cargo_toml: Some(toml.to_string()),
            system: system.to_string(),
            dir,
        }
    }

    const RISK: &str = r#"{"capability":"credit.calculateRisk","system":"credit","version":"1.0.0","description":"d","implements":"Risk@3","reads":["q.graphql"]}"#;
    const CLEAN_TOML: &str =
        "[package]\nname = \"calculate-risk\"\n[dependencies]\nserde = \"1\"\n";

    fn sound() -> Tree {
        Tree {
            under_architecture: true,
            systems: vec!["credit".to_string(), "dataguard".to_string()],
            capabilities: vec![capability("credit", "calculate-risk", RISK, CLEAN_TOML)],
            micro_uis: vec![(
                "credit".to_string(),
                manifest(
                    "apps/credit/risk-badge/micro-ui.json",
                    r#"{"microUi":"RiskBadge","system":"credit","description":"d","needs":["credit.calculateRisk"],"actions":["credit.requestLimitChange"],"props":{}}"#,
                ),
            )],
            data_capabilities: vec![manifest(
                "crates/dataguard/data-capabilities/request-limit-change/data-capability.json",
                r#"{"dataCapability":"credit.requestLimitChange","owner":"dataguard","effect":"update","touches":["Account.creditLimit"]}"#,
            )],
            aggregates: Vec::new(),
            concepts: vec![("Risk".to_string(), 3)],
            has_query_registry: true,
        }
    }

    #[test]
    fn a_sound_tree_breaks_no_rule_and_documents_its_concepts() {
        let tree = sound();
        assert_eq!(violations(&tree, None), Vec::<String>::new());
        assert_eq!(undocumented_concepts(&tree), Vec::<String>::new());
    }

    #[test]
    fn a_capability_that_depends_on_a_capability_or_the_write_side_is_caught() {
        let mut tree = sound();
        tree.capabilities[0].cargo_toml = Some(
            "[dependencies]\n# dataguard mentioned in a comment is fine\nlate = { path = \"../late-payments\" }\nfoo = { path = \"../../capabilities/x\" }\n".to_string(),
        );
        let found = violations(&tree, None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("depends on another Capability"));
    }

    #[test]
    fn a_capability_manifest_that_invokes_or_leaves_its_system_is_caught() {
        let mut tree = sound();
        tree.capabilities[0].manifest = Some(manifest(
            "crates/credit/capabilities/calculate-risk/capability.json",
            r#"{"capability":"orders.calculateRisk","system":"orders","invokes":["x"],"reads":[]}"#,
        ));
        let found = violations(&tree, None);
        assert!(found.iter().any(|f| f.contains("`invokes` field")));
        assert!(
            found
                .iter()
                .any(|f| f.contains("not in the `credit` namespace"))
        );
        assert!(found.iter().any(|f| f.contains("`system` is not `credit`")));
        assert!(found.iter().any(|f| f.contains("no persisted query")));
    }

    #[test]
    fn a_micro_ui_needing_an_unknown_or_foreign_capability_is_caught() {
        let mut tree = sound();
        tree.micro_uis[0].1 = manifest(
            "apps/credit/risk-badge/micro-ui.json",
            r#"{"microUi":"RiskBadge","system":"credit","needs":["credit.nothing","orders.list"],"actions":["credit.unknown"]}"#,
        );
        tree.capabilities.push(capability(
            "orders",
            "list",
            r#"{"capability":"orders.list","system":"orders","reads":["q"]}"#,
            CLEAN_TOML,
        ));
        let found = violations(&tree, None);
        assert!(
            found
                .iter()
                .any(|f| f.contains("needs `credit.nothing`, a Capability no manifest declares"))
        );
        assert!(
            found
                .iter()
                .any(|f| f.contains("needs `orders.list`, outside its own system"))
        );
        assert!(
            found
                .iter()
                .any(|f| f.contains("triggers `credit.unknown`"))
        );
    }

    #[test]
    fn a_data_capability_outside_the_dataguard_or_with_no_effect_is_caught() {
        let mut tree = sound();
        tree.data_capabilities[0] = manifest(
            "crates/dataguard/data-capabilities/x/data-capability.json",
            r#"{"dataCapability":"credit.x","owner":"credit","effect":"patch"}"#,
        );
        let found = violations(&tree, None);
        assert!(
            found
                .iter()
                .any(|f| f.contains("`owner` is not `dataguard`"))
        );
        assert!(found.iter().any(|f| f.contains("`effect` is not one of")));
    }

    #[test]
    fn the_declared_unit_must_be_there_on_its_declared_side() {
        let tree = sound();
        let capability = Declaration {
            unit: Unit::Capability,
            system: "credit".to_string(),
            concept: Some("Risk@3".to_string()),
            effect: None,
            touches: Vec::new(),
        };
        assert_eq!(
            violations(&tree, Some(&capability)),
            [] as [std::string::String; 0]
        );
        let other = Declaration {
            concept: Some("Churn@1".to_string()),
            ..capability.clone()
        };
        assert!(violations(&tree, Some(&other))[0].contains("implementing `Churn@1`"));
        let pure_insert = Declaration {
            unit: Unit::DataCapability,
            effect: Some(Effect::Insert),
            concept: None,
            ..capability.clone()
        };
        let found = violations(&tree, Some(&pure_insert));
        assert!(found[0].contains("declared a pure insert"), "{found:?}");
        let concept = Declaration {
            unit: Unit::Concept,
            concept: Some("Risk@4".to_string()),
            ..capability.clone()
        };
        assert!(violations(&tree, Some(&concept))[0].contains("Risk@4"));
        let plumbing = Declaration {
            unit: Unit::Infrastructure,
            concept: None,
            system: "bestiary".to_string(),
            ..capability
        };
        let found = violations(&tree, Some(&plumbing));
        assert!(found[0].contains("crates/bestiary/"), "{found:?}");
        let mut with_crate = sound();
        with_crate.systems.push("bestiary".to_string());
        assert_eq!(
            violations(&with_crate, Some(&plumbing)),
            [] as [std::string::String; 0]
        );
    }

    #[test]
    fn a_concept_implemented_but_not_documented_is_named() {
        let mut tree = sound();
        tree.concepts.clear();
        let found = undocumented_concepts(&tree);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("concepts/Risk/v3/concept.json"));
    }

    #[test]
    fn the_tree_is_read_from_the_checkout_s_folders() {
        let root = PathBuf::from("/ws");
        let mut existing = HashMap::new();
        existing.insert(root.join("contracts/README.md"), String::new());
        existing.insert(
            root.join("crates/credit/capabilities/calculate-risk/capability.json"),
            RISK.to_string(),
        );
        existing.insert(
            root.join("crates/credit/capabilities/calculate-risk/Cargo.toml"),
            CLEAN_TOML.to_string(),
        );
        existing.insert(
            root.join("concepts/Risk/v3/concept.json"),
            r#"{"concept":"Risk","version":3}"#.to_string(),
        );
        existing.insert(
            root.join("concepts/Risk/v2/concept.json"),
            "not json".to_string(),
        );
        existing.insert(root.join("queries/registry.json"), "{}".to_string());
        let disk = FakeDisk {
            existing,
            ..FakeDisk::default()
        };
        let tree = Tree::read(&disk, &root);
        assert!(tree.under_architecture);
        assert_eq!(tree.capabilities.len(), 1);
        assert_eq!(tree.capabilities[0].system, "credit");
        assert_eq!(tree.systems, ["credit"]);
        assert_eq!(tree.concepts, [("Risk".to_string(), 3)]);
        assert!(tree.has_query_registry);
        assert_eq!(violations(&tree, None), [] as [std::string::String; 0]);
        assert!(!Tree::read(&FakeDisk::default(), &root).under_architecture);
    }
}
