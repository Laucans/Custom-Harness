//! The checkout read as the architecture sees it, and the rules it must keep.
//!
//! [`Tree::read`] walks the folders the architecture asks for — Capability
//! crates under `crates/<system>/capabilities/`, Micro-UIs under `apps/`,
//! `DataCapabilities` and aggregates under `crates/dataguard/`, Concepts under
//! `concepts/` — through the `Disk` port, and fails on nothing: an absent or
//! unreadable manifest is one the rules report.
//!
//! [`violations`] checks what a diff is reviewed against: a Capability depends
//! on no Capability and on nothing of the write side; its manifest names its
//! system and invokes nothing; a Micro-UI needs Capabilities that exist, in its
//! own system, and triggers `DataCapabilities` that exist; a `DataCapability`
//! belongs to the `DataGuard` and declares its effect; and the unit a task
//! declared under `## Architecture` is actually there, on the side it said;
//! and the mock-up keeps up with the screens — every page `mock/pages.json`
//! lists is there, every frozen screen under `apps/screens/` has a page.
//! [`undocumented_concepts`] names every Concept a Capability implements
//! (`implements: Risk@3`) that `concepts/Risk/v3/concept.json` does not
//! document. Both are pure over an in-memory [`Tree`].
//!
//! The repository's CI gates (`assets/gates.yml`, installed by `init-repo`)
//! run the same rules; `dev_loop` runs these after each round, so a round that
//! drifted stops with the list rather than at the next pull request's checks.

use std::path::Path;

use harness_core::ports::shell::disk::Disk;
use serde_json::Value;

use crate::vocabulary::{Declaration, Effect, Side, Unit};

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
    /// Every frozen screen under `apps/screens/`, by its `screen` name.
    pub screens: Vec<String>,
    /// `mock/pages.json`, if present (contract N).
    pub mock_pages: Option<Manifest>,
    /// The pages listed in `mock/pages.json` whose file is under `mock/`.
    pub mocked: Vec<String>,
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
        for file in disk.file_names(&root.join("apps/screens")) {
            let Some(stem) = file.strip_suffix(".json") else {
                continue;
            };
            let name = text(&format!("apps/screens/{file}"))
                .and_then(|content| serde_json::from_str::<Value>(&content).ok())
                .and_then(|json| str_of(&json, "screen").map(str::to_string))
                .unwrap_or_else(|| stem.to_string());
            tree.screens.push(name);
        }
        tree.mock_pages = manifest("mock/pages.json");
        for page in tree.mock_pages.as_ref().map_or_else(Vec::new, pages_of) {
            if disk.exists(&root.join("mock").join(&page)) {
                tree.mocked.push(page);
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

/// The `path` of every page a `mock/pages.json` lists.
fn pages_of(manifest: &Manifest) -> Vec<String> {
    manifest
        .json
        .as_ref()
        .and_then(|json| json.get("pages")?.as_array())
        .map(|pages| {
            pages
                .iter()
                .filter_map(|page| str_of(page, "path").map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The `screen` each page of a `mock/pages.json` mocks, when it names one.
fn mocked_screens(manifest: &Manifest) -> Vec<String> {
    manifest
        .json
        .as_ref()
        .and_then(|json| json.get("pages")?.as_array())
        .map(|pages| {
            pages
                .iter()
                .filter_map(|page| str_of(page, "screen").map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
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

    found.extend(mockup_kept(tree));

    if let Some(task) = task {
        found.extend(unit_present(tree, task));
    }
    found
}

/// The mock-up keeps up with the screens: every page listed is there, every
/// frozen screen has a page that names it. A repository with no screens and
/// no `mock/pages.json` yet is fine — the mock-up begins with the first page.
fn mockup_kept(tree: &Tree) -> Vec<String> {
    let mut found = Vec::new();
    match &tree.mock_pages {
        None => {
            if !tree.screens.is_empty() {
                found.push(format!(
                    "mock/pages.json is missing (contract N), and {} frozen screen(s) wait to be mocked",
                    tree.screens.len()
                ));
            }
            return found;
        }
        Some(Manifest { json: None, path }) => {
            found.push(format!("{path} is not JSON"));
            return found;
        }
        Some(manifest) => {
            for page in pages_of(manifest) {
                if !tree.mocked.contains(&page) {
                    found.push(format!(
                        "{}: lists {page}, which is not under mock/",
                        manifest.path
                    ));
                }
            }
            let mocked = mocked_screens(manifest);
            for screen in &tree.screens {
                if !mocked.contains(screen) {
                    found.push(format!(
                        "apps/screens/{screen}.json: no page in {} mocks this screen — mock it first",
                        manifest.path
                    ));
                }
            }
        }
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
                unmocked(tree, "a Micro-UI")
            } else {
                missing(&format!("Micro-UI of system `{system}`"))
            }
        }
        Unit::Composition => unmocked(tree, "a screen composition"),
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
        | Unit::Migration
        | Unit::Infrastructure
        | Unit::Tooling => Vec::new(),
    }
}

/// A UI unit is shown somewhere: the mock-up has a page under `mock/` for
/// it, or the task is told to mock the screen first (contract N).
fn unmocked(tree: &Tree, what: &str) -> Vec<String> {
    if tree.mocked.is_empty() {
        vec![format!(
            "the task built {what} and mock/pages.json lists no page under mock/ that shows it — \
             mock the screen first (contract N)"
        )]
    } else {
        Vec::new()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_disk::FakeDisk;
    use std::collections::HashMap;
    use std::path::PathBuf;

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
            screens: vec!["credit-review".to_string()],
            mock_pages: Some(manifest(
                "mock/pages.json",
                r#"{"version":1,"pages":[{"path":"pages/credit-review.html","title":"Credit review","objective":"o","screen":"credit-review"}]}"#,
            )),
            mocked: vec!["pages/credit-review.html".to_string()],
        }
    }

    #[test]
    fn the_mockup_keeps_up_with_the_screens() {
        // A screen with no page, and a listed page that is not there.
        let mut tree = sound();
        tree.screens.push("limit-review".to_string());
        tree.mocked.clear();
        let found = violations(&tree, None);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].contains("not under mock/"));
        assert!(found[1].contains("limit-review"));
        // No mock-up at all while screens exist.
        let mut bare = sound();
        bare.mock_pages = None;
        bare.mocked.clear();
        let found = violations(&bare, None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("pages.json is missing"));
        // Nothing to mock yet: nothing to say.
        bare.screens.clear();
        assert!(violations(&bare, None).is_empty());
        // A Micro-UI or a composition task needs a mocked page.
        let mut unmocked = sound();
        unmocked.mocked.clear();
        unmocked.mock_pages = Some(manifest("mock/pages.json", r#"{"version":1,"pages":[]}"#));
        unmocked.screens.clear();
        for unit in [Unit::MicroUi, Unit::Composition] {
            let task = Declaration {
                unit,
                system: "credit".to_string(),
                concept: None,
                effect: None,
                touches: Vec::new(),
            };
            let found = violations(&unmocked, Some(&task));
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].contains("contract N"));
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
        existing.insert(
            root.join("apps/screens/credit-review.json"),
            r#"{"screen":"credit-review"}"#.to_string(),
        );
        existing.insert(
            root.join("mock/pages.json"),
            r#"{"version":1,"pages":[{"path":"pages/credit-review.html","title":"t","objective":"o","screen":"credit-review"},{"path":"pages/gone.html","title":"t","objective":"o"}]}"#.to_string(),
        );
        existing.insert(
            root.join("mock/pages/credit-review.html"),
            "<html>".to_string(),
        );
        let disk = FakeDisk { existing };
        let tree = Tree::read(&disk, &root);
        assert_eq!(tree.screens, ["credit-review"]);
        assert_eq!(tree.mocked, ["pages/credit-review.html"]);
        assert!(tree.under_architecture);
        assert_eq!(tree.capabilities.len(), 1);
        assert_eq!(tree.capabilities[0].system, "credit");
        assert_eq!(tree.systems, ["credit"]);
        assert_eq!(tree.concepts, [("Risk".to_string(), 3)]);
        assert!(tree.has_query_registry);
        let found = violations(&tree, None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("pages/gone.html"));
        assert!(!Tree::read(&FakeDisk::default(), &root).under_architecture);
    }
}
