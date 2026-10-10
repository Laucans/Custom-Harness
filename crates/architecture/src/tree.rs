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
//! and the mock-up keeps up with the screens and the Micro-UIs — every page
//! `mock/pages.json` lists is there, every frozen screen under `apps/screens/`
//! has a page, every Micro-UI under `apps/` has a micro-frontend with its
//! fragment.
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
    /// The micro-frontends listed in `mock/pages.json` whose fragment is
    /// under `mock/`, by name.
    pub mocked_components: Vec<String>,
    /// The micro-frontends built from a Micro-UI's code (`source`), with
    /// what the bundle was built from and what the code is now.
    pub bundles: Vec<Bundle>,
}

/// A micro-frontend bundled from a Micro-UI's code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The micro-frontend's name.
    pub name: String,
    /// The Micro-UI's folder, `apps/<system>/<micro-ui>`.
    pub source: String,
    /// The digest `mock/build.mjs` recorded, if it ran.
    pub built_from: Option<String>,
    /// The digest of the inputs as they are now.
    pub digest: String,
}

/// The files of `mock/src/` every bundle embeds.
const SHARED_SOURCES: [&str; 4] = ["server.js", "screen.js", "reads.js", "commands.js"];

/// Folders a digest never walks into.
const SKIPPED_FOLDERS: [&str; 2] = ["node_modules", "dist"];

/// Every file under `rel` (relative to `root`), recursively, as `(path, text)`.
fn files_under(disk: &dyn Disk, root: &Path, rel: &str, out: &mut Vec<(String, String)>) {
    let dir = root.join(rel);
    for name in disk.file_names(&dir) {
        if name.starts_with('.') {
            continue;
        }
        let path = format!("{rel}/{name}");
        if let Some(text) = disk.read_to_string(&root.join(&path)) {
            out.push((path, text));
        }
    }
    for name in disk.dir_names(&dir) {
        if name.starts_with('.') || SKIPPED_FOLDERS.contains(&name.as_str()) {
            continue;
        }
        files_under(disk, root, &format!("{rel}/{name}"), out);
    }
}

/// What a micro-frontend's bundle is built from: its entry, the shared
/// files of `mock/src/`, every file under `<source>/src/`.
fn bundle_inputs(disk: &dyn Disk, root: &Path, name: &str, source: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut add = |rel: String| {
        if let Some(text) = disk.read_to_string(&root.join(&rel)) {
            files.push((rel, text));
        }
    };
    add(format!("mock/src/{name}.js"));
    for shared in SHARED_SOURCES {
        add(format!("mock/src/{shared}"));
    }
    files_under(disk, root, &format!("{source}/src"), &mut files);
    files
}

/// The digest `mock/build.mjs` records: FNV-1a 64 over `path\0content\0`
/// for every input sorted by path, as sixteen hex digits.
#[must_use]
pub fn digest(files: &[(String, String)]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hash = OFFSET;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    for (path, content) in sorted {
        feed(path.as_bytes());
        feed(&[0]);
        feed(content.as_bytes());
        feed(&[0]);
    }
    format!("{hash:016x}")
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
        read_mockup(disk, root, &mut tree);
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

/// What the mock-up adds to the tree: the frozen screens, `mock/pages.json`,
/// the pages and micro-frontends it lists that are under `mock/`, and the
/// bundles with what they were built from.
fn read_mockup(disk: &dyn Disk, root: &Path, tree: &mut Tree) {
    let text = |rel: &str| disk.read_to_string(&root.join(rel));
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
    tree.mock_pages = text("mock/pages.json").map(|content| Manifest {
        path: "mock/pages.json".to_string(),
        json: serde_json::from_str(&content).ok(),
    });
    for page in tree.mock_pages.as_ref().map_or_else(Vec::new, pages_of) {
        if disk.exists(&root.join("mock").join(&page)) {
            tree.mocked.push(page);
        }
    }
    for (name, fragment) in tree
        .mock_pages
        .as_ref()
        .map_or_else(Vec::new, components_of)
    {
        if disk.exists(&root.join("mock").join(&fragment)) {
            tree.mocked_components.push(name);
        }
    }
    for (name, source, built_from) in tree.mock_pages.as_ref().map_or_else(Vec::new, sources_of) {
        let digest = digest(&bundle_inputs(disk, root, &name, &source));
        tree.bundles.push(Bundle {
            name,
            source,
            built_from,
            digest,
        });
    }
}

/// The micro-frontends a `mock/pages.json` lists: `(name, fragment)`.
fn components_of(manifest: &Manifest) -> Vec<(String, String)> {
    manifest
        .json
        .as_ref()
        .and_then(|json| json.get("components")?.as_array())
        .map(|components| {
            components
                .iter()
                .filter_map(|c| {
                    Some((
                        str_of(c, "name")?.to_string(),
                        str_of(c, "fragment")?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The micro-frontends a `mock/pages.json` builds from a Micro-UI's code:
/// `(name, source, builtFrom)`.
fn sources_of(manifest: &Manifest) -> Vec<(String, String, Option<String>)> {
    manifest
        .json
        .as_ref()
        .and_then(|json| json.get("components")?.as_array())
        .map(|components| {
            components
                .iter()
                .filter_map(|c| {
                    Some((
                        str_of(c, "name")?.to_string(),
                        str_of(c, "source")?.to_string(),
                        str_of(c, "builtFrom").map(str::to_string),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The micro-frontends each page of a `mock/pages.json` injects:
/// `(page path, name)`.
fn injected(manifest: &Manifest) -> Vec<(String, String)> {
    manifest
        .json
        .as_ref()
        .and_then(|json| json.get("pages")?.as_array())
        .map(|pages| {
            pages
                .iter()
                .flat_map(|page| {
                    let path = str_of(page, "path").unwrap_or_default().to_string();
                    page.get("components")
                        .and_then(Value::as_array)
                        .map(|names| {
                            names
                                .iter()
                                .filter_map(Value::as_str)
                                .map(|n| (path.clone(), n.to_string()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
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

/// Every Micro-UI declared under `apps/`: its name and its manifest.
fn micro_ui_manifests(tree: &Tree) -> Vec<(String, &Value)> {
    tree.micro_uis
        .iter()
        .filter_map(|(_, m)| {
            let json = m.json.as_ref()?;
            Some((str_of(json, "microUi")?.to_string(), json))
        })
        .collect()
}

/// The entry of `name` under `components` of a `mock/pages.json`.
fn component_entry<'a>(manifest: &'a Manifest, name: &str) -> Option<&'a Value> {
    manifest
        .json
        .as_ref()?
        .get("components")?
        .as_array()?
        .iter()
        .find(|c| str_of(c, "name") == Some(name))
}

/// The ids under `key`, sorted — what two listings are compared on.
fn sorted_ids(json: &Value, key: &str) -> Vec<String> {
    let mut ids = strings_of(json, key);
    ids.sort();
    ids
}

/// The mock-up keeps up with the screens and the Micro-UIs: every page
/// listed is there, every frozen screen has a page that names it, every
/// Micro-UI built has a micro-frontend with its fragment. A repository with
/// no screen, no Micro-UI and no `mock/pages.json` yet is fine — the mock-up
/// begins with the first page.
fn mockup_kept(tree: &Tree) -> Vec<String> {
    let mut found = Vec::new();
    let micro_uis = micro_ui_manifests(tree);
    match &tree.mock_pages {
        None => {
            if !tree.screens.is_empty() || !micro_uis.is_empty() {
                found.push(format!(
                    "mock/pages.json is missing (contract N), and {} frozen screen(s) and {} Micro-UI(s) wait to be mocked",
                    tree.screens.len(),
                    micro_uis.len()
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
            let listed: Vec<String> = components_of(manifest)
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            for (name, fragment) in components_of(manifest) {
                if !tree.mocked_components.contains(&name) {
                    found.push(format!(
                        "{}: lists the micro-frontend {name} at {fragment}, which is not under mock/",
                        manifest.path
                    ));
                }
            }
            for (page, name) in injected(manifest) {
                if !listed.contains(&name) {
                    found.push(format!(
                        "{}: {page} injects {name}, a micro-frontend `components` does not list",
                        manifest.path
                    ));
                }
            }
            // A Micro-UI built is a micro-frontend mocked — its real code on
            // the mock server — and the listing says the same needs and
            // actions as its manifest: the easel and the gates read the
            // listing, not the code.
            for (name, ui) in &micro_uis {
                let Some(entry) = component_entry(manifest, name) else {
                    found.push(format!(
                        "apps/: the Micro-UI {name} has no micro-frontend in {} — mock it (contract N)",
                        manifest.path
                    ));
                    continue;
                };
                for key in ["needs", "actions"] {
                    if sorted_ids(ui, key) != sorted_ids(entry, key) {
                        found.push(format!(
                            "{}: the micro-frontend {name} lists other `{key}` than its micro-ui.json — keep the listing in sync",
                            manifest.path
                        ));
                    }
                }
            }
            // A bundle is the Micro-UI's code as it is: built, and built
            // from what the code is now.
            for bundle in &tree.bundles {
                match &bundle.built_from {
                    None => found.push(format!(
                        "{}: the micro-frontend {} names its source but was never built — run mock/build.sh and commit",
                        manifest.path, bundle.name
                    )),
                    Some(built) if built != &bundle.digest => found.push(format!(
                        "{}: the bundle of {} is stale — {} changed since mock/build.sh last ran; run it and commit",
                        manifest.path, bundle.name, bundle.source
                    )),
                    Some(_) => {}
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
                r#"{"version":1,"pages":[{"path":"pages/credit-review.html","title":"Credit review","objective":"o","screen":"credit-review","components":["RiskBadge"]}],"components":[{"name":"RiskBadge","system":"credit","description":"d","fragment":"components/RiskBadge.html","needs":["credit.calculateRisk"],"actions":["credit.requestLimitChange"]}]}"#,
            )),
            mocked: vec!["pages/credit-review.html".to_string()],
            mocked_components: vec!["RiskBadge".to_string()],
            bundles: Vec::new(),
        }
    }

    #[test]
    fn the_digest_is_the_one_build_mjs_writes() {
        assert_eq!(digest(&[]), "cbf29ce484222325");
        let files = vec![
            (
                "apps/credit/risk-badge/src/index.ts".to_string(),
                "export {}".to_string(),
            ),
            ("mock/src/RiskBadge.js".to_string(), "mount();".to_string()),
        ];
        assert_eq!(digest(&files), "c14e3564be9bd6b2");
        let mut reversed = files.clone();
        reversed.reverse();
        assert_eq!(digest(&reversed), digest(&files), "order does not matter");
    }

    #[test]
    fn a_bundle_is_built_from_the_code_as_it_is() {
        let mut tree = sound();
        tree.bundles.push(Bundle {
            name: "RiskBadge".into(),
            source: "apps/credit/risk-badge".into(),
            built_from: None,
            digest: "c14e3564be9bd6b2".into(),
        });
        let found = violations(&tree, None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("never built"));
        tree.bundles[0].built_from = Some("0000000000000000".into());
        let found = violations(&tree, None);
        assert!(found[0].contains("is stale"), "{found:?}");
        tree.bundles[0].built_from = Some("c14e3564be9bd6b2".into());
        assert!(violations(&tree, None).is_empty());
    }

    #[test]
    fn a_built_micro_ui_is_a_mocked_micro_frontend() {
        // A second Micro-UI under apps/ with no micro-frontend at all.
        let mut tree = sound();
        tree.micro_uis.push((
            "credit".to_string(),
            manifest(
                "apps/credit/late-payments/micro-ui.json",
                r#"{"microUi":"LatePayments","system":"credit","description":"d","needs":["credit.calculateRisk"],"props":{}}"#,
            ),
        ));
        let found = violations(&tree, None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("LatePayments has no micro-frontend"));
        // The listing drifted from the manifest.
        let mut drifted = sound();
        drifted.mock_pages = Some(manifest(
            "mock/pages.json",
            r#"{"version":1,"pages":[{"path":"pages/credit-review.html","title":"t","objective":"o","screen":"credit-review","components":["RiskBadge"]}],"components":[{"name":"RiskBadge","system":"credit","description":"d","fragment":"components/RiskBadge.html","needs":[],"actions":["credit.requestLimitChange"]}]}"#,
        ));
        let found = violations(&drifted, None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("other `needs`"));
        // No mock-up at all while a Micro-UI exists.
        let mut bare = sound();
        bare.screens.clear();
        bare.mock_pages = None;
        bare.mocked.clear();
        bare.mocked_components.clear();
        let found = violations(&bare, None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("1 Micro-UI(s) wait"));
    }

    #[test]
    fn a_micro_frontend_has_its_fragment_and_a_page_injects_only_listed_ones() {
        let mut tree = sound();
        tree.mocked_components.clear();
        let found = violations(&tree, None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("components/RiskBadge.html"));
        let mut stray = sound();
        stray.mock_pages = Some(manifest(
            "mock/pages.json",
            r#"{"version":1,"pages":[{"path":"pages/credit-review.html","title":"t","objective":"o","screen":"credit-review","components":["Ghost"]}],"components":[]}"#,
        ));
        stray.mocked_components.clear();
        stray.micro_uis.clear();
        let found = violations(&stray, None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("injects Ghost"));
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
        // Nothing to mock yet — no screen, no Micro-UI: nothing to say.
        bare.screens.clear();
        bare.micro_uis.clear();
        assert!(violations(&bare, None).is_empty());
        // A Micro-UI or a composition task needs a mocked page.
        let mut unmocked = sound();
        unmocked.mocked.clear();
        unmocked.mocked_components.clear();
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
            assert!(found.iter().any(|f| f.contains("contract N")), "{found:?}");
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
            r#"{"version":1,"pages":[{"path":"pages/credit-review.html","title":"t","objective":"o","screen":"credit-review"},{"path":"pages/gone.html","title":"t","objective":"o"}],"components":[{"name":"RiskBadge","system":"credit","description":"d","fragment":"components/RiskBadge.html","source":"apps/credit/risk-badge","builtFrom":"c14e3564be9bd6b2"}]}"#.to_string(),
        );
        existing.insert(
            root.join("mock/pages/credit-review.html"),
            "<html>".to_string(),
        );
        existing.insert(
            root.join("mock/components/RiskBadge.html"),
            "<div>".to_string(),
        );
        existing.insert(root.join("mock/src/RiskBadge.js"), "mount();".to_string());
        existing.insert(
            root.join("apps/credit/risk-badge/src/index.ts"),
            "export {}".to_string(),
        );
        existing.insert(
            root.join("apps/credit/risk-badge/src/node_modules/x.js"),
            "skipped".to_string(),
        );
        let disk = FakeDisk { existing };
        let tree = Tree::read(&disk, &root);
        assert_eq!(tree.screens, ["credit-review"]);
        assert_eq!(tree.mocked, ["pages/credit-review.html"]);
        assert_eq!(tree.mocked_components, ["RiskBadge"]);
        assert_eq!(tree.bundles.len(), 1);
        assert_eq!(tree.bundles[0].digest, "c14e3564be9bd6b2");
        assert_eq!(
            tree.bundles[0].built_from.as_deref(),
            Some("c14e3564be9bd6b2")
        );
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
