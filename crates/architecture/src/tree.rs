//! The checkout read as the architecture sees it, and the rules it must keep.
//!
//! [`Tree::read`] walks the folders the architecture asks for — Capability
//! crates under `crates/<system>/capabilities/`, Micro-UIs under `apps/`,
//! `DataCapabilities` under `crates/<system>/data-capabilities/`, aggregates
//! under `crates/<system>/aggregates/`, the registry's `Cargo.toml`,
//! Concepts under `concepts/` — through the `Disk` port, and fails on
//! nothing: an absent or unreadable manifest is one the rules report.
//!
//! [`violations`] checks what a diff is reviewed against: a Capability depends
//! on no Capability and on nothing of the write side; its manifest names its
//! system and invokes nothing; a Micro-UI needs Capabilities that exist, in
//! any system, and triggers `DataCapabilities` that exist; a `DataCapability`
//! belongs to the `DataGuard`, sits under the system its id names (never
//! under `crates/dataguard/`), declares its effect and touches no derived
//! field; an aggregate sits under its system, in a folder named after it
//! (never under `crates/dataguard/`); every unit crate is named in the
//! registry and its library is named as the registry's glue calls it; and
//! the unit a task
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

/// A unit folder that is a crate: what the registry names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitCrate {
    /// `crates/<system>/<kind>/<folder>`, the kind one of [`UNIT_KINDS`].
    pub dir: String,
    /// Its `Cargo.toml`.
    pub cargo_toml: String,
    /// Its manifest says `"host": "hostless"`: no server runs it, and the
    /// registry leaves it out.
    pub hostless: bool,
}

/// The three kinds of unit folder under a system, with their manifest.
pub const UNIT_KINDS: [(&str, &str); 3] = [
    ("aggregates", "aggregate.json"),
    ("data-capabilities", "data-capability.json"),
    ("capabilities", "capability.json"),
];

/// The registry's manifest, which `scripts/register.sh` writes.
pub const REGISTRY: &str = "crates/dataguard/registry/Cargo.toml";

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
    /// Every `aggregate.json` under `crates/<system>/aggregates/`.
    pub aggregates: Vec<Manifest>,
    /// Every unit folder holding a `Cargo.toml`, of the three kinds.
    pub unit_crates: Vec<UnitCrate>,
    /// The registry's `Cargo.toml` ([`REGISTRY`]), when there is one.
    pub registry: Option<String>,
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
    /// The design system, as `mock/design/design.json` has it (contract O):
    /// `None` when there is none.
    pub design: Option<Manifest>,
    /// The kit's generated components, by name with their title, as
    /// `mock/kit/kit.json` lists them (contract O).
    pub kit: Vec<(String, String)>,
    /// The root `Cargo.toml`, when there is one: what names the workspace's
    /// members, by glob or by name.
    pub root_manifest: Option<String>,
    /// `queries/registry.json` (contract C), as `(capability, file, sha256)`
    /// per entry; empty when there is no registry or it is not JSON.
    pub registered_queries: Vec<(String, String, String)>,
    /// The query files the Capabilities' `reads` name, resolved.
    pub reads: Vec<Read>,
    /// The plan's graph (`plan/graph.json`, contract R), when the product
    /// has one: the ids a milestone will produce count as declared.
    pub graph: Option<crate::graph::Graph>,
}

/// One `reads` entry of a Capability, resolved as `scripts/queries.sh`
/// resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    /// The Capability's id.
    pub capability: String,
    /// The path as the manifest wrote it.
    pub written: String,
    /// The path found, with the file's hash — `None` when the file is
    /// nowhere under the repository, the folder or the system.
    pub found: Option<(String, String)>,
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

/// The files of `mock/src/` every bundle's digest reads, the same list as
/// `mock/build.mjs`'s `SHARED` — a product on an older `build.mjs` must
/// compute the digest the harness checks. The legacy `reads.js` and
/// `commands.js` stay in it: the server imports them at run time now, but
/// an absent file is no input, so once `scripts/split-handlers.mjs` removed
/// them the digest is the server's and the screen host's alone, and
/// `mock/handlers/` is never read.
const SHARED_SOURCES: [&str; 4] = ["server.js", "screen.js", "reads.js", "commands.js"];

/// Whether rewriting `path` (from the repository's root) stales every bundle.
///
/// A file each one embeds (`mock/src/server.js`, `mock/src/screen.js`) or
/// the build that bundles and digests them (`mock/build.mjs`,
/// `mock/build.sh`). Whatever rewrites one rebuilds the
/// bundles in the same commit (`sh mock/build.sh`), or every branch built
/// on it fails `pages-mocked--bundles`.
#[must_use]
pub fn stales_every_bundle(path: &str) -> bool {
    matches!(path, "mock/build.mjs" | "mock/build.sh")
        || path
            .strip_prefix("mock/src/")
            .is_some_and(|name| SHARED_SOURCES.contains(&name))
}

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
    /// Whether the plan's graph knows `id` as a `kind` some milestone
    /// produces: a contract the consumer builds against before it is built,
    /// its fixture under `mock/fixtures/` standing for it.
    #[must_use]
    pub fn plans(&self, kind: crate::graph::Kind, id: &str) -> bool {
        self.graph.as_ref().is_some_and(|graph| {
            graph
                .ids
                .get(id)
                .is_some_and(|i| i.kind == kind && i.producer.is_some())
        })
    }

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
            root_manifest: text("Cargo.toml"),
            registry: text(REGISTRY),
            graph: text(crate::graph::PATH).and_then(|t| crate::graph::Graph::parse(&t).ok()),
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
        // Under every system, as the rules lay it out
        // (`crates/<system>/data-capabilities/<name>/`): reading only the
        // DataGuard's folder judged ai.saveSettings undeclared (#43).
        for system in disk.dir_names(&root.join("crates")) {
            let folder = root.join("crates").join(&system).join("data-capabilities");
            for name in disk.dir_names(&folder) {
                if let Some(found) = manifest(&format!(
                    "crates/{system}/data-capabilities/{name}/data-capability.json"
                )) {
                    tree.data_capabilities.push(found);
                }
            }
        }
        // Under every system too (`crates/<system>/aggregates/<Aggregate>/`);
        // the DataGuard's folder is read with the others, so that
        // `violations` can name the one an aggregate belongs in.
        for system in disk.dir_names(&root.join("crates")) {
            let folder = root.join("crates").join(&system).join("aggregates");
            for name in disk.dir_names(&folder) {
                if let Some(found) =
                    manifest(&format!("crates/{system}/aggregates/{name}/aggregate.json"))
                {
                    tree.aggregates.push(found);
                }
            }
        }
        tree.unit_crates = unit_crates(disk, root);
        read_queries(disk, root, &mut tree);
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

/// Every unit folder holding a `Cargo.toml`, as `scripts/register.sh` walks
/// them: by system, then by kind, then by folder.
fn unit_crates(disk: &dyn Disk, root: &Path) -> Vec<UnitCrate> {
    let text = |rel: &str| disk.read_to_string(&root.join(rel));
    let mut found = Vec::new();
    for system in disk.dir_names(&root.join("crates")) {
        for (kind, _) in UNIT_KINDS {
            for name in disk.dir_names(&root.join("crates").join(&system).join(kind)) {
                let dir = format!("crates/{system}/{kind}/{name}");
                let Some(cargo_toml) = text(&format!("{dir}/Cargo.toml")) else {
                    continue;
                };
                // Any of the three manifests, read as the script reads it:
                // whitespace aside, `"host":"hostless"`.
                let hostless = UNIT_KINDS.iter().any(|(_, manifest)| {
                    text(&format!("{dir}/{manifest}")).is_some_and(|content| {
                        let packed: String =
                            content.chars().filter(|c| !c.is_whitespace()).collect();
                        packed.contains("\"host\":\"hostless\"")
                    })
                });
                found.push(UnitCrate {
                    dir,
                    cargo_toml,
                    hostless,
                });
            }
        }
    }
    found
}

/// The SHA-256 of `text`, as `scripts/queries.sh` records it.
#[must_use]
pub fn sha256(text: &str) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let digest = sha2::Sha256::digest(text.as_bytes());
    digest.iter().fold(String::new(), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    })
}

/// The registry and the `reads` of every Capability, resolved as the
/// script resolves them: from the root, from the Capability's folder, from
/// its system's folder.
fn read_queries(disk: &dyn Disk, root: &Path, tree: &mut Tree) {
    let text = |rel: &str| disk.read_to_string(&root.join(rel));
    if let Some(registry) = text("queries/registry.json")
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .and_then(|json| json.get("queries")?.as_array().cloned())
    {
        for entry in registry {
            if let (Some(capability), Some(file), Some(sha)) = (
                str_of(&entry, "capability"),
                str_of(&entry, "file"),
                str_of(&entry, "sha256"),
            ) {
                tree.registered_queries.push((
                    capability.to_string(),
                    file.to_string(),
                    sha.to_string(),
                ));
            }
        }
    }
    for crate_ in &tree.capabilities {
        let Some(json) = crate_.manifest.as_ref().and_then(|m| m.json.as_ref()) else {
            continue;
        };
        let Some(id) = str_of(json, "capability") else {
            continue;
        };
        for read in strings_of(json, "reads") {
            let candidates = [
                read.clone(),
                format!("{}/{read}", crate_.dir),
                format!("crates/{}/{read}", crate_.system),
            ];
            let found = candidates.iter().find_map(|candidate| {
                text(candidate).map(|content| (candidate.clone(), sha256(&content)))
            });
            tree.reads.push(Read {
                capability: id.to_string(),
                written: read,
                found,
            });
        }
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
    tree.design = text("mock/design/design.json").map(|content| Manifest {
        path: "mock/design/design.json".to_string(),
        json: serde_json::from_str(&content).ok(),
    });
    tree.kit = text("mock/kit/kit.json")
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .and_then(|json| json.get("components")?.as_array().cloned())
        .map(|components| {
            components
                .iter()
                .filter(|c| str_of(c, "state") == Some("generated"))
                .filter_map(|c| {
                    Some((
                        str_of(c, "name")?.to_string(),
                        str_of(c, "title").unwrap_or_default().to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
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

/// A manifest's string field, when it is JSON and has one.
fn str_of_manifest<'a>(manifest: &'a Manifest, key: &str) -> Option<&'a str> {
    manifest.json.as_ref().and_then(|json| str_of(json, key))
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

/// Why a `DataCapability` manifest read at `path` does not sit where its id
/// says, in one line — `None` when it does. The path is the one
/// [`Tree::read`] writes, `crates/<system>/data-capabilities/<name>/…`,
/// and the folder must be the id's namespace: never `dataguard`, which
/// owns every `DataCapability` and is the system of none.
fn misplaced(path: &str, id: Option<&str>) -> Option<String> {
    let mut parts = path.split('/');
    let folder = parts.nth(1).unwrap_or_default();
    let name = parts.nth(1).unwrap_or_default();
    let Some(id) = id else {
        return Some(format!("{path} names no `dataCapability`"));
    };
    match system_of_id(id) {
        Some(namespace) if namespace == folder && namespace != "dataguard" => None,
        Some(namespace) if namespace != "dataguard" => Some(format!(
            "{path}: `{id}` belongs under crates/{namespace}/data-capabilities/{name}/, its \
             id's namespace — the DataGuard owns a DataCapability (`owner: dataguard`), it \
             does not house it"
        )),
        _ => Some(format!(
            "{path}: `{id}` names no system as its id's namespace — a DataCapability is \
             `<system>.<name>`, under crates/<system>/data-capabilities/{name}/; the DataGuard \
             owns it, it is not its system"
        )),
    }
}

/// Why an aggregate manifest read at `path` does not sit where it belongs,
/// in one line — `None` when it does. The path is the one [`Tree::read`]
/// writes, `crates/<system>/aggregates/<Folder>/aggregate.json`: the folder
/// is the aggregate's name, and the system is never `dataguard`, which
/// judges every aggregate and houses none. The system it belongs in is the
/// one the `DataCapabilities` targeting it share, `<system>` otherwise.
fn aggregate_misplaced(path: &str, name: Option<&str>, tree: &Tree) -> Option<String> {
    let mut parts = path.split('/');
    let system = parts.nth(1).unwrap_or_default();
    let folder = parts.nth(1).unwrap_or_default();
    let Some(name) = name else {
        return Some(format!("{path} names no `aggregate`"));
    };
    if system == "dataguard" {
        let home = targeting_system(tree, name).unwrap_or_else(|| "<system>".to_string());
        return Some(format!(
            "{path}: `{name}` belongs under crates/{home}/aggregates/{name}/, an aggregate's \
             folder in its system — the DataGuard judges every aggregate, it houses none"
        ));
    }
    (folder != name).then(|| {
        format!(
            "{path}: `{name}` belongs under crates/{system}/aggregates/{name}/, an aggregate's \
             folder named after it"
        )
    })
}

/// The one system the `DataCapabilities` targeting `aggregate` share,
/// their ids' namespace; `None` when none targets it, or several systems do.
fn targeting_system(tree: &Tree, aggregate: &str) -> Option<String> {
    let mut systems: Vec<&str> = tree
        .data_capabilities
        .iter()
        .filter_map(|m| m.json.as_ref())
        .filter(|json| json.get("target").and_then(|t| str_of(t, "aggregate")) == Some(aggregate))
        .filter_map(|json| str_of(json, "dataCapability").and_then(system_of_id))
        .filter(|system| *system != "dataguard")
        .collect();
    systems.sort_unstable();
    systems.dedup();
    match systems.as_slice() {
        [one] => Some((*one).to_string()),
        _ => None,
    }
}

/// `AiSettings` as `ai_settings`, as the registry's build script spells an
/// aggregate's library: an underscore before an uppercase letter that
/// follows a lowercase one or a digit, `-` as `_`, all lowercase.
fn snake(name: &str) -> String {
    let mut out = String::new();
    let mut previous: Option<char> = None;
    for c in name.chars() {
        if c == '-' {
            out.push('_');
        } else {
            if c.is_ascii_uppercase()
                && previous.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit())
            {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        }
        previous = Some(c);
    }
    out
}

/// The library a `Cargo.toml` builds: `[lib] name`, else the package's
/// name with `-` as `_`; `None` when it names neither.
fn lib_name(toml: &str) -> Option<String> {
    let mut section = "";
    let mut package = None;
    let mut lib = None;
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed;
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != "name" {
            continue;
        }
        let value = value.trim().trim_matches('"').to_string();
        match section {
            "[package]" if package.is_none() => package = Some(value.replace('-', "_")),
            "[lib]" if lib.is_none() => lib = Some(value),
            _ => {}
        }
    }
    lib.or(package)
}

/// The unit dependencies a registry's `Cargo.toml` names, as its build
/// script reads them: every `path = "../../<system>/<kind>/<folder>"` of a
/// unit kind, as its package and `crates/<system>/<kind>/<folder>`.
fn registered(manifest: &str) -> Vec<(String, String)> {
    manifest
        .lines()
        .filter_map(|line| {
            let (key, rest) = line.split_once("path = \"")?;
            let (path, _) = rest.split_once('"')?;
            let unit = path.strip_prefix("../../")?.trim_end_matches('/');
            let parts: Vec<&str> = unit.split('/').collect();
            let kind = parts.get(1)?;
            let package = key.split('=').next().unwrap_or_default().trim();
            (parts.len() == 3 && UNIT_KINDS.iter().any(|(k, _)| k == kind))
                .then(|| (package.to_string(), format!("crates/{unit}")))
        })
        .collect()
}

/// The packages a registry's `[package.metadata.cargo-machete]` ignores,
/// in the order written; `None` when it has no `ignored` list.
fn machete_ignored(manifest: &str) -> Option<Vec<String>> {
    let mut section = manifest
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "[package.metadata.cargo-machete]")
        .skip(1)
        .take_while(|line| !line.starts_with('['))
        .filter(|line| !line.starts_with('#'));
    let first = section.find(|line| {
        line.strip_prefix("ignored")
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    })?;
    let text: String = std::iter::once(first).chain(section).collect();
    let (_, list) = text.split_once('[')?;
    let (list, _) = list.split_once(']')?;
    Some(
        list.split('"')
            .skip(1)
            .step_by(2)
            .map(ToString::to_string)
            .collect(),
    )
}

/// True when a `Cargo.toml` takes `harness-dataguard` from a source of its
/// own instead of the workspace, as `scripts/register.sh --check` reads it:
/// an inline line or a `[…dependencies.harness-dataguard]` table without
/// `workspace = true`, or a dotted key naming a source.
fn engine_elsewhere(toml: &str) -> bool {
    let inherits = |line: &str| {
        line.split_once("workspace").is_some_and(|(_, rest)| {
            rest.trim_start()
                .strip_prefix('=')
                .is_some_and(|v| v.trim_start().starts_with("true"))
        })
    };
    let mut table = false;
    let mut inherited = false;
    for line in toml.lines().map(str::trim) {
        if line.starts_with('[') {
            if table && !inherited {
                return true;
            }
            table = line.ends_with("dependencies.harness-dataguard]");
            inherited = false;
            continue;
        }
        if table && line.starts_with("workspace") && inherits(line) {
            inherited = true;
        }
        if let Some(rest) = line.strip_prefix("harness-dataguard") {
            let rest = rest.trim_start();
            if rest.starts_with('=') && !inherits(rest) {
                return true;
            }
            let source = rest.strip_prefix('.').and_then(|key| {
                ["git", "path", "version", "tag", "branch", "rev"]
                    .into_iter()
                    .find(|name| {
                        key.strip_prefix(name)
                            .is_some_and(|after| after.trim_start().starts_with('='))
                    })
            });
            if source.is_some() {
                return true;
            }
        }
    }
    table && !inherited
}

/// Every unit crate is named in the registry, and the registry names no
/// other (`scripts/register.sh --check`, the gate `units-registered`); a
/// `DataCapability`'s library is named after its folder, an aggregate's
/// `aggregate_<snake>`: the registry's glue calls them by that name; a unit
/// that needs the engine takes it from the workspace, so the registry
/// plants one engine.
fn units_registered(tree: &Tree) -> Vec<String> {
    let mut found = Vec::new();
    let served: Vec<&str> = tree
        .unit_crates
        .iter()
        .filter(|unit| !unit.hostless)
        .map(|unit| unit.dir.as_str())
        .collect();
    // The script writes the registry as soon as there is a unit crate, a
    // hostless one included: it then names none.
    match &tree.registry {
        None if !tree.unit_crates.is_empty() => found.push(format!(
            "{REGISTRY} is missing and {} unit crate(s) wait on the registry — run \
             scripts/register.sh",
            tree.unit_crates.len()
        )),
        None => {}
        Some(manifest) => {
            let (mut packages, named): (Vec<String>, Vec<String>) =
                registered(manifest).into_iter().unzip();
            packages.sort_unstable();
            // The units are used by glue in OUT_DIR, which cargo machete
            // does not read: the script lists them all as ignored.
            if machete_ignored(manifest).is_none_or(|ignored| ignored != packages) {
                found.push(format!(
                    "{REGISTRY}: [package.metadata.cargo-machete] does not ignore exactly the \
                     registry's unit packages, sorted — run scripts/register.sh"
                ));
            }
            for dir in served.iter().filter(|dir| !named.iter().any(|n| n == *dir)) {
                found.push(format!(
                    "{dir}: a unit crate the registry does not name — run scripts/register.sh"
                ));
            }
            for dir in named.iter().filter(|dir| !served.contains(&dir.as_str())) {
                found.push(format!(
                    "{REGISTRY} names {dir}, where the registry finds no unit crate — run \
                     scripts/register.sh"
                ));
            }
        }
    }
    for unit in tree
        .unit_crates
        .iter()
        .filter(|unit| engine_elsewhere(&unit.cargo_toml))
    {
        found.push(format!(
            "{}/Cargo.toml takes harness-dataguard from its own source — write \
             harness-dataguard = {{ workspace = true }}: the registry plants one engine",
            unit.dir
        ));
    }
    for unit in &tree.unit_crates {
        let mut parts = unit.dir.split('/').skip(2);
        let (Some(kind), Some(folder)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (what, expected) = match kind {
            "aggregates" => ("aggregate", format!("aggregate_{}", snake(folder))),
            "data-capabilities" => ("DataCapability", folder.replace('-', "_")),
            _ => continue,
        };
        let lib = lib_name(&unit.cargo_toml);
        if lib.as_deref() != Some(expected.as_str()) {
            found.push(format!(
                "{}/Cargo.toml: the library of the {what} {folder} is named {} — name it \
                 {expected}: the registry's glue calls it by that name",
                unit.dir,
                lib.as_deref().unwrap_or("nothing")
            ));
        }
    }
    found
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
    // Rule 2: a slice is a member by glob, never by name — a sparse checkout
    // that leaves it out must still resolve (the gate `workspace-members-by-glob`).
    for member in tree
        .root_manifest
        .as_deref()
        .map(crate::closure::explicit_members)
        .unwrap_or_default()
    {
        let parts: Vec<&str> = member.split('/').collect();
        if parts.len() == 4
            && parts[0] == "crates"
            && matches!(
                parts[2],
                "capabilities" | "data-capabilities" | "aggregates"
            )
        {
            found.push(format!(
                "Cargo.toml: {member} is a unit listed as a member by name — list units by \
                 glob, so a narrowed checkout resolves and adding one edits no shared file"
            ));
        }
    }
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
                // Runtime dependencies only, as rule 4 below: a test reaching
                // the engine's client re-export links nothing at run time
                // (`ForgeDesMondes`' `live.getDiffusion`, 2026-10-24).
                let offending: Vec<&str> = runtime_dependencies(toml)
                    .into_iter()
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
                // Rule 4: read-only by the types — through its system's read
                // crate, never a driver of its own (and a dev-dependency is
                // not a dependency: a test-only crate reaches no store).
                let drivers: Vec<&str> = runtime_dependencies(toml)
                    .into_iter()
                    .filter(|line| DRIVERS.iter().any(|driver| names_crate(line, driver)))
                    .collect();
                if !drivers.is_empty() {
                    found.push(format!(
                        "{}/Cargo.toml holds a database driver — a Capability reads through its \
                         system's read crate, never a driver of its own: {}",
                        crate_.dir,
                        drivers.join(" | ").trim()
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
                // A Capability reads the Data through persisted queries, or
                // the outside through a port (contract Q): one of the two.
                if strings_of(json, "reads").is_empty() && strings_of(json, "ports").is_empty() {
                    found.push(format!(
                        "{path} declares no persisted query under `reads` and no port under \
                         `ports` — every read is declared"
                    ));
                }
            }
        }
    }

    for (_, manifest) in &tree.micro_uis {
        let Some(json) = &manifest.json else {
            found.push(format!("{} is not JSON", manifest.path));
            continue;
        };
        for need in strings_of(json, "needs") {
            if !capability_ids.contains(&need.as_str())
                && !tree.plans(crate::graph::Kind::Capability, &need)
            {
                found.push(format!(
                    "{}: needs `{need}`, a Capability no manifest declares",
                    manifest.path
                ));
            }
        }
        for action in strings_of(json, "actions") {
            if !data_capability_ids.contains(&action.as_str())
                && !tree.plans(crate::graph::Kind::DataCapability, &action)
            {
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
        // Its folder is its id's namespace (`campaign.saveEntity` sits under
        // crates/campaign/data-capabilities/save-entity/): the DataGuard
        // owns every DataCapability, it houses none — ForgeDesMondes'
        // milestone 18 laid three of `campaign` under crates/dataguard/, and
        // two conventions coexisted with no gate to say so.
        if let Some(why) = misplaced(&manifest.path, str_of(json, "dataCapability")) {
            found.push(why);
        }
        if str_of(json, "effect").and_then(Effect::parse).is_none() {
            found.push(format!(
                "{}: `effect` is not one of insert, update, delete, upsert",
                manifest.path
            ));
        }
        // Ownership (contracts F, I): a mutation targets an aggregate that
        // declares every field it touches — the gate `ownership`.
        let touches = strings_of(json, "touches");
        let mutates = str_of(json, "effect").and_then(Effect::parse) != Some(Effect::Insert)
            || !touches.is_empty();
        if mutates {
            let target = json
                .get("target")
                .and_then(|t| str_of(t, "aggregate"))
                .unwrap_or_default();
            let aggregate = tree.aggregates.iter().find(|a| {
                a.json
                    .as_ref()
                    .and_then(|j| str_of(j, "aggregate"))
                    .is_some_and(|name| name == target)
            });
            match aggregate.and_then(|a| a.json.as_ref()) {
                None => found.push(format!(
                    "{}: mutates `{target}`, which no aggregate.json declares (contract I)",
                    manifest.path
                )),
                Some(declared) => {
                    for field in &touches {
                        let key = field.rsplit('.').next().unwrap_or(field);
                        let fields = declared.get("fields").and_then(Value::as_object);
                        match fields.and_then(|fields| fields.get(key)) {
                            None => found.push(format!(
                                "{}: touches `{field}`, which the aggregate `{target}` does not declare (contract I)",
                                manifest.path
                            )),
                            // A derived field is recomputed: the engine
                            // refuses a command that writes it (Forbidden).
                            Some(spec) if str_of(spec, "kind") == Some("derived") => {
                                found.push(format!(
                                    "{}: touches `{field}`, which the aggregate `{target}` declares derived — recomputed, never written (contract I)",
                                    manifest.path
                                ));
                            }
                            Some(_) => {}
                        }
                    }
                }
            }
        }
    }

    // Contract I: an aggregate sits under its system, in a folder named
    // after it — the gate `aggregate-placed`.
    for manifest in &tree.aggregates {
        let Some(json) = &manifest.json else {
            found.push(format!("{} is not JSON", manifest.path));
            continue;
        };
        if let Some(why) = aggregate_misplaced(&manifest.path, str_of(json, "aggregate"), tree) {
            found.push(why);
        }
    }

    found.extend(units_registered(tree));
    found.extend(mockup_kept(tree));
    found.extend(queries_registered(tree));

    if let Some(task) = task {
        found.extend(unit_present(tree, task));
    }
    found
}

/// The database drivers a Capability must not hold (`scripts/isolation.sh`,
/// rule 4): it reads through its system's read crate.
pub const DRIVERS: [&str; 8] = [
    "sqlx",
    "tokio-postgres",
    "postgres",
    "diesel",
    "sea-orm",
    "rusqlite",
    "mysql",
    "mysql_async",
];

/// The lines of `[dependencies]` — not `[dev-dependencies]`, not
/// `[build-dependencies]` — of a `Cargo.toml`, comments left out.
fn runtime_dependencies(toml: &str) -> Vec<&str> {
    let mut in_runtime = false;
    let mut lines = Vec::new();
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_runtime = trimmed == "[dependencies]";
            continue;
        }
        if in_runtime && !trimmed.is_empty() && !trimmed.starts_with('#') {
            lines.push(trimmed);
        }
    }
    lines
}

/// True when a dependency line names this crate: `sqlx = …`, `sqlx.workspace`,
/// `"sqlx" = …` — the name before `=` or `.`, underscores and dashes alike.
fn names_crate(line: &str, name: &str) -> bool {
    let key = line
        .split(['=', '.'])
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"');
    key.replace('_', "-") == name.replace('_', "-")
}

/// Contract C: every `reads` of a Capability is a file, registered under
/// that Capability with the hash of the file as it is — what
/// `scripts/queries.sh --check` refuses, read the same way here.
fn queries_registered(tree: &Tree) -> Vec<String> {
    let mut found = Vec::new();
    for Read {
        capability: id,
        written: read,
        found: resolved,
    } in &tree.reads
    {
        let Some((path, sha)) = resolved else {
            found.push(format!(
                "{id} reads `{read}`, which is nowhere under the repository, its folder or its system (contract C)"
            ));
            continue;
        };
        match tree
            .registered_queries
            .iter()
            .find(|(capability, file, _)| capability == id && file == path)
        {
            None => found.push(format!(
                "{id} reads `{path}`, which queries/registry.json does not register — run scripts/queries.sh (contract C)"
            )),
            Some((_, _, registered)) if registered != sha => found.push(format!(
                "{id} reads `{path}`, which changed since queries/registry.json was written — run scripts/queries.sh (contract C)"
            )),
            Some(_) => {}
        }
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
        Unit::Aggregate => aggregate_present(tree, task),
        Unit::PersistedQuery if !tree.has_query_registry => {
            missing("persisted query, and queries/registry.json does not exist")
        }
        Unit::Infrastructure if !system.is_empty() && !tree.systems.iter().any(|s| s == system) => {
            missing(&format!(
                "infrastructure unit of the system `{system}`, and crates/{system}/ does not \
                 exist — the stack builds it in Rust, there"
            ))
        }
        Unit::PersistedQuery | Unit::Infrastructure | Unit::Tooling => Vec::new(),
    }
}

/// An `aggregate` task documented the aggregate it named — or one at all.
fn aggregate_present(tree: &Tree, task: &Declaration) -> Vec<String> {
    let missing = |what: &str| vec![format!("the task declared it built an {what}")];
    match task.aggregate.as_deref() {
        Some(name)
            if !tree
                .aggregates
                .iter()
                .any(|m| str_of_manifest(m, "aggregate") == Some(name)) =>
        {
            missing(&format!(
                "aggregate `{name}`, and no aggregate.json declares it"
            ))
        }
        None if tree.aggregates.is_empty() => missing("aggregate, and no aggregate.json exists"),
        _ => Vec::new(),
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
            graph: None,
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
                "crates/credit/data-capabilities/request-limit-change/data-capability.json",
                r#"{"dataCapability":"credit.requestLimitChange","owner":"dataguard","effect":"update","target":{"aggregate":"Account"},"touches":["Account.creditLimit"]}"#,
            )],
            aggregates: vec![manifest(
                "crates/credit/aggregates/Account/aggregate.json",
                r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative"}},"invariants":[],"relations":[]}"#,
            )],
            unit_crates: Vec::new(),
            registry: None,
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
            design: None,
            kit: Vec::new(),
            root_manifest: None,
            registered_queries: Vec::new(),
            reads: Vec::new(),
        }
    }

    #[test]
    fn a_mutation_targets_an_aggregate_that_declares_what_it_touches() {
        let with = |data_capability: &str, aggregate: Option<&str>| {
            let mut tree = sound();
            tree.data_capabilities = vec![manifest(
                "crates/credit/data-capabilities/x/data-capability.json",
                data_capability,
            )];
            tree.aggregates = aggregate
                .map(|a| {
                    vec![manifest(
                        "crates/credit/aggregates/Account/aggregate.json",
                        a,
                    )]
                })
                .unwrap_or_default();
            violations(&tree, None)
        };
        let account =
            r#"{"aggregate":"Account","fields":{"creditLimit":{}},"invariants":[],"relations":[]}"#;
        let update = r#"{"dataCapability":"credit.x","owner":"dataguard","effect":"update","target":{"aggregate":"Account"},"touches":["Account.creditLimit"]}"#;
        assert!(
            !with(update, Some(account))
                .iter()
                .any(|v| v.contains("contract I")),
            "a declared field"
        );
        let elsewhere = with(update, None);
        assert!(
            elsewhere
                .iter()
                .any(|v| v.contains("no aggregate.json declares")),
            "{elsewhere:?}"
        );
        let undeclared = with(
            r#"{"dataCapability":"credit.x","owner":"dataguard","effect":"insert","target":{"aggregate":"Account"},"touches":["Account.tags"]}"#,
            Some(account),
        );
        assert!(
            undeclared
                .iter()
                .any(|v| v.contains("touches `Account.tags`")),
            "{undeclared:?}"
        );
        let pure = with(
            r#"{"dataCapability":"credit.x","owner":"dataguard","effect":"insert","target":{"aggregate":"Nobody"},"touches":[]}"#,
            None,
        );
        assert!(
            !pure.iter().any(|v| v.contains("contract I")),
            "a pure insert owns nothing: {pure:?}"
        );
    }

    #[test]
    fn a_capability_holding_a_database_driver_is_a_violation_but_a_dev_one_is_not() {
        let with = |toml: &str| {
            let mut tree = sound();
            tree.capabilities[0].cargo_toml = Some(toml.to_string());
            violations(&tree, None)
        };
        let driver = with("[package]\nname = \"risk\"\n\n[dependencies]\nsqlx = \"0.8\"\n");
        assert!(
            driver.iter().any(|v| v.contains("database driver")),
            "{driver:?}"
        );
        let workspace = with("[dependencies]\ntokio_postgres.workspace = true\n");
        assert!(
            workspace.iter().any(|v| v.contains("database driver")),
            "{workspace:?}"
        );
        let dev = with("[dependencies]\nserde = \"1\"\n\n[dev-dependencies]\nsqlx = \"0.8\"\n");
        assert!(
            !dev.iter().any(|v| v.contains("database driver")),
            "{dev:?}"
        );
        let read = with("[dependencies]\ncredit-read = { path = \"../../read\" }\n");
        assert!(
            !read.iter().any(|v| v.contains("database driver")),
            "{read:?}"
        );
    }

    #[test]
    fn a_dependency_line_names_its_crate_whatever_the_spelling() {
        assert!(names_crate("sqlx = \"0.8\"", "sqlx"));
        assert!(names_crate("sqlx.workspace = true", "sqlx"));
        assert!(names_crate(
            "tokio_postgres = { version = \"0.7\" }",
            "tokio-postgres"
        ));
        assert!(!names_crate("sqlx-helper = \"1\"", "sqlx"));
        assert!(!names_crate("serde = \"1\"", "sqlx"));
        assert_eq!(
            runtime_dependencies(
                "[dependencies]\na = \"1\"\n# b = \"2\"\n\n[dev-dependencies]\nc = \"3\"\n[build-dependencies]\nd = \"4\"\n"
            ),
            ["a = \"1\""]
        );
    }

    #[test]
    fn a_read_is_registered_under_its_capability_with_the_hash_of_the_file_as_it_is() {
        let sha = sha256("query X { a }");
        let registered = |capability: &str, file: &str, sha: &str| Tree {
            registered_queries: vec![(capability.to_string(), file.to_string(), sha.to_string())],
            reads: vec![Read {
                capability: "credit.calculateRisk".to_string(),
                written: "queries/x.graphql".to_string(),
                found: Some((
                    "crates/credit/queries/x.graphql".to_string(),
                    sha256("query X { a }"),
                )),
            }],
            ..Tree::default()
        };
        let fine = queries_registered(&registered(
            "credit.calculateRisk",
            "crates/credit/queries/x.graphql",
            &sha,
        ));
        assert!(fine.is_empty(), "{fine:?}");
        let stale = queries_registered(&registered(
            "credit.calculateRisk",
            "crates/credit/queries/x.graphql",
            "0000",
        ));
        assert!(stale[0].contains("changed since"), "{stale:?}");
        let elsewhere = queries_registered(&registered(
            "orders.list",
            "crates/credit/queries/x.graphql",
            &sha,
        ));
        assert!(elsewhere[0].contains("does not register"), "{elsewhere:?}");
        let missing = queries_registered(&Tree {
            reads: vec![Read {
                capability: "credit.calculateRisk".to_string(),
                written: "queries/gone.graphql".to_string(),
                found: None,
            }],
            ..Tree::default()
        });
        assert!(missing[0].contains("nowhere"), "{missing:?}");
    }

    #[test]
    fn the_reads_are_resolved_from_the_root_the_folder_or_the_system_and_hashed() {
        let mut disk = crate::fake_disk::FakeDisk::default();
        let put = |disk: &mut crate::fake_disk::FakeDisk, path: &str, text: &str| {
            disk.existing
                .insert(std::path::PathBuf::from(path), text.to_string());
        };
        put(&mut disk, "/r/contracts/README.md", "");
        put(
            &mut disk,
            "/r/crates/credit/capabilities/risk/capability.json",
            r#"{"capability":"credit.calculateRisk","system":"credit","reads":["queries/x.graphql","local.sql","nowhere.sql"]}"#,
        );
        put(
            &mut disk,
            "/r/crates/credit/capabilities/risk/Cargo.toml",
            "[package]\n",
        );
        put(
            &mut disk,
            "/r/crates/credit/queries/x.graphql",
            "query X { a }",
        );
        put(
            &mut disk,
            "/r/crates/credit/capabilities/risk/local.sql",
            "select 1",
        );
        put(
            &mut disk,
            "/r/queries/registry.json",
            &format!(
                r#"{{"queries":[{{"query":"X","capability":"credit.calculateRisk","file":"crates/credit/queries/x.graphql","sha256":"{}","fields":["a"]}}]}}"#,
                sha256("query X { a }")
            ),
        );
        let tree = Tree::read(&disk, std::path::Path::new("/r"));
        assert_eq!(tree.registered_queries.len(), 1);
        assert_eq!(tree.reads.len(), 3);
        assert_eq!(
            tree.reads[0].found.as_ref().map(|(p, _)| p.as_str()),
            Some("crates/credit/queries/x.graphql"),
            "from the system's folder"
        );
        assert_eq!(
            tree.reads[1].found.as_ref().map(|(p, _)| p.as_str()),
            Some("crates/credit/capabilities/risk/local.sql"),
            "from the Capability's folder"
        );
        assert_eq!(tree.reads[2].found, None, "nowhere");
        let found = queries_registered(&tree);
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn the_hash_is_sha256_as_the_script_writes_it() {
        assert_eq!(
            sha256("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn what_the_plan_s_graph_produces_counts_as_declared() {
        let mut tree = sound();
        tree.micro_uis[0].1 = manifest(
            "apps/credit/risk-badge/micro-ui.json",
            r#"{"microUi":"RiskBadge","system":"credit","needs":["credit.calculateRisk","credit.listAccounts"],"actions":["credit.closeAccount"]}"#,
        );
        let undeclared = |tree: &Tree| {
            violations(tree, None)
                .into_iter()
                .filter(|v| v.contains("no manifest declares"))
                .collect::<Vec<_>>()
        };
        assert_eq!(undeclared(&tree).len(), 2, "{:?}", undeclared(&tree));
        tree.graph = crate::graph::Graph::parse(
            r#"{"version":1,"milestones":[],"ids":{"credit.listAccounts":{"kind":"capability","producer":7},"credit.closeAccount":{"kind":"data-capability","producer":7}}}"#,
        )
        .ok();
        assert!(undeclared(&tree).is_empty(), "{:?}", undeclared(&tree));
        assert!(tree.plans(crate::graph::Kind::Capability, "credit.listAccounts"));
        assert!(
            !tree.plans(crate::graph::Kind::DataCapability, "credit.listAccounts"),
            "the kind counts"
        );
        tree.graph = crate::graph::Graph::parse(
            r#"{"version":1,"milestones":[],"ids":{"credit.listAccounts":{"kind":"capability","producer":null}}}"#,
        )
        .ok();
        assert!(
            !tree.plans(crate::graph::Kind::Capability, "credit.listAccounts"),
            "nobody's"
        );
    }

    #[test]
    fn a_data_capability_is_read_under_any_system() {
        // #43: ai.saveSettings sits under crates/ai/, as the rules lay it out.
        // The DataGuard's folder is read too — not as a home, so that
        // `violations` can name the one it belongs in.
        let mut disk = crate::fake_disk::FakeDisk::default();
        for (path, text) in [
            (
                "/r/crates/ai/data-capabilities/save-settings/data-capability.json",
                r#"{"dataCapability":"ai.saveSettings"}"#,
            ),
            (
                "/r/crates/dataguard/data-capabilities/limit/data-capability.json",
                r#"{"dataCapability":"credit.limit"}"#,
            ),
        ] {
            disk.existing
                .insert(std::path::PathBuf::from(path), text.to_string());
        }
        let tree = Tree::read(&disk, std::path::Path::new("/r"));
        let paths: Vec<&str> = tree
            .data_capabilities
            .iter()
            .map(|m| m.path.as_str())
            .collect();
        assert!(
            paths.contains(&"crates/ai/data-capabilities/save-settings/data-capability.json"),
            "{paths:?}"
        );
        assert!(
            paths.contains(&"crates/dataguard/data-capabilities/limit/data-capability.json"),
            "{paths:?}"
        );
        let found = violations(&tree, None);
        assert!(
            found.iter().any(|v| {
                v.starts_with("crates/dataguard/data-capabilities/limit/")
                    && v.contains("belongs under crates/credit/data-capabilities/limit/")
            }),
            "{found:?}"
        );
    }

    #[test]
    fn a_data_capability_sits_under_the_system_its_id_names_never_under_the_dataguard() {
        // ForgeDesMondes milestone 18: campaign.saveEntity was laid under
        // crates/dataguard/data-capabilities/ while ai.saveSettings sat
        // under crates/ai/, and nothing said which was the rule.
        const SAVE: &str = r#"{"dataCapability":"campaign.saveEntity","owner":"dataguard","effect":"update","target":{"aggregate":"Account"},"touches":["Account.creditLimit"]}"#;
        // What the rules say of that one manifest (the Micro-UI of `sound`
        // then triggers a DataCapability nobody declares: not this test's).
        let with = |path: &str, json: &str| {
            let mut tree = sound();
            tree.data_capabilities = vec![manifest(path, json)];
            violations(&tree, None)
                .into_iter()
                .filter(|v| v.starts_with(path))
                .collect::<Vec<_>>()
        };
        let placed = with(
            "crates/campaign/data-capabilities/save-entity/data-capability.json",
            SAVE,
        );
        assert!(
            !placed.iter().any(|v| v.contains("namespace")),
            "its id's namespace is its folder: {placed:?}"
        );
        let under_the_dataguard = with(
            "crates/dataguard/data-capabilities/save-entity/data-capability.json",
            SAVE,
        );
        assert_eq!(
            under_the_dataguard,
            [
                "crates/dataguard/data-capabilities/save-entity/data-capability.json: \
              `campaign.saveEntity` belongs under crates/campaign/data-capabilities/save-entity/, \
              its id's namespace — the DataGuard owns a DataCapability (`owner: dataguard`), it \
              does not house it"
            ]
        );
        // Any other system's folder is as wrong as the DataGuard's.
        let elsewhere = with(
            "crates/media/data-capabilities/save-entity/data-capability.json",
            SAVE,
        );
        assert!(
            elsewhere
                .iter()
                .any(|v| v.contains("belongs under crates/campaign/data-capabilities/save-entity/")),
            "{elsewhere:?}"
        );
        // `dataguard` is the owner of every DataCapability, the system of none.
        let owner_as_system = with(
            "crates/dataguard/data-capabilities/limit/data-capability.json",
            r#"{"dataCapability":"dataguard.limit","owner":"dataguard","effect":"insert","touches":[]}"#,
        );
        assert!(
            owner_as_system.iter().any(|v| {
                v.contains("`dataguard.limit` names no system as its id's namespace")
                    && v.contains("crates/<system>/data-capabilities/limit/")
            }),
            "{owner_as_system:?}"
        );
        let no_namespace = with(
            "crates/credit/data-capabilities/limit/data-capability.json",
            r#"{"dataCapability":"limit","owner":"dataguard","effect":"insert","touches":[]}"#,
        );
        assert!(
            no_namespace
                .iter()
                .any(|v| v.contains("`limit` names no system as its id's namespace")),
            "{no_namespace:?}"
        );
        let no_id = with(
            "crates/credit/data-capabilities/limit/data-capability.json",
            r#"{"owner":"dataguard","effect":"insert","touches":[]}"#,
        );
        assert!(
            no_id.iter().any(|v| {
                v == "crates/credit/data-capabilities/limit/data-capability.json names no \
                      `dataCapability`"
            }),
            "{no_id:?}"
        );
    }

    #[test]
    fn the_root_manifest_is_read_with_the_tree() {
        let mut disk = crate::fake_disk::FakeDisk::default();
        disk.existing.insert(
            std::path::PathBuf::from("/r/Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*/capabilities/*\"]\n".to_string(),
        );
        disk.existing.insert(
            std::path::PathBuf::from("/r/contracts/README.md"),
            String::new(),
        );
        let tree = Tree::read(&disk, std::path::Path::new("/r"));
        assert!(
            tree.root_manifest
                .as_deref()
                .is_some_and(|toml| toml.contains("members")),
            "{:?}",
            tree.root_manifest
        );
        let bare = Tree::read(
            &crate::fake_disk::FakeDisk::default(),
            std::path::Path::new("/none"),
        );
        assert_eq!(bare.root_manifest, None);
    }

    #[test]
    fn a_slice_listed_as_a_member_by_name_is_a_violation_and_plumbing_is_not() {
        let listed = |members: &str| Tree {
            root_manifest: Some(format!("[workspace]\nmembers = [{members}]\n")),
            ..sound()
        };
        let by_name = violations(&listed("\"crates/credit/capabilities/risk\""), None);
        assert!(
            by_name
                .iter()
                .any(|v| v.contains("listed as a member by name")),
            "{by_name:?}"
        );
        let data = violations(&listed("\"crates/credit/data-capabilities/add\""), None);
        assert!(data.iter().any(|v| v.contains("by name")), "{data:?}");
        for fine in [
            "\"crates/*/capabilities/*\"",
            "\"crates/dataguard/server\"",
            "\"crates/credit/capabilities\"",
            "\"apps/credit/capabilities/risk\"",
            "\"crates/credit/queries/x\"",
        ] {
            let found = violations(&listed(fine), None);
            assert!(
                !found.iter().any(|v| v.contains("by name")),
                "{fine}: {found:?}"
            );
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
        assert!(violations(&tree, None).is_empty(), "expected nothing");
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
        assert!(violations(&bare, None).is_empty(), "expected nothing");
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
                aggregate: None,
                produces: Vec::new(),
                needs: Vec::new(),
                profile: None,
                host: None,
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
    fn a_capability_whose_test_reaches_the_engine_breaks_no_rule() {
        let mut tree = sound();
        tree.capabilities[0].cargo_toml = Some(
            "[dependencies]\nserde_json = \"1\"\n\n[dev-dependencies]\nharness-dataguard = { workspace = true }\nother = { path = \"../../capabilities/x\" }\n".to_string(),
        );
        assert_eq!(violations(&tree, None), Vec::<String>::new());
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

        // ForgeDesMondes #42: what the llm port received, read through it.
        tree.capabilities[0].manifest = Some(manifest(
            "crates/credit/capabilities/calculate-risk/capability.json",
            r#"{"capability":"credit.calculateRisk","system":"credit","reads":[],"ports":["llm"]}"#,
        ));
        let found = violations(&tree, None);
        assert!(
            !found.iter().any(|f| f.contains("no persisted query")),
            "{found:?}"
        );
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
            !found.iter().any(|f| f.contains("needs `orders.list`")),
            "reading another system's Capability is free"
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
            "crates/credit/data-capabilities/x/data-capability.json",
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
            aggregate: None,
            produces: Vec::new(),
            needs: Vec::new(),
            profile: None,
            host: None,
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
        existing.insert(
            root.join("crates/credit/capabilities/calculate-risk/q.graphql"),
            "# fields: a.b\nquery Q { a }".to_string(),
        );
        existing.insert(
            root.join("queries/registry.json"),
            format!(
                r#"{{"queries":[{{"query":"Q","capability":"credit.calculateRisk","file":"crates/credit/capabilities/calculate-risk/q.graphql","sha256":"{}","fields":["a.b"]}}]}}"#,
                sha256("# fields: a.b\nquery Q { a }")
            ),
        );
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
            root.join("mock/design/design.json"),
            r#"{"version":1,"state":"validated"}"#.to_string(),
        );
        existing.insert(
            root.join("mock/kit/kit.json"),
            r#"{"version":1,"components":[{"name":"Button","title":"Button","state":"generated"},{"name":"Badge","title":"Badge","state":"pending"},{"name":"HeroFrame","title":"Hero frame","state":"generated"}]}"#.to_string(),
        );
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
        // The design system and the kit, as the UI builds on them: generated components only.
        assert_eq!(
            tree.design
                .as_ref()
                .and_then(|m| m.json.as_ref())
                .and_then(|j| str_of(j, "state")),
            Some("validated")
        );
        assert_eq!(
            tree.kit,
            [
                ("Button".to_string(), "Button".to_string()),
                ("HeroFrame".to_string(), "Hero frame".to_string())
            ]
        );
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
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].contains("registry"), "a unit crate, no registry");
        assert!(found[1].contains("pages/gone.html"));
        assert!(!Tree::read(&FakeDisk::default(), &root).under_architecture);
    }

    #[test]
    fn a_handler_never_enters_a_bundle_digest_and_the_shared_server_does() {
        let root = PathBuf::from("/ws");
        let mut existing = HashMap::new();
        existing.insert(root.join("mock/src/RiskBadge.js"), "mount();".to_string());
        existing.insert(
            root.join("apps/credit/risk-badge/src/index.ts"),
            "export {}".to_string(),
        );
        let digest_of = |existing: &HashMap<PathBuf, String>| {
            let disk = FakeDisk {
                existing: existing.clone(),
            };
            digest(&bundle_inputs(
                &disk,
                &root,
                "RiskBadge",
                "apps/credit/risk-badge",
            ))
        };
        assert_eq!(digest_of(&existing), "c14e3564be9bd6b2");
        for handler in [
            "mock/handlers/credit.tag.js",
            "mock/handlers/_lib/credit.js",
        ] {
            existing.insert(root.join(handler), "export const x = 1;".to_string());
        }
        assert_eq!(
            digest_of(&existing),
            "c14e3564be9bd6b2",
            "a handler file is imported at run time, never digested"
        );
        // The legacy files count while they are there, as an older build.mjs
        // counts them; once split away, the digest is back to the bare one.
        let mut legacy = existing.clone();
        legacy.insert(root.join("mock/src/commands.js"), "c".to_string());
        let with_commands = digest_of(&legacy);
        assert_ne!(with_commands, "c14e3564be9bd6b2", "commands.js counts");
        legacy.insert(root.join("mock/src/reads.js"), "r".to_string());
        assert_ne!(digest_of(&legacy), with_commands, "and reads.js");
        existing.insert(root.join("mock/src/server.js"), "server".to_string());
        let with_server = digest_of(&existing);
        assert_ne!(with_server, "c14e3564be9bd6b2", "every bundle embeds it");
        existing.insert(root.join("mock/src/screen.js"), "screen".to_string());
        assert_ne!(digest_of(&existing), with_server, "and the screen host");
    }

    #[test]
    fn what_every_bundle_embeds_or_builds_with_stales_them_all() {
        for path in [
            "mock/src/server.js",
            "mock/src/screen.js",
            "mock/src/commands.js",
            "mock/src/reads.js",
            "mock/build.mjs",
            "mock/build.sh",
        ] {
            assert!(stales_every_bundle(path), "{path}");
        }
        for path in [
            "mock/src/RiskBadge.js",
            "mock/handlers/credit.tag.js",
            "mock/pages.json",
            "src/server.js",
            "mock/src/server.json",
            "scripts/split-handlers.mjs",
        ] {
            assert!(!stales_every_bundle(path), "{path}");
        }
    }

    #[test]
    fn the_unit_crates_are_read_with_their_host() {
        let mut disk = FakeDisk::default();
        for (path, text) in [
            (
                "/r/crates/credit/capabilities/risk/Cargo.toml",
                "[package]\nname = \"risk\"\n",
            ),
            (
                "/r/crates/credit/data-capabilities/export/Cargo.toml",
                "[package]\nname = \"export\"\n",
            ),
            (
                "/r/crates/credit/data-capabilities/export/data-capability.json",
                r#"{ "dataCapability": "credit.export", "host" : "hostless" }"#,
            ),
            (
                "/r/crates/credit/aggregates/Account/aggregate.json",
                r#"{"aggregate":"Account"}"#,
            ),
        ] {
            disk.existing.insert(PathBuf::from(path), text.to_string());
        }
        let tree = Tree::read(&disk, Path::new("/r"));
        assert_eq!(
            tree.unit_crates
                .iter()
                .map(|u| (u.dir.as_str(), u.hostless))
                .collect::<Vec<_>>(),
            [
                ("crates/credit/data-capabilities/export", true),
                ("crates/credit/capabilities/risk", false)
            ],
            "by kind, aggregates first; a folder without a Cargo.toml is no crate"
        );
        assert_eq!(tree.registry, None);
        assert_eq!(tree.aggregates.len(), 1);
    }

    #[test]
    fn an_aggregate_is_read_under_any_system_and_placed_under_its_own() {
        let mut disk = FakeDisk::default();
        for (path, text) in [
            (
                "/r/crates/credit/aggregates/Account/aggregate.json",
                r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative"}}}"#,
            ),
            (
                "/r/crates/dataguard/aggregates/Ledger/aggregate.json",
                r#"{"aggregate":"Ledger","fields":{}}"#,
            ),
            (
                "/r/crates/credit/data-capabilities/post/data-capability.json",
                r#"{"dataCapability":"credit.post","owner":"dataguard","effect":"insert","target":{"aggregate":"Ledger"},"touches":[]}"#,
            ),
        ] {
            disk.existing.insert(PathBuf::from(path), text.to_string());
        }
        let tree = Tree::read(&disk, Path::new("/r"));
        assert_eq!(tree.aggregates.len(), 2, "{:?}", tree.aggregates);
        let found = violations(&tree, None);
        assert_eq!(
            found,
            [
                "crates/dataguard/aggregates/Ledger/aggregate.json: `Ledger` belongs under \
                 crates/credit/aggregates/Ledger/, an aggregate's folder in its system — the \
                 DataGuard judges every aggregate, it houses none"
            ]
        );
    }

    #[test]
    fn an_aggregate_s_folder_is_its_name_and_its_system_is_its_targeters() {
        let with = |path: &str, json: &str, targeting: &[&str]| {
            let mut tree = sound();
            tree.aggregates = vec![manifest(path, json)];
            tree.data_capabilities = targeting
                .iter()
                .map(|id| {
                    manifest(
                        "crates/x/data-capabilities/y/data-capability.json",
                        &format!(
                            r#"{{"dataCapability":"{id}","target":{{"aggregate":"Ledger"}}}}"#
                        ),
                    )
                })
                .collect();
            aggregate_misplaced(
                path,
                tree.aggregates[0]
                    .json
                    .as_ref()
                    .and_then(|j| str_of(j, "aggregate")),
                &tree,
            )
        };
        let ledger = r#"{"aggregate":"Ledger"}"#;
        assert_eq!(
            with(
                "crates/credit/aggregates/Ledger/aggregate.json",
                ledger,
                &[]
            ),
            None
        );
        let renamed = with(
            "crates/credit/aggregates/Ledgers/aggregate.json",
            ledger,
            &[],
        );
        assert!(
            renamed.as_deref().is_some_and(|why| why.contains(
                "`Ledger` belongs under crates/credit/aggregates/Ledger/, an aggregate's folder named after it"
            )),
            "{renamed:?}"
        );
        for (targeting, home) in [
            (&[][..], "<system>"),
            (&["credit.post", "credit.void"][..], "credit"),
            (&["credit.post", "orders.post"][..], "<system>"),
            (&["dataguard.post", "post"][..], "<system>"),
        ] {
            let why = with(
                "crates/dataguard/aggregates/Ledger/aggregate.json",
                ledger,
                targeting,
            )
            .unwrap_or_default();
            assert!(
                why.contains(&format!("belongs under crates/{home}/aggregates/Ledger/")),
                "{targeting:?}: {why}"
            );
        }
        assert_eq!(
            with("crates/credit/aggregates/Ledger/aggregate.json", "{}", &[]).as_deref(),
            Some("crates/credit/aggregates/Ledger/aggregate.json names no `aggregate`")
        );
        let mut broken = sound();
        broken.aggregates = vec![manifest(
            "crates/credit/aggregates/Ledger/aggregate.json",
            "{",
        )];
        assert!(
            violations(&broken, None)
                .iter()
                .any(|v| v == "crates/credit/aggregates/Ledger/aggregate.json is not JSON"),
            "{:?}",
            violations(&broken, None)
        );
    }

    #[test]
    fn a_data_capability_touches_no_derived_field() {
        let mut tree = sound();
        tree.aggregates = vec![manifest(
            "crates/credit/aggregates/Account/aggregate.json",
            r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"derived"},"tags":{"kind":"authoritative"}}}"#,
        )];
        let found = violations(&tree, None);
        assert_eq!(
            found,
            [
                "crates/credit/data-capabilities/request-limit-change/data-capability.json: touches \
                 `Account.creditLimit`, which the aggregate `Account` declares derived — recomputed, \
                 never written (contract I)"
            ]
        );
    }

    #[test]
    fn an_aggregate_listed_as_a_member_by_name_is_a_violation() {
        let tree = Tree {
            root_manifest: Some(
                "[workspace]\nmembers = [\"crates/credit/aggregates/Account\"]\n".to_string(),
            ),
            ..sound()
        };
        let found = violations(&tree, None);
        assert!(
            found.iter().any(|v| v
                .contains("crates/credit/aggregates/Account is a unit listed as a member by name")),
            "{found:?}"
        );
    }

    /// A registry that names no unit: cargo machete ignores nothing.
    const NOTHING_IGNORED: &str = "[package.metadata.cargo-machete]\nignored = []\n\n";

    fn unit(dir: &str, toml: &str, hostless: bool) -> UnitCrate {
        UnitCrate {
            dir: dir.to_string(),
            cargo_toml: toml.to_string(),
            hostless,
        }
    }

    #[test]
    fn every_unit_crate_is_named_in_the_registry_and_no_other() {
        let account = unit(
            "crates/credit/aggregates/Account",
            "[package]\nname = \"aggregate-account\"\n",
            false,
        );
        let export = unit(
            "crates/credit/data-capabilities/export",
            "[package]\nname = \"export\"\n",
            true,
        );
        let with = |units: Vec<UnitCrate>, registry: Option<&str>| {
            units_registered(&Tree {
                unit_crates: units,
                registry: registry.map(ToString::to_string),
                ..Tree::default()
            })
        };
        assert!(with(Vec::new(), None).is_empty(), "nothing to register");
        assert_eq!(
            with(vec![export.clone()], None),
            [
                "crates/dataguard/registry/Cargo.toml is missing and 1 unit crate(s) wait on the \
                 registry — run scripts/register.sh"
            ],
            "a hostless unit too: the script writes the registry, naming none"
        );
        assert_eq!(
            with(vec![account.clone(), export.clone()], None).len(),
            1,
            "one line for the missing registry"
        );
        let named = "[package.metadata.cargo-machete]\nignored = [\n    \"aggregate-account\",\n]\n\n\
                     [dependencies]\nharness-dataguard = { git = \"x\" }\n\
                     aggregate-account = { path = \"../../credit/aggregates/Account\" }\n";
        assert_eq!(
            with(vec![account.clone(), export], Some(named)),
            Vec::<String>::new()
        );
        assert_eq!(
            with(
                vec![account],
                Some(&format!("{NOTHING_IGNORED}[dependencies]\n"))
            ),
            [
                "crates/credit/aggregates/Account: a unit crate the registry does not name — run \
                 scripts/register.sh"
            ]
        );
        assert_eq!(
            with(Vec::new(), Some(named)),
            [
                "crates/dataguard/registry/Cargo.toml names crates/credit/aggregates/Account, \
                 where the registry finds no unit crate — run scripts/register.sh"
            ]
        );
    }

    #[test]
    fn a_unit_s_library_is_named_as_the_registry_s_glue_calls_it() {
        let with = |dir: &str, toml: &str| {
            units_registered(&Tree {
                unit_crates: vec![unit(dir, toml, true)],
                registry: Some(NOTHING_IGNORED.to_string()),
                ..Tree::default()
            })
        };
        assert_eq!(
            with(
                "crates/ai/aggregates/AiSettings",
                "[package]\nname = \"aggregate-ai-settings\"\n"
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            with(
                "crates/ai/data-capabilities/save-settings",
                "[package]\nname = \"save-settings\"\n"
            ),
            Vec::<String>::new()
        );
        assert!(
            with(
                "crates/ai/capabilities/anything",
                "[package]\nname = \"else\"\n"
            )
            .is_empty(),
            "a Capability's library is not judged here"
        );
        assert_eq!(
            with(
                "crates/ai/aggregates/AiSettings",
                "[package]\nname = \"aggregate-ai-settings\"\n\n[lib]\nname = \"settings\"\n"
            ),
            [
                "crates/ai/aggregates/AiSettings/Cargo.toml: the library of the aggregate \
                 AiSettings is named settings — name it aggregate_ai_settings: the registry's \
                 glue calls it by that name"
            ]
        );
        assert_eq!(
            with(
                "crates/ai/data-capabilities/save-settings",
                "[dependencies]\n"
            ),
            [
                "crates/ai/data-capabilities/save-settings/Cargo.toml: the library of the \
                 DataCapability save-settings is named nothing — name it save_settings: the \
                 registry's glue calls it by that name"
            ]
        );
        assert!(
            with("crates/ai", "[package]\nname = \"ai\"\n").is_empty(),
            "not a unit folder"
        );
    }

    #[test]
    fn a_unit_takes_the_engine_from_the_workspace_only() {
        for inherited in [
            "[dependencies]\nharness-dataguard = { workspace = true }\n",
            "[dependencies]\nharness-dataguard = { workspace = true, features = [\"postgres\"] }\n",
            "[dependencies]\nharness-dataguard.workspace = true\n",
            "[dependencies.harness-dataguard]\nworkspace = true\n[lib]\nname = \"x\"\n",
            "[dependencies.harness-dataguard]\nworkspace = true\n",
            "[dependencies]\nserde_json = \"1\"\n",
            "[package]\nname = \"harness-dataguard-like\"\n",
        ] {
            assert!(!engine_elsewhere(inherited), "{inherited}");
        }
        for own in [
            "[dependencies]\nharness-dataguard = { git = \"x\", tag = \"dataguard-v2\" }\n",
            "[dependencies]\nharness-dataguard = { path = \"../dataguard\" }\n",
            "[dependencies]\nharness-dataguard = { workspace = false }\n",
            "[dependencies]\nharness-dataguard.git = \"x\"\n",
            "[dependencies]\nharness-dataguard.path = \"x\"\n",
            "[dependencies]\nharness-dataguard.version = \"2\"\n",
            "[dependencies]\nharness-dataguard.tag = \"x\"\n",
            "[dependencies]\nharness-dataguard.branch = \"x\"\n",
            "[dependencies]\nharness-dataguard.rev = \"x\"\n",
            "[dev-dependencies.harness-dataguard]\ngit = \"x\"\n",
            // A source that merely spells it is no inheritance.
            "[dependencies.harness-dataguard]\ngit = \"https://x/workspace=true\"\n",
            "[dependencies.harness-dataguard]\ngit = \"x\"\n[lib]\nname = \"x\"\n",
        ] {
            assert!(engine_elsewhere(own), "{own}");
        }
        assert!(
            !engine_elsewhere("[dependencies]\nharness-dataguard.features = [\"uuid\"]\n"),
            "a dotted key that names no source"
        );
        let found = units_registered(&Tree {
            unit_crates: vec![unit(
                "crates/credit/aggregates/Account",
                "[package]\nname = \"aggregate-account\"\n\n[dependencies]\nharness-dataguard = { git = \"x\" }\n",
                false,
            )],
            registry: Some(
                "[package.metadata.cargo-machete]\nignored = [\"aggregate-account\"]\n\
                 aggregate-account = { path = \"../../credit/aggregates/Account\" }\n"
                    .to_string(),
            ),
            ..Tree::default()
        });
        assert_eq!(
            found,
            [
                "crates/credit/aggregates/Account/Cargo.toml takes harness-dataguard from its own \
                 source — write harness-dataguard = { workspace = true }: the registry plants one \
                 engine"
            ]
        );
    }

    #[test]
    fn names_are_spelled_as_the_registry_s_build_script_spells_them() {
        assert_eq!(snake("AiSettings"), "ai_settings");
        assert_eq!(snake("Account"), "account");
        assert_eq!(snake("HTTPCache"), "httpcache");
        assert_eq!(snake("Level2Map"), "level2_map");
        assert_eq!(snake("my-Thing"), "my_thing");
        assert_eq!(
            lib_name("[package]\nname = \"a-b\"\n[lib]\nname = \"c\"\n").as_deref(),
            Some("c")
        );
        assert_eq!(
            lib_name("[dependencies]\nname = \"x\"\n[package]\nname = \"a-b\"\nversion = \"1\"\n")
                .as_deref(),
            Some("a_b")
        );
        assert_eq!(
            lib_name("[package]\nname = \"a\"\n[package]\nname = \"b\"\n").as_deref(),
            Some("a"),
            "the first name holds"
        );
        assert_eq!(
            lib_name("[lib]\nname = \"a\"\n[lib]\nname = \"b\"\n").as_deref(),
            Some("a"),
            "the first library name holds too"
        );
        assert_eq!(lib_name("[lib]\nnames = \"x\"\n"), None);
        assert_eq!(
            registered(
                "a = { path = \"../../credit/aggregates/A/\" }\n\
                 b = { path = \"../../credit/read\" }\n\
                 c = { path = \"../server\" }\n\
                 d = { path = \"../../credit/capabilities/risk\" }\n\
                 e = { path = \"../../credit/nothing/x\" }\n\
                 f = { path = \"../../credit/capabilities/risk/deeper\" }\n"
            ),
            [
                ("a".to_string(), "crates/credit/aggregates/A".to_string()),
                (
                    "d".to_string(),
                    "crates/credit/capabilities/risk".to_string()
                )
            ]
        );
    }

    #[test]
    fn the_registry_s_machete_list_is_read_as_the_script_writes_it() {
        assert_eq!(machete_ignored("[dependencies]\na = \"1\"\n"), None);
        assert_eq!(
            machete_ignored("[package.metadata.cargo-machete]\n\n[dependencies]\n"),
            None,
            "a section without its list"
        );
        assert_eq!(machete_ignored(NOTHING_IGNORED), Some(Vec::new()));
        assert_eq!(
            machete_ignored(
                "[package]\nname = \"r\"\n\n[package.metadata.cargo-machete]\n# a comment, \"x\"\n\
                 ignored_too = [\"y\"]\nignored = [\n    \"a\",\n    \"b\",\n]\n\n\
                 [dependencies]\nc = { path = \"../../s/aggregates/C\" }\n"
            ),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(
            machete_ignored("[package.metadata.cargo-machete]\nignored=[\"a\"]\n"),
            Some(vec!["a".to_string()])
        );
        assert_eq!(
            machete_ignored(
                "[package.metadata.cargo-machete]\n[dependencies]\nignored = [\"a\"]\n"
            ),
            None,
            "a list under another table is not machete's"
        );

        let check = |registry: &str| {
            units_registered(&Tree {
                unit_crates: Vec::new(),
                registry: Some(registry.to_string()),
                ..Tree::default()
            })
            .into_iter()
            .filter(|v| v.contains("cargo-machete"))
            .count()
        };
        let units =
            "b = { path = \"../../s/aggregates/B\" }\na = { path = \"../../s/capabilities/a\" }\n";
        let missing = format!("[package]\n[dependencies]\n{units}");
        let listed = |list: &str| {
            format!("[package.metadata.cargo-machete]\nignored = [{list}]\n[dependencies]\n{units}")
        };
        assert_eq!(check(&missing), 1, "no list");
        assert_eq!(check(&listed("\"a\", \"b\"")), 0, "sorted, every unit");
        assert_eq!(check(&listed("\"b\", \"a\"")), 1, "unsorted");
        assert_eq!(check(&listed("\"a\"")), 1, "one unit left out");
        assert_eq!(check(&listed("\"a\", \"b\", \"c\"")), 1, "one no unit");
        assert_eq!(check(NOTHING_IGNORED), 0, "no unit, nothing ignored");
    }
}
