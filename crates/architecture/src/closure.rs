//! The closure of a task's unit: the part of the checkout a session
//! building that one unit needs, and nothing else.
//!
//! The architecture promises that a slice is built from what it declares:
//! its own folder, the plumbing it reaches, the manifests of the ids it
//! `needs`, the Concepts it implements, the aggregate it targets, its mock
//! page. The loop mounts exactly that (`git sparse-checkout`, through the
//! `Repo` port): what is not in the closure is not on the disk, so a session
//! cannot read it — the bound is physical, not a sentence in a prompt.
//!
//! Pure: the checkout is read once as a [`Tree`], the workspace's crate graph
//! once as a [`CargoGraph`] (what `cargo metadata` answers), and the closure
//! is computed from the two and the task's `## Architecture`. A test hands
//! it a tree it built by hand.
//!
//! The patterns are `git sparse-checkout`'s own, in its non-cone syntax: a
//! leading `/` anchors at the root, a trailing `/` names a folder, `!`
//! takes back what an earlier line gave. The first two lines keep every
//! file at the root (`Cargo.toml`, `package.json`, `CLAUDE.md`) and no
//! folder, then each folder the closure needs is added back.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::tree::Tree;
use crate::vocabulary::{Declaration, Unit};

/// A crate of the product's workspace, as `cargo metadata` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crate {
    /// The package name.
    pub name: String,
    /// Its folder, relative to the workspace root, no trailing slash.
    pub dir: String,
    /// The names of the workspace crates it depends on directly.
    pub deps: Vec<String>,
}

/// The workspace's crate graph: who depends on whom, by path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CargoGraph {
    /// Every workspace crate.
    pub crates: Vec<Crate>,
}

impl CargoGraph {
    /// Reads what `cargo metadata --format-version 1 --no-deps` printed.
    ///
    /// `None` when the text is not that: a product with no Cargo workspace (a
    /// TypeScript host) answers nothing, and the closure then narrows by
    /// folder alone.
    #[must_use]
    pub fn parse(metadata: &str) -> Option<Self> {
        let json: Value = serde_json::from_str(metadata).ok()?;
        let root = json.get("workspace_root")?.as_str()?.trim_end_matches('/');
        let packages = json.get("packages")?.as_array()?;
        let crates = packages
            .iter()
            .filter_map(|package| {
                let name = package.get("name")?.as_str()?.to_string();
                let manifest = package.get("manifest_path")?.as_str()?;
                let rel = manifest.strip_prefix(root)?.trim_start_matches('/');
                let dir = rel.strip_suffix("/Cargo.toml").unwrap_or(rel).to_string();
                let deps = package
                    .get("dependencies")
                    .and_then(Value::as_array)
                    .map(|deps| {
                        deps.iter()
                            .filter(|dep| dep.get("path").is_some_and(|p| !p.is_null()))
                            .filter_map(|dep| dep.get("name")?.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(Crate { name, dir, deps })
            })
            .collect();
        Some(Self { crates })
    }

    /// The crate whose folder is `dir`.
    #[must_use]
    pub fn owning(&self, dir: &str) -> Option<&Crate> {
        let dir = dir.trim_matches('/');
        self.crates.iter().find(|c| c.dir == dir)
    }

    /// The folders of every workspace crate `name` reaches, transitively,
    /// its own excluded. Sorted, so a prompt lists them stably.
    #[must_use]
    pub fn reached(&self, name: &str) -> Vec<String> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut queue = vec![name.to_string()];
        while let Some(current) = queue.pop() {
            let Some(found) = self.crates.iter().find(|c| c.name == current) else {
                continue;
            };
            for dep in &found.deps {
                if dep != name && seen.insert(dep.clone()) {
                    queue.push(dep.clone());
                }
            }
        }
        seen.into_iter()
            .filter_map(|dep| self.crates.iter().find(|c| c.name == dep))
            .map(|c| c.dir.clone())
            .collect()
    }
}

/// What a task's session gets of the checkout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Closure {
    /// The sparse-checkout patterns, in order. Empty when not narrowed.
    pub patterns: Vec<String>,
    /// The ids the task `needs` that the checkout does not hold: a task
    /// that cannot start, because what it builds on is not merged.
    pub missing: Vec<String>,
    /// False when the unit reaches the whole workspace, or the declaration
    /// does not say enough to narrow: `reason` says which.
    pub narrowed: bool,
    /// Why the tree stays whole, when it does.
    pub reason: String,
}

/// What every closure carries: the root's files, the CI, the scripts, the
/// contracts, the data model, the documents but the architecture's (the
/// rule card stands for it), the query registry, and the rules' tests a
/// task makes pass (two tasks stopped without them).
const ALWAYS: [&str; 12] = [
    "/*",
    "!/*/",
    "/.github/",
    "/.cargo/",
    "/scripts/",
    "/contracts/",
    "/data/",
    "/docs/",
    "!/docs/ARCHITECTURE.md",
    "!/docs/DATA-INTEGRITY.md",
    "/queries/",
    "/tests/rules/",
];

/// The folder of the product's `DataGuard` server, installed by the
/// harness: a `DataCapability` is a dependency line of it.
const SERVER: &str = "crates/dataguard/server";

/// The engine's manual: the write side reads it, nobody else.
const ENGINE_MANUAL: &str = "/docs/DATA-INTEGRITY.md";

/// The folders the root `Cargo.toml` lists as members by name — not by
/// glob. A glob matches what is on the disk, so a sparse tree resolves; a
/// name must be there, so every closure carries it.
#[must_use]
pub fn explicit_members(root_manifest: &str) -> Vec<String> {
    members_of(root_manifest)
        .into_iter()
        .filter(|entry| !entry.contains('*'))
        .collect()
}

/// Every entry of the root `Cargo.toml`'s `members`, trimmed, globs kept.
fn members_of(root_manifest: &str) -> Vec<String> {
    // Comments dropped first, and the key read where a line starts with it:
    // a comment naming `workspace-members-by-glob` once made `[workspace]`
    // read as the array.
    let code: String = root_manifest
        .lines()
        .map(|line| line.split_once('#').map_or(line, |(before, _)| before))
        .collect::<Vec<_>>()
        .join("\n");
    let mut offset = 0;
    let mut start = None;
    for line in code.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed
            .strip_prefix("members")
            .is_some_and(|rest| rest.trim_start().starts_with('='))
        {
            start = Some(offset);
            break;
        }
        offset += line.len();
    }
    let Some(start) = start else {
        return Vec::new();
    };
    let after = &code[start..];
    let Some(open) = after.find('[') else {
        return Vec::new();
    };
    let Some(close) = after[open..].find(']') else {
        return Vec::new();
    };
    after[open + 1..open + close]
        .split(',')
        .map(|entry| entry.trim().trim_matches(['"', '\'']).trim())
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.trim_matches('/').to_string())
        .collect()
}

/// A port's name: lowercase letters, digits and dashes — never a dotted
/// Capability id nor a versioned Concept.
fn port_like(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The members the root `Cargo.toml` lists by glob, as written.
fn glob_members(root_manifest: &str) -> Vec<String> {
    members_of(root_manifest)
        .into_iter()
        .filter(|entry| entry.contains('*'))
        .collect()
}

/// A folder pattern: anchored, trailing slash.
fn folder(dir: &str) -> String {
    format!("/{}/", dir.trim_matches('/'))
}

/// A folder taken back: a slice of the same system a task must not see.
fn not_folder(dir: &str) -> String {
    format!("!/{}/", dir.trim_matches('/'))
}

/// A single file pattern: anchored.
fn file(path: &str) -> String {
    format!("/{}", path.trim_start_matches('/'))
}

/// The folder a manifest sits in.
fn dir_of(path: &str) -> String {
    path.rsplit_once('/')
        .map_or_else(String::new, |(dir, _)| dir.to_string())
}

const fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Where the tree holds each kind of id.
struct Index<'a> {
    tree: &'a Tree,
}

impl Index<'_> {
    fn capability_dir(&self, id: &str) -> Option<String> {
        self.tree
            .capabilities
            .iter()
            .find(|c| {
                c.manifest
                    .as_ref()
                    .and_then(|m| m.json.as_ref())
                    .and_then(|j| j.get("capability")?.as_str())
                    .is_some_and(|found| same(found, id))
            })
            .map(|c| c.dir.clone())
    }

    fn data_capability_dir(&self, id: &str) -> Option<String> {
        self.tree
            .data_capabilities
            .iter()
            .find(|m| {
                m.json
                    .as_ref()
                    .and_then(|j| j.get("dataCapability")?.as_str())
                    .is_some_and(|found| same(found, id))
            })
            .map(|m| dir_of(&m.path))
    }

    fn aggregate_dir(&self, name: &str) -> Option<String> {
        self.tree
            .aggregates
            .iter()
            .find(|m| {
                m.json
                    .as_ref()
                    .and_then(|j| j.get("aggregate")?.as_str())
                    .is_some_and(|found| same(found, name))
            })
            .map(|m| dir_of(&m.path))
    }

    fn micro_ui_dir(&self, id: &str) -> Option<String> {
        self.tree
            .micro_uis
            .iter()
            .find(|(_, m)| {
                m.json
                    .as_ref()
                    .and_then(|j| j.get("microUi")?.as_str())
                    .is_some_and(|found| same(found, id))
            })
            .map(|(_, m)| dir_of(&m.path))
    }

    fn concept_dir(&self, id: &str) -> Option<String> {
        let (name, version) = id.split_once('@')?;
        let version: u64 = version.trim_start_matches('v').parse().ok()?;
        self.tree
            .concepts
            .iter()
            .find(|(found, n)| same(found, name) && *n == version)
            .map(|(found, n)| format!("concepts/{found}/v{n}"))
    }

    /// True when some unit of the checkout answers to this id.
    fn knows(&self, id: &str) -> bool {
        self.capability_dir(id).is_some()
            || self.data_capability_dir(id).is_some()
            || self.aggregate_dir(id).is_some()
            || self.micro_ui_dir(id).is_some()
            || self.concept_dir(id).is_some()
            || self.tree.screens.iter().any(|s| same(s, id))
            || self.tree.mocked_components.iter().any(|c| same(c, id))
    }
}

/// Adds `pattern` once, keeping the order of first addition.
fn add(patterns: &mut Vec<String>, pattern: String) {
    if !patterns.contains(&pattern) {
        patterns.push(pattern);
    }
}

/// The closure of `task` in this checkout.
///
/// `graph` is the workspace's crate graph when `cargo metadata` answered;
/// without it a Rust slice is narrowed by folder — its system's crates
/// minus the other slices — rather than by what it reaches.
/// `root_manifest` is the root `Cargo.toml`, for the members listed by name.
#[must_use]
pub fn closure(
    tree: &Tree,
    task: &Declaration,
    graph: Option<&CargoGraph>,
    root_manifest: Option<&str>,
) -> Closure {
    let index = Index { tree };
    let missing: Vec<String> = task
        .needs
        .iter()
        .filter(|id| !index.knows(id))
        .cloned()
        .collect();
    let whole = |reason: &str| Closure {
        patterns: Vec::new(),
        missing: missing.clone(),
        narrowed: false,
        reason: reason.to_string(),
    };
    if task.unit == Unit::Tooling {
        return whole("tooling reaches the whole workspace");
    }
    let system = task.system.trim();
    if system.is_empty() && task.unit != Unit::Infrastructure && task.unit != Unit::Concept {
        return whole("the task declares no system");
    }

    let mut patterns: Vec<String> = ALWAYS.iter().map(ToString::to_string).collect();
    for member in root_manifest.map(explicit_members).unwrap_or_default() {
        add(&mut patterns, folder(&member));
    }
    // A member glob that matches nothing fails the whole workspace: the
    // product keeps a README under each glob's parent so one always matches.
    for glob in root_manifest.map(glob_members).unwrap_or_default() {
        if let Some((parent, _)) = glob.rsplit_once('/') {
            add(&mut patterns, file(&format!("{parent}/README.md")));
        }
    }
    // A need the tree cannot place, named like a port, is its crate
    // (`crates/ports/<port>/`, contract Q): #42 needed `llm`.
    for id in task
        .needs
        .iter()
        .filter(|id| !index.knows(id) && port_like(id))
    {
        add(&mut patterns, folder(&format!("crates/ports/{id}")));
    }
    let own = task.produces.first().map(String::as_str);

    match task.unit {
        Unit::Capability | Unit::PersistedQuery | Unit::DataCapability => {
            let own_dir = own.and_then(|id| {
                if task.unit == Unit::DataCapability {
                    index.data_capability_dir(id)
                } else {
                    index.capability_dir(id)
                }
            });
            let reached = own_dir
                .as_deref()
                .and_then(|dir| graph.and_then(|g| g.owning(dir)))
                .map(|found| graph.map(|g| g.reached(&found.name)).unwrap_or_default());
            if let (Some(dir), Some(plumbing)) = (&own_dir, reached) {
                add(&mut patterns, folder(dir));
                for plumb in plumbing {
                    add(&mut patterns, folder(&plumb));
                }
            } else {
                // No graph, or a unit not built yet: its system's crates,
                // minus the slices it must not know.
                add(&mut patterns, folder(&format!("crates/{system}")));
                add(
                    &mut patterns,
                    not_folder(&format!("crates/{system}/capabilities/*")),
                );
                add(
                    &mut patterns,
                    not_folder(&format!("crates/{system}/data-capabilities/*")),
                );
                if let Some(dir) = &own_dir {
                    add(&mut patterns, folder(dir));
                }
            }
            add(&mut patterns, folder(&format!("crates/{system}/queries")));
            if task.unit == Unit::DataCapability {
                // The server's dependency line is the one shared file it
                // edits — unless no server runs it.
                if !task.is_hostless() {
                    add(&mut patterns, folder(SERVER));
                }
                if let Some(dir) = task
                    .aggregate
                    .as_deref()
                    .and_then(|a| index.aggregate_dir(a))
                {
                    add(&mut patterns, folder(&dir));
                }
            }
            add(&mut patterns, folder("mock/src"));
            add(&mut patterns, folder("mock/fixtures"));
        }
        Unit::Aggregate | Unit::Infrastructure => {
            add(&mut patterns, ENGINE_MANUAL.to_string());
            if system.is_empty() {
                add(&mut patterns, folder("crates"));
            } else {
                add(&mut patterns, folder(&format!("crates/{system}")));
            }
            add(&mut patterns, folder("mock/src"));
            add(&mut patterns, folder("mock/fixtures"));
        }
        Unit::MicroUi | Unit::Composition => {
            add(&mut patterns, folder("apps"));
            let own_dir = own.and_then(|id| index.micro_ui_dir(id));
            for (_, manifest) in &tree.micro_uis {
                let dir = dir_of(&manifest.path);
                if own_dir.as_deref() != Some(dir.as_str()) {
                    add(&mut patterns, not_folder(&dir));
                }
            }
            add(&mut patterns, folder("mock"));
            for id in &task.needs {
                if let Some(dir) = index.capability_dir(id) {
                    add(&mut patterns, file(&format!("{dir}/capability.json")));
                }
                if let Some(dir) = index.data_capability_dir(id) {
                    add(&mut patterns, file(&format!("{dir}/data-capability.json")));
                }
            }
        }
        Unit::Concept => {
            add(&mut patterns, folder("concepts"));
        }
        Unit::Tooling => unreachable!("returned above"),
    }

    // Every crate the tree carries brings what it depends on by path: cargo
    // loads none of the workspace otherwise. The server names every
    // DataCapability, an adapter its port (#40, #41 stopped on them).
    if let Some(g) = graph {
        let carried: Vec<String> = patterns
            .iter()
            .filter_map(|p| p.strip_prefix('/')?.strip_suffix('/'))
            .filter_map(|dir| g.owning(dir).map(|found| found.name.clone()))
            .collect();
        for name in carried {
            for dir in g.reached(&name) {
                add(&mut patterns, folder(&dir));
            }
        }
    }

    // The Concepts the task implements or names, whichever the unit.
    let named = task.concept.iter().chain(task.needs.iter());
    for id in named {
        if let Some(dir) = index.concept_dir(id) {
            add(&mut patterns, folder(&dir));
        }
    }
    Closure {
        patterns,
        missing,
        narrowed: true,
        reason: String::new(),
    }
}

/// Whether `path` (relative, no leading slash) is inside the closure.
///
/// The matching `git sparse-checkout` does, on the shapes [`closure`]
/// writes: `/*` (every file at the root), `!/*/` (no folder), `/dir/` and
/// `!/dir/` (a folder and its contents), `/dir/*/` (every folder directly
/// under one), `/file`. Later lines win.
#[must_use]
pub fn allows(patterns: &[String], path: &str) -> bool {
    let path = path.trim_start_matches('/');
    let mut allowed = false;
    for pattern in patterns {
        let (negated, body) = pattern
            .strip_prefix('!')
            .map_or((false, pattern.as_str()), |rest| (true, rest));
        if matches(body, path) {
            allowed = !negated;
        }
    }
    allowed
}

/// One pattern against one path.
fn matches(pattern: &str, path: &str) -> bool {
    let body = pattern.trim_start_matches('/');
    if body == "*" {
        return !path.contains('/');
    }
    if body == "*/" {
        return path.contains('/');
    }
    if let Some(dir) = body.strip_suffix('/') {
        return dir.strip_suffix("/*").map_or_else(
            || path == dir || path.starts_with(&format!("{dir}/")),
            // `/crates/s/capabilities/*/`: a folder directly under `parent`.
            |parent| {
                path.strip_prefix(parent)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .is_some_and(|rest| rest.contains('/'))
            },
        );
    }
    path == body
}

/// The closure as a prompt tells it: one folder or file per line.
#[must_use]
pub fn describe(closure: &Closure) -> String {
    closure
        .patterns
        .iter()
        .filter(|p| !p.starts_with('!') && *p != "/*")
        .map(|p| format!("- {p}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{CapabilityCrate, Manifest};
    use crate::vocabulary::Effect;
    use proptest::prelude::*;

    fn manifest(path: &str, json: &str) -> Manifest {
        Manifest {
            path: path.to_string(),
            json: serde_json::from_str(json).ok(),
        }
    }

    fn tree() -> Tree {
        Tree {
            under_architecture: true,
            capabilities: vec![
                CapabilityCrate {
                    dir: "crates/credit/capabilities/calculate-risk".to_string(),
                    system: "credit".to_string(),
                    manifest: Some(manifest(
                        "crates/credit/capabilities/calculate-risk/capability.json",
                        r#"{"capability":"credit.calculateRisk","system":"credit","implements":"Risk@3"}"#,
                    )),
                    cargo_toml: None,
                },
                CapabilityCrate {
                    dir: "crates/credit/capabilities/late-payments".to_string(),
                    system: "credit".to_string(),
                    manifest: Some(manifest(
                        "crates/credit/capabilities/late-payments/capability.json",
                        r#"{"capability":"credit.latePayments","system":"credit"}"#,
                    )),
                    cargo_toml: None,
                },
            ],
            micro_uis: vec![
                (
                    "credit".to_string(),
                    manifest(
                        "apps/credit/risk-badge/micro-ui.json",
                        r#"{"microUi":"RiskBadge","system":"credit"}"#,
                    ),
                ),
                (
                    "credit".to_string(),
                    manifest(
                        "apps/credit/late-list/micro-ui.json",
                        r#"{"microUi":"LatePaymentsList","system":"credit"}"#,
                    ),
                ),
            ],
            data_capabilities: vec![manifest(
                "crates/dataguard/data-capabilities/request-limit/data-capability.json",
                r#"{"dataCapability":"credit.requestLimitChange","owner":"dataguard"}"#,
            )],
            aggregates: vec![manifest(
                "crates/dataguard/aggregates/Account/aggregate.json",
                r#"{"aggregate":"Account"}"#,
            )],
            concepts: vec![("Risk".to_string(), 3)],
            screens: vec!["credit-review".to_string()],
            mocked_components: vec!["RiskBadge".to_string()],
            ..Tree::default()
        }
    }

    fn graph() -> CargoGraph {
        CargoGraph {
            crates: vec![
                Crate {
                    name: "calculate-risk".to_string(),
                    dir: "crates/credit/capabilities/calculate-risk".to_string(),
                    deps: vec!["credit-read".to_string()],
                },
                Crate {
                    name: "credit-read".to_string(),
                    dir: "crates/credit/read".to_string(),
                    deps: vec!["credit-types".to_string()],
                },
                Crate {
                    name: "credit-types".to_string(),
                    dir: "crates/credit/types".to_string(),
                    deps: Vec::new(),
                },
                Crate {
                    name: "late-payments".to_string(),
                    dir: "crates/credit/capabilities/late-payments".to_string(),
                    deps: vec!["credit-read".to_string()],
                },
                Crate {
                    name: "dataguard-server".to_string(),
                    dir: "crates/dataguard/server".to_string(),
                    deps: vec!["request-limit".to_string()],
                },
                Crate {
                    name: "request-limit".to_string(),
                    dir: "crates/dataguard/data-capabilities/request-limit".to_string(),
                    deps: vec!["dataguard-engine".to_string()],
                },
                Crate {
                    name: "dataguard-engine".to_string(),
                    dir: "crates/dataguard/engine".to_string(),
                    deps: Vec::new(),
                },
            ],
        }
    }

    fn declared(unit: Unit, system: &str, produces: &[&str], needs: &[&str]) -> Declaration {
        Declaration {
            unit,
            system: system.to_string(),
            concept: None,
            effect: None,
            touches: Vec::new(),
            aggregate: None,
            produces: produces.iter().map(|s| (*s).to_string()).collect(),
            needs: needs.iter().map(|s| (*s).to_string()).collect(),
            profile: None,
            host: None,
        }
    }

    #[test]
    fn the_graph_is_read_from_cargo_metadata() {
        let metadata = r#"{
          "workspace_root": "/w",
          "packages": [
            {"name": "a", "manifest_path": "/w/crates/s/capabilities/a/Cargo.toml",
             "dependencies": [{"name": "plumb", "path": "/w/crates/s/plumb"}, {"name": "serde", "path": null}]},
            {"name": "plumb", "manifest_path": "/w/crates/s/plumb/Cargo.toml", "dependencies": []}
          ]
        }"#;
        let graph = CargoGraph::parse(metadata).expect("a graph");
        assert_eq!(graph.crates.len(), 2);
        assert_eq!(graph.crates[0].dir, "crates/s/capabilities/a");
        assert_eq!(
            graph.crates[0].deps,
            ["plumb"],
            "a registry dependency is not a path"
        );
        assert_eq!(graph.reached("a"), ["crates/s/plumb"]);
        assert!(graph.owning("crates/s/plumb/").is_some());
        assert!(CargoGraph::parse("not json").is_none());
        assert!(
            CargoGraph::parse(r#"{"packages": []}"#).is_none(),
            "no root, no graph"
        );
    }

    #[test]
    fn reaching_is_transitive_and_never_lists_the_crate_itself() {
        let reached = graph().reached("calculate-risk");
        assert_eq!(reached, ["crates/credit/read", "crates/credit/types"]);
        assert!(graph().reached("nobody").is_empty(), "expected nothing");
    }

    #[test]
    fn a_capability_gets_its_folder_its_plumbing_and_its_concept_and_no_other_slice() {
        let task = Declaration {
            concept: Some("Risk@3".to_string()),
            ..declared(Unit::Capability, "credit", &["credit.calculateRisk"], &[])
        };
        let found = closure(&tree(), &task, Some(&graph()), None);
        assert!(found.narrowed, "{}", found.reason);
        assert!(
            found
                .patterns
                .contains(&"/crates/credit/capabilities/calculate-risk/".to_string())
        );
        assert!(found.patterns.contains(&"/crates/credit/read/".to_string()));
        assert!(
            found
                .patterns
                .contains(&"/crates/credit/types/".to_string())
        );
        assert!(found.patterns.contains(&"/concepts/Risk/v3/".to_string()));
        assert!(
            found
                .patterns
                .contains(&"/crates/credit/queries/".to_string())
        );
        assert!(found.patterns.contains(&"/mock/src/".to_string()));
        assert!(
            !found.patterns.iter().any(|p| p.contains("late-payments")),
            "{:?}",
            found.patterns
        );
        assert!(
            !found.patterns.iter().any(|p| p == "/crates/credit/"),
            "narrowed by the graph, not by folder: {:?}",
            found.patterns
        );
        assert!(
            found
                .patterns
                .contains(&"!/docs/ARCHITECTURE.md".to_string())
        );
        assert_eq!(found.patterns[0], "/*");
        assert_eq!(found.patterns[1], "!/*/");
    }

    #[test]
    fn a_new_data_capability_gets_what_the_server_names_and_the_glob_placeholders() {
        // #40: the server names every DataCapability by path, and an empty
        // member glob fails the workspace — cargo loaded nothing.
        let task = declared(Unit::DataCapability, "dataguard", &["dataguard.other"], &[]);
        let manifest = "[workspace]\nmembers = [\"crates/*/capabilities/*\", \"crates/data\"]\n";
        let found = closure(&tree(), &task, Some(&graph()), Some(manifest));
        assert!(
            found
                .patterns
                .contains(&"/crates/dataguard/data-capabilities/request-limit/".to_string()),
            "{:?}",
            found.patterns
        );
        assert!(
            found
                .patterns
                .contains(&"/crates/*/capabilities/README.md".to_string())
        );
        assert!(found.patterns.contains(&"/crates/data/".to_string()));
        assert_eq!(glob_members(manifest), ["crates/*/capabilities/*"]);
    }

    #[test]
    fn a_named_member_brings_what_it_depends_on_by_path() {
        // #41, a Capability: the root names the server, which names every
        // DataCapability by path — cargo loads nothing without them.
        let task = declared(Unit::Capability, "credit", &["credit.revenue"], &[]);
        let manifest = "[workspace]\nmembers = [\"crates/dataguard/server\"]\n";
        let found = closure(&tree(), &task, Some(&graph()), Some(manifest));
        for dir in [
            "/crates/dataguard/server/",
            "/crates/dataguard/data-capabilities/request-limit/",
            "/crates/dataguard/engine/",
        ] {
            assert!(
                found.patterns.contains(&dir.to_string()),
                "{dir}: {:?}",
                found.patterns
            );
        }
        let alone = closure(&tree(), &task, Some(&graph()), None);
        assert!(
            !alone.patterns.iter().any(|p| p.contains("request-limit")),
            "nothing names it: {:?}",
            alone.patterns
        );
    }

    #[test]
    fn a_needed_port_and_the_rules_tests_are_in_the_closure() {
        // ForgeDesMondes #42: `needs: llm` stopped on crates/ports/llm/ and
        // tests/rules/business/ missing from the tree.
        let task = declared(
            Unit::Capability,
            "credit",
            &["credit.revenue"],
            &["llm", "credit.calculateRisk", "Risk@3"],
        );
        let found = closure(&tree(), &task, Some(&graph()), None);
        assert!(found.patterns.contains(&"/crates/ports/llm/".to_string()));
        assert!(found.patterns.contains(&"/tests/rules/".to_string()));
        assert!(
            !found
                .patterns
                .iter()
                .any(|p| p.contains("ports/credit") || p.contains("ports/Risk")),
            "{:?}",
            found.patterns
        );
        assert!(port_like("ports-2") && !port_like("") && !port_like("Llm"));
    }

    #[test]
    fn a_capability_not_built_yet_gets_its_system_minus_the_other_slices() {
        let task = declared(Unit::Capability, "credit", &["credit.revenue"], &[]);
        let found = closure(&tree(), &task, Some(&graph()), None);
        assert!(found.narrowed);
        assert!(found.patterns.contains(&"/crates/credit/".to_string()));
        assert!(
            found
                .patterns
                .contains(&"!/crates/credit/capabilities/*/".to_string())
        );
        assert!(
            found
                .patterns
                .contains(&"!/crates/credit/data-capabilities/*/".to_string())
        );
    }

    #[test]
    fn without_a_graph_a_built_capability_still_gets_its_own_folder_back() {
        let task = declared(Unit::Capability, "credit", &["credit.calculateRisk"], &[]);
        let found = closure(&tree(), &task, None, None);
        let exclude = found
            .patterns
            .iter()
            .position(|p| p == "!/crates/credit/capabilities/*/")
            .expect("the other slices taken back");
        let own = found
            .patterns
            .iter()
            .position(|p| p == "/crates/credit/capabilities/calculate-risk/")
            .expect("its own folder");
        assert!(
            own > exclude,
            "given back after the exclusion, or git drops it"
        );
    }

    #[test]
    fn a_data_capability_gets_its_aggregate_and_the_server() {
        let task = Declaration {
            effect: Some(Effect::Update),
            aggregate: Some("Account".to_string()),
            ..declared(
                Unit::DataCapability,
                "dataguard",
                &["credit.requestLimitChange"],
                &[],
            )
        };
        let found = closure(&tree(), &task, Some(&graph()), None);
        assert!(
            found
                .patterns
                .contains(&"/crates/dataguard/data-capabilities/request-limit/".to_string())
        );
        assert!(
            found
                .patterns
                .contains(&"/crates/dataguard/engine/".to_string())
        );
        assert!(
            found
                .patterns
                .contains(&"/crates/dataguard/server/".to_string())
        );
        assert!(
            found
                .patterns
                .contains(&"/crates/dataguard/aggregates/Account/".to_string())
        );
        assert!(
            !found.patterns.contains(&ENGINE_MANUAL.to_string()),
            "the engine's manual is the write side's: {:?}",
            found.patterns
        );
        let hostless = Declaration {
            host: Some("hostless".to_string()),
            ..task
        };
        let found = closure(&tree(), &hostless, Some(&graph()), None);
        assert!(
            !found
                .patterns
                .contains(&"/crates/dataguard/server/".to_string()),
            "no server runs it: {:?}",
            found.patterns
        );
    }

    #[test]
    fn a_micro_ui_gets_the_mock_up_the_manifests_it_needs_and_no_other_micro_ui() {
        let task = declared(
            Unit::MicroUi,
            "credit",
            &["RiskBadge"],
            &["credit.calculateRisk", "credit.requestLimitChange"],
        );
        let found = closure(&tree(), &task, None, None);
        assert!(found.narrowed);
        assert!(found.patterns.contains(&"/apps/".to_string()));
        assert!(
            found
                .patterns
                .contains(&"!/apps/credit/late-list/".to_string())
        );
        assert!(
            !found
                .patterns
                .contains(&"!/apps/credit/risk-badge/".to_string())
        );
        assert!(found.patterns.contains(&"/mock/".to_string()));
        assert!(
            found.patterns.contains(
                &"/crates/credit/capabilities/calculate-risk/capability.json".to_string()
            )
        );
        assert!(found.patterns.contains(
            &"/crates/dataguard/data-capabilities/request-limit/data-capability.json".to_string()
        ));
        assert!(
            !found
                .patterns
                .iter()
                .any(|p| p.ends_with("calculate-risk/")),
            "the manifest, not the code: {:?}",
            found.patterns
        );
        assert!(found.missing.is_empty(), "{:?}", found.missing);
    }

    #[test]
    fn an_aggregate_gets_its_whole_system() {
        let task = declared(Unit::Aggregate, "dataguard", &["Account"], &[]);
        let found = closure(&tree(), &task, Some(&graph()), None);
        assert!(found.patterns.contains(&"/crates/dataguard/".to_string()));
        assert!(
            found.patterns.contains(&ENGINE_MANUAL.to_string()),
            "the write side reads the engine's manual: {:?}",
            found.patterns
        );
    }

    #[test]
    fn infrastructure_without_a_system_gets_every_crate() {
        let task = declared(Unit::Infrastructure, "", &[], &[]);
        let found = closure(&tree(), &task, None, None);
        assert!(found.narrowed);
        assert!(found.patterns.contains(&"/crates/".to_string()));
    }

    #[test]
    fn a_concept_gets_the_concepts_folder() {
        let task = declared(Unit::Concept, "", &["Risk@4"], &[]);
        let found = closure(&tree(), &task, None, None);
        assert!(found.narrowed);
        assert!(found.patterns.contains(&"/concepts/".to_string()));
    }

    #[test]
    fn tooling_is_never_narrowed_and_says_why() {
        let task = declared(Unit::Tooling, "platform", &[], &[]);
        let found = closure(&tree(), &task, None, None);
        assert!(!found.narrowed);
        assert!(found.patterns.is_empty(), "expected nothing");
        assert!(found.reason.contains("whole workspace"));
    }

    #[test]
    fn a_slice_without_a_system_is_not_narrowed() {
        let task = declared(Unit::Capability, "", &["x"], &[]);
        let found = closure(&tree(), &task, None, None);
        assert!(!found.narrowed);
        assert!(found.reason.contains("no system"));
    }

    #[test]
    fn a_need_the_checkout_does_not_hold_is_missing() {
        let task = declared(
            Unit::MicroUi,
            "credit",
            &["RiskBadge"],
            &[
                "credit.calculateRisk",
                "orders.list",
                "Risk@3",
                "credit-review",
            ],
        );
        let found = closure(&tree(), &task, None, None);
        assert_eq!(found.missing, ["orders.list"]);
    }

    #[test]
    fn ids_are_known_whatever_their_case() {
        let task = declared(
            Unit::MicroUi,
            "credit",
            &["riskbadge"],
            &["CREDIT.CALCULATERISK"],
        );
        let found = closure(&tree(), &task, None, None);
        assert!(found.missing.is_empty(), "expected nothing");
        assert!(
            !found
                .patterns
                .contains(&"!/apps/credit/risk-badge/".to_string())
        );
    }

    #[test]
    fn explicit_members_are_read_and_globs_are_not() {
        let toml = "[workspace]\nmembers = [\n  \"crates/*/capabilities/*\",\n  \"crates/dataguard/server\",\n  'crates/platform/tooling/',\n]\nresolver = \"2\"\n";
        assert_eq!(
            explicit_members(toml),
            ["crates/dataguard/server", "crates/platform/tooling"]
        );
        assert!(
            explicit_members("[package]\nname = \"x\"\n").is_empty(),
            "expected nothing"
        );
        assert!(
            explicit_members("members = [").is_empty(),
            "unclosed, nothing"
        );
    }

    #[test]
    fn a_comment_naming_members_is_not_read_as_the_key() {
        // ForgeDesMondes' root manifest, verbatim head: its comment quotes
        // the gate `workspace-members-by-glob` above `[workspace]`.
        let toml = "# the two globs: adding one edits no shared file (rule 2, gate\n# `workspace-members-by-glob`). Only the plumbing is named.\n[workspace]\nresolver = \"3\"\nmembers = [\n    \"crates/*/capabilities/*\",\n    \"crates/data\", # the Data layer [plumbing]\n    \"crates/dataguard/server\",\n]\n";
        assert_eq!(
            explicit_members(toml),
            ["crates/data", "crates/dataguard/server"]
        );
        assert!(
            explicit_members("[workspace]\nmembersx = [\"a\"]\n").is_empty(),
            "another key, nothing"
        );
    }

    #[test]
    fn a_single_explicit_member_is_read_whole() {
        assert_eq!(
            explicit_members("members = [\"crates/dataguard/server\"]"),
            ["crates/dataguard/server"]
        );
    }

    #[test]
    fn a_concept_is_known_by_name_and_version_together() {
        let index = Index { tree: &tree() };
        assert_eq!(
            index.concept_dir("Risk@3").as_deref(),
            Some("concepts/Risk/v3")
        );
        assert!(
            index.concept_dir("Risk@2").is_none(),
            "the version is not documented"
        );
        assert!(index.concept_dir("Other@3").is_none(), "the name is not");
        assert!(
            index.concept_dir("Risk").is_none(),
            "no version, no concept"
        );
    }

    #[test]
    fn an_id_is_known_by_any_one_kind_of_unit() {
        let index = Index { tree: &tree() };
        for id in [
            "credit.calculateRisk",
            "credit.requestLimitChange",
            "Account",
            "LatePaymentsList",
            "Risk@3",
            "credit-review",
            "RiskBadge",
        ] {
            assert!(index.knows(id), "{id}");
        }
        assert!(!index.knows("nobody.knows"));
    }

    #[test]
    fn an_explicit_member_enters_every_closure() {
        let task = declared(Unit::Capability, "credit", &["credit.calculateRisk"], &[]);
        let toml = "members = [\"crates/*/capabilities/*\", \"crates/dataguard/server\"]";
        let found = closure(&tree(), &task, Some(&graph()), Some(toml));
        assert!(
            found
                .patterns
                .contains(&"/crates/dataguard/server/".to_string())
        );
    }

    #[test]
    fn a_path_is_allowed_by_the_last_pattern_that_matches_it() {
        let patterns: Vec<String> = [
            "/*",
            "!/*/",
            "/crates/credit/",
            "!/crates/credit/capabilities/*/",
            "/crates/credit/capabilities/risk/",
            "/apps/credit/x/micro-ui.json",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert!(allows(&patterns, "Cargo.toml"), "a root file");
        assert!(!allows(&patterns, "docs/x.md"), "a folder not given back");
        assert!(allows(&patterns, "crates/credit/read/src/lib.rs"));
        assert!(
            !allows(&patterns, "crates/credit/capabilities/score/src/lib.rs"),
            "another slice"
        );
        assert!(
            allows(&patterns, "crates/credit/capabilities/risk/src/lib.rs"),
            "its own, given back"
        );
        assert!(allows(&patterns, "apps/credit/x/micro-ui.json"), "one file");
        assert!(
            !allows(&patterns, "apps/credit/x/index.ts"),
            "not its neighbour"
        );
        assert!(!allows(&[], "anything"), "no pattern, nothing");
    }

    #[test]
    fn the_description_lists_what_is_there_not_what_is_taken_back() {
        let task = declared(Unit::MicroUi, "credit", &["RiskBadge"], &[]);
        let found = closure(&tree(), &task, None, None);
        let text = describe(&found);
        assert!(text.contains("- /apps/"));
        assert!(text.contains("- /mock/"));
        assert!(!text.contains("late-list"));
        assert!(!text.contains("- /*"));
    }

    proptest! {
        #[test]
        fn every_pattern_is_anchored_and_leaves_the_root_alone(
            system in "[a-z]{1,8}",
            produces in proptest::collection::vec("[A-Za-z.]{1,12}", 0..3),
            needs in proptest::collection::vec("[A-Za-z.@0-9]{1,12}", 0..4),
            unit in 0usize..9,
        ) {
            let task = declared(Unit::ALL[unit], &system, &produces.iter().map(String::as_str).collect::<Vec<_>>(), &needs.iter().map(String::as_str).collect::<Vec<_>>());
            let found = closure(&tree(), &task, Some(&graph()), None);
            for pattern in &found.patterns {
                let body = pattern.trim_start_matches('!');
                prop_assert!(body.starts_with('/'), "{pattern}");
                prop_assert!(!body.contains(".."), "{pattern}");
            }
            prop_assert!(found.missing.iter().all(|m| needs.contains(m)));
            prop_assert!(found.narrowed != found.patterns.is_empty() || !found.narrowed);
            if found.narrowed {
                prop_assert_eq!(&found.patterns[..2], &["/*".to_string(), "!/*/".to_string()]);
            }
        }
    }
}
